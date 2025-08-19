#!/bin/bash
#
# Functions for managing TDX development VMs and local development containers.
# This script is intended to be sourced, not executed directly.

################################################################################
# Creates a TDX-enabled development VM with all dependencies pre-installed.
################################################################################
dev_create_vm() {
  info "Creating TDX development VM..."
  check_deps gcloud
  resolve_gcp_identity

  # Check if VM already exists
  if gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
    warn "VM ${TDX_VM_NAME} already exists in zone ${TDX_VM_ZONE}"
    return 0
  fi

  # Generate SSH key if it doesn't exist
  local ssh_key_path="$HOME/.ssh/nxcc-tdx-dev"
  if [[ ! -f "$ssh_key_path" ]]; then
    info "Generating SSH key for TDX VM..."
    ssh-keygen -t rsa -b 4096 -f "$ssh_key_path" -N "" -C "nxcc-tdx-dev"
  fi

  # Create the startup script
  local startup_script_path="/tmp/nxcc-tdx-setup.sh"
  cat > "$startup_script_path" << 'EOF'
#!/bin/bash
set -e

echo "=== NXCC TDX Development VM Setup ==="
echo "Started at: $(date)"

# Update system
apt-get update

# Install minimal dependencies: Docker, git, and basic tools
apt-get install -y --no-install-recommends \
  curl wget git vim \
  ca-certificates gnupg lsb-release

# Install Docker
curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin

# Add ubuntu user to docker group
usermod -aG docker ubuntu

# Enable and start Docker service
systemctl enable docker
systemctl start docker

# Verify TDX is available (check for TDX-specific files/capabilities)
echo "=== Checking TDX Support ==="
if [ -e /sys/firmware/tdx_guest ]; then
  echo "TDX guest support detected!"
else
  echo "Warning: TDX guest support not detected in /sys/firmware/tdx_guest"
fi

echo "=== TDX VM Setup Complete ==="
echo "Finished at: $(date)"
echo "VM is ready for TDX testing with Docker!"
EOF

  info "Creating TDX-enabled VM: ${TDX_VM_NAME}"
  info "Zone: ${TDX_VM_ZONE}"
  info "Machine type: ${TDX_VM_MACHINE_TYPE} (confidential computing enabled)"
  
  gcloud compute instances create "${TDX_VM_NAME}" \
    --project="${RESOLVED_PROJECT_ID}" \
    --account="${RESOLVED_GCP_ACCOUNT}" \
    --zone="${TDX_VM_ZONE}" \
    --machine-type="${TDX_VM_MACHINE_TYPE}" \
    --image-family="${TDX_VM_IMAGE_FAMILY}" \
    --image-project="${TDX_VM_IMAGE_PROJECT}" \
    --boot-disk-size=50GB \
    --boot-disk-type=pd-standard \
    --confidential-compute \
    --maintenance-policy=TERMINATE \
    --metadata-from-file startup-script="$startup_script_path" \
    --metadata ssh-keys="ubuntu:$(cat "${ssh_key_path}".pub)" \
    --scopes="https://www.googleapis.com/auth/cloud-platform" \
    --tags="nxcc-dev"

  # Clean up startup script
  rm -f "$startup_script_path"

  success "TDX VM ${TDX_VM_NAME} created successfully!"
  info "Waiting for VM to become ready..."
  
  # Wait for VM to be running
  while true; do
    local vm_status
    vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
    if [[ "$vm_status" == "RUNNING" ]]; then
      break
    fi
    info "VM status: $vm_status, waiting..."
    sleep 5
  done

  success "VM is running. Setup script is installing dependencies..."
  info "This may take 5-10 minutes. You can connect with:"
  info "  ./infra/infra.sh dev connect"
  info ""
  info "To check setup progress:"
  info "  gcloud compute ssh ${TDX_VM_NAME} --zone=${TDX_VM_ZONE} --command='sudo journalctl -u google-startup-scripts.service -f'"
}

################################################################################
# Connects to the TDX development VM via SSH.
################################################################################
dev_connect_vm() {
  info "Connecting to TDX development VM..."
  check_deps gcloud ssh
  resolve_gcp_identity

  # Check if VM exists and is running
  if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
    error "VM ${TDX_VM_NAME} does not exist. Create it first with: ./infra/infra.sh dev create"
  fi

  local vm_status
  vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
  
  if [[ "$vm_status" != "RUNNING" ]]; then
    warn "VM is not running (status: $vm_status). Starting it..."
    gcloud compute instances start "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
    
    info "Waiting for VM to start..."
    while true; do
      local vm_status
      vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
      if [[ "$vm_status" == "RUNNING" ]]; then
        break
      fi
      sleep 3
    done
  fi

  info "Connecting to ${TDX_VM_NAME}..."
  info "Once connected, navigate to: cd /home/ubuntu/nxcc/node"
  info ""
  
  # Use gcloud SSH for automatic key management
  gcloud compute ssh ubuntu@"${TDX_VM_NAME}" \
    --zone="${TDX_VM_ZONE}" \
    --project="${RESOLVED_PROJECT_ID}" \
    --account="${RESOLVED_GCP_ACCOUNT}"
}

################################################################################
# Destroys the TDX development VM.
################################################################################
dev_destroy_vm() {
  info "Destroying TDX development VM..."
  check_deps gcloud
  resolve_gcp_identity

  if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
    warn "VM ${TDX_VM_NAME} does not exist. Nothing to destroy."
    return 0
  fi

  info "Deleting VM: ${TDX_VM_NAME}"
  gcloud compute instances delete "${TDX_VM_NAME}" \
    --zone="${TDX_VM_ZONE}" \
    --project="${RESOLVED_PROJECT_ID}" \
    --account="${RESOLVED_GCP_ACCOUNT}" \
    --quiet

  success "TDX VM ${TDX_VM_NAME} destroyed successfully!"
  info "SSH keys are preserved in ~/.ssh/nxcc-tdx-dev for future use"
}

################################################################################
# Shows the status of the TDX development VM.
################################################################################
dev_status_vm() {
  info "Checking TDX development VM status..."
  check_deps gcloud
  resolve_gcp_identity

  if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
    warn "VM ${TDX_VM_NAME} does not exist."
    info "Create it with: ./infra/infra.sh dev create"
    return 0
  fi

  info "VM Details:"
  gcloud compute instances describe "${TDX_VM_NAME}" \
    --zone="${TDX_VM_ZONE}" \
    --project="${RESOLVED_PROJECT_ID}" \
    --account="${RESOLVED_GCP_ACCOUNT}" \
    --format="table(name,status,machineType.basename(),zone.basename(),confidentialInstanceConfig.enableConfidentialCompute)"

  local external_ip
  external_ip=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(networkInterfaces[0].accessConfigs[0].natIP)")
  
  if [[ -n "$external_ip" ]]; then
    info "External IP: $external_ip"
    info "Connect with: ./infra/infra.sh dev connect"
  fi
}

################################################################################
# Runs a local development container with all tools pre-installed.
# 
# Arguments:
#   --platform <platform>  Specify the platform (e.g., linux/amd64, linux/arm64)
#   --build                Force rebuild of the container
################################################################################
dev_run_container() {
  info "Starting NXCC development container..."
  check_deps docker
  
  local project_root platform_arg build_platform run_platform force_build=false
  project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  
  # Parse arguments
  while [[ $# -gt 0 ]]; do
    case $1 in
      --platform)
        platform_arg="$2"
        shift 2
        ;;
      --build)
        force_build=true
        shift
        ;;
      *)
        shift
        ;;
    esac
  done
  
  # Set platform arguments
  if [[ -n "$platform_arg" ]]; then
    build_platform="--platform $platform_arg"
    run_platform="--platform $platform_arg"
    info "Using specified platform: $platform_arg"
  else
    # Auto-detect current platform for running
    local current_arch
    current_arch="$(docker info --format '{{.Architecture}}')"
    run_platform="--platform linux/$current_arch"
    info "Auto-detected platform: linux/$current_arch"
  fi
  
  # Build the development container if it doesn't exist or force rebuild
  if [[ "$force_build" == true ]] || ! docker image inspect nxcc-dev &>/dev/null; then
    info "Building development container (this may take a few minutes)..."
    # shellcheck disable=SC2086  # We want word splitting for platform_arg
    docker build $build_platform -f "${project_root}/dev/Dockerfile" -t nxcc-dev "${project_root}"
  fi
  
  info "Running development container with project mounted at /workspace"
  info "Available tools: rust, node, pnpm, forge, kubectl, kind, grpcurl"
  info ""
  info "Try these commands inside the container:"
  info "  cd /workspace/node && cargo build          # Build Rust components"
  info "  cd /workspace/contracts/evm && forge build  # Build smart contracts"
  info "  cd /workspace/sdk && pnpm build             # Build CLI and SDK"
  info "  cd /workspace && ./e2e/e2e_test.sh          # Run e2e tests"
  info ""
  
  # Run the container interactively with the project mounted
  # shellcheck disable=SC2086  # We want word splitting for run_platform
  docker run -it --rm \
    $run_platform \
    -v "${project_root}:/workspace" \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -w /workspace \
    nxcc-dev /bin/bash
}
