#!/bin/bash
#
# Functions for managing TDX development VMs.
# This script is intended to be sourced, not executed directly.

################################################################################
# Creates a TDX-enabled development VM with all dependencies pre-installed.
################################################################################
dev_create_vm() {
	local use_dedicated=false

	# Parse arguments
	while [[ $# -gt 0 ]]; do
		case $1 in
		--dedicated)
			use_dedicated=true
			shift
			;;
		*)
			error "Unknown option for dev create: $1"
			return 1
			;;
		esac
	done

	info "Creating TDX development VM..."
	check_deps gcloud
	resolve_gcp_identity

	# Determine effective preemptible setting
	local effective_preemptible="${TDX_VM_PREEMPTIBLE}"
	if [[ "$use_dedicated" == "true" ]]; then
		effective_preemptible="false"
	fi

	# Validate required configuration variables
	local required_vars=(
		"TDX_VM_NAME"
		"TDX_VM_ZONE"
		"TDX_VM_MACHINE_TYPE"
		"TDX_VM_IMAGE_FAMILY"
		"TDX_VM_IMAGE_PROJECT"
		"TDX_VM_PREEMPTIBLE"
		"RESOLVED_PROJECT_ID"
		"RESOLVED_GCP_ACCOUNT"
	)

	for var in "${required_vars[@]}"; do
		if [[ -z "${!var}" ]]; then
			error "Required configuration variable $var is not set or empty"
			return 1
		fi
	done

	# Check gcloud version for TDX support (requires 535.0.0+)
	local gcloud_version
	gcloud_version=$(gcloud version | head -1 | awk '{print $4}' | cut -d. -f1)
	if [[ "${gcloud_version:-0}" -lt 535 ]]; then
		warn "gcloud version may not support TDX. Consider updating: gcloud components update"
	fi

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

	# --- Copy cloud-init config ---
	local cloud_init_path="/tmp/nxcc-tdx-cloud-init.yaml"
	local dev_lib_dir
	dev_lib_dir="$(dirname "${BASH_SOURCE[0]}")"
	local cloud_init_file="${dev_lib_dir}/cloud_init.yaml"

	if [ ! -f "$cloud_init_file" ]; then
		error "Cloud-init config not found at: $cloud_init_file"
	fi

	# Copy cloud-init config (declarative only, no templating needed)
	cp "$cloud_init_file" "$cloud_init_path"

	# Determine preemptible setting
	local preemptible_flag=""
	local instance_type="dedicated"
	if [[ "${effective_preemptible}" == "true" ]]; then
		preemptible_flag="--preemptible"
		instance_type="preemptible"
	fi

	info "Creating TDX-enabled VM: ${TDX_VM_NAME}"
	info "Zone: ${TDX_VM_ZONE}"
	info "Machine type: ${TDX_VM_MACHINE_TYPE} (confidential computing enabled)"
	info "Instance type: ${instance_type}"
	info "Project: ${RESOLVED_PROJECT_ID}"
	info "Account: ${RESOLVED_GCP_ACCOUNT}"

	# Show the gcloud command that will be executed
	info "Executing gcloud compute instances create command..."

	if ! gcloud compute instances create "${TDX_VM_NAME}" \
		--project="${RESOLVED_PROJECT_ID}" \
		--account="${RESOLVED_GCP_ACCOUNT}" \
		--zone="${TDX_VM_ZONE}" \
		--machine-type="${TDX_VM_MACHINE_TYPE}" \
		--image-family="${TDX_VM_IMAGE_FAMILY}" \
		--image-project="${TDX_VM_IMAGE_PROJECT}" \
		--boot-disk-size=20GB \
		--boot-disk-type=pd-ssd \
		--confidential-compute-type=TDX \
		--maintenance-policy=TERMINATE \
		--metadata-from-file user-data="$cloud_init_path" \
		--metadata ssh-keys="ubuntu:$(cat "${ssh_key_path}".pub)" \
		--scopes="https://www.googleapis.com/auth/cloud-platform" \
		--tags="nxcc-dev" \
		$preemptible_flag \
		--verbosity=info; then
		error "Failed to create VM ${TDX_VM_NAME}. Check the gcloud output above for details."
		rm -f "$cloud_init_path"
		return 1
	fi

	rm -f "$cloud_init_path"
	success "TDX VM ${TDX_VM_NAME} created successfully!"
	info "Waiting for VM to become ready..."

	while true; do
		local vm_status
		vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
		if [[ "$vm_status" == "RUNNING" ]]; then break; fi
		info "VM status: $vm_status, waiting..."
		sleep 5
	done

	success "VM is running. Waiting for SSH connectivity and cloud-init to complete..."
	local max_wait=600 elapsed=0
	local ssh_connected=false

	# First wait for SSH to be available
	info "Testing SSH connectivity..."
	while [[ $elapsed -lt $max_wait ]]; do
		if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="echo 'SSH connected'" --quiet 2>/dev/null; then
			success "SSH connectivity established!"
			ssh_connected=true
			break
		fi
		sleep 10
		elapsed=$((elapsed + 10))
		info "Waiting for SSH connectivity... (${elapsed}s/${max_wait}s)"
	done

	if [[ "$ssh_connected" != "true" ]]; then
		error "SSH connectivity failed after ${max_wait}s. Cannot proceed with setup."
		return 1
	fi

	# Now wait for cloud-init
	info "Waiting for cloud-init to complete system setup..."
	elapsed=0
	while [[ $elapsed -lt $max_wait ]]; do
		local cloud_init_status
		cloud_init_status=$(gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="sudo cloud-init status" 2>/dev/null | grep "status:" | awk '{print $2}')

		case "$cloud_init_status" in
		"done")
			success "Cloud-init setup completed successfully!"
			break
			;;
		"error")
			error "Cloud-init failed! Checking detailed status..."
			gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="sudo cloud-init status --long"
			return 1
			;;
		"running")
			info "Cloud-init still running... (${elapsed}s/${max_wait}s)"
			;;
		*)
			info "Cloud-init status: ${cloud_init_status} (${elapsed}s/${max_wait}s)"
			;;
		esac

		sleep 10
		elapsed=$((elapsed + 10))
	done

	if [[ $elapsed -ge $max_wait ]]; then
		error "Cloud-init setup timed out after ${max_wait}s."
		return 1
	fi

	# Now run the actual setup script with logging
	info "Running TDX development environment setup script..."
	local setup_script_path="${dev_lib_dir}/setup_tdx_vm.sh"

	if [ ! -f "$setup_script_path" ]; then
		error "Setup script not found at: $setup_script_path"
		return 1
	fi

	# Upload all setup scripts and supporting files
	local temp_script
	temp_script="/tmp/setup_tdx_vm_$(date +%s).sh"
	local container_script_path="${dev_lib_dir}/setup_container.sh"
	local temp_container_script
	temp_container_script="/tmp/setup_container_$(date +%s).sh"

	info "Uploading setup scripts and supporting files..."

	# Upload main setup script
	if ! gcloud compute scp --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" "$setup_script_path" ubuntu@"${TDX_VM_NAME}:$temp_script"; then
		error "Failed to upload main setup script"
		return 1
	fi

	# Upload supporting files needed by setup script
	local supporting_files=(
		"$dev_lib_dir/tdx_verification.py"
		"$dev_lib_dir/dev-container.sh"
		"$dev_lib_dir/setup-nxcc.sh"
	)

	for file in "${supporting_files[@]}"; do
		if [ -f "$file" ]; then
			local filename
			filename=$(basename "$file")
			if ! gcloud compute scp --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" "$file" ubuntu@"${TDX_VM_NAME}:/tmp/$filename"; then
				error "Failed to upload supporting file: $filename"
				return 1
			fi
		else
			error "Supporting file not found: $file"
			return 1
		fi
	done

	# Upload container setup script if it exists and is not already present on VM
	if [ -f "$container_script_path" ]; then
		# Check if container setup script already exists on VM
		if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="test -f $temp_container_script" 2>/dev/null; then
			info "Container setup script already exists on VM, skipping upload"
		else
			if ! gcloud compute scp --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" "$container_script_path" ubuntu@"${TDX_VM_NAME}:$temp_container_script"; then
				error "Failed to upload container setup script"
				return 1
			fi
			info "Container setup script uploaded successfully"
		fi
		info "All setup scripts and supporting files processed successfully"
	else
		info "Setup scripts and supporting files uploaded (container script will use fallback method)"
	fi

	# Execute the setup script with proper environment and capture all output
	info "Executing setup script with full logging..."
	local script_env="SCRIPT_DIR=/tmp CONTAINER_SETUP_SCRIPT=$temp_container_script"
	if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="$script_env sudo -E bash $temp_script 2>&1 | sudo tee -a /var/log/nxcc-setup.log >/dev/null"; then
		# Clean up temporary files
		gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="rm -f $temp_script $temp_container_script" 2>/dev/null || true
		success "TDX development environment setup completed successfully!"

		# Verify the setup was successful
		if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="test -f /home/ubuntu/.nxcc-setup-complete"; then
			success "Setup completion verified!"

			# Show next steps
			info ""
			info "🎉 TDX development environment is ready!"
			info ""
			info "Next steps:"
			info "  • Connect to VM: ./infra.sh dev ssh"
			info "  • Verify TDX: ssh to VM and run 'python3 tdx_verification.py'"
			info "  • Sync code: ./infra.sh dev push"
			info "  • Connect to container: ./infra.sh dev container"
			info ""
			success "Environment setup completed successfully - container is running!"
			return 0
		else
			error "Setup script execution completed but setup verification failed!"
			info "Check setup logs: ./infra.sh dev ssh -- 'sudo cat /var/log/nxcc-setup.log'"
			return 1
		fi
	else
		error "Setup script execution failed!"
		info "Check logs: ./infra.sh dev ssh -- 'sudo cat /var/log/nxcc-setup.log'"
		return 1
	fi
}

################################################################################
# Verifies that the setup script completed successfully.
################################################################################
dev_complete_setup() {
	info "Checking TDX development environment setup status..."

	# Check if setup completion file exists and setup was successful
	if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="test -f /home/ubuntu/.nxcc-setup-complete"; then
		success "TDX development environment setup completed successfully!"
		return 0
	else
		error "TDX development environment setup failed or did not complete!"
		info "Check cloud-init logs: sudo journalctl -u cloud-init"
		info "Check setup logs: sudo cat /var/log/nxcc-setup.log"
		return 1
	fi
}

################################################################################
# Verifies TDX functionality on the development VM using the deployed script.
################################################################################
dev_verify_tdx() {
	info "Verifying TDX attestation capabilities..."

	if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="sudo python3 /home/ubuntu/tdx_verification.py"; then
		return 0
	else
		return 1
	fi
}

################################################################################
# Starts the NXCC development container on the VM.
################################################################################
dev_start_container() {
	info "Setting up NXCC development container..."
	check_deps gcloud
	resolve_gcp_identity

	local dev_lib_dir
	dev_lib_dir="$(dirname "${BASH_SOURCE[0]}")"
	local setup_script_path="${dev_lib_dir}/setup_container.sh"

	if [ ! -f "$setup_script_path" ]; then
		error "Container setup script not found at: $setup_script_path"
		return 1
	fi

	# Upload and execute the setup script
	local temp_script
	temp_script="/tmp/setup_container_$(date +%s).sh"

	# Check if container setup script already exists on VM
	if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="test -f $temp_script" 2>/dev/null; then
		info "Container setup script already exists on VM, skipping upload"
	else
		info "Uploading container setup script..."
		if ! gcloud compute scp --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" "$setup_script_path" ubuntu@"${TDX_VM_NAME}:$temp_script"; then
			error "Failed to upload container setup script to VM"
			return 1
		fi
	fi

	info "Executing container setup script..."
	# Set environment variable and execute the setup script
	local env_vars="NXCC_DEV_IMAGE=${NXCC_DEV_IMAGE:-ghcr.io/nxcc-bridge/dev:latest}"
	if gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="$env_vars bash $temp_script && rm -f $temp_script"; then
		success "Development container started successfully!"
		return 0
	else
		error "Container setup script execution failed!"
		return 1
	fi
}

################################################################################
# Manages the development container (start/restart/status).
################################################################################
dev_manage_container() {
	local action="${1:-start}"
	local detached_flag=""

	if [[ "$2" == "--detached" || "$2" == "-d" ]]; then
		detached_flag="--detached"
	fi

	info "Managing development container (action: $action)..."
	check_deps gcloud
	resolve_gcp_identity

	if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		error "VM ${TDX_VM_NAME} does not exist. Create it first with: ./infra.sh dev create"
	fi

	case "$action" in
	"start" | "restart")
		if dev_start_container; then
			success "Development container is running"
			if [[ -z "$detached_flag" ]]; then
				info "Container started successfully. Use 'docker exec -it nxcc-dev-container bash' to connect."
			fi
		else
			error "Failed to start development container"
			return 1
		fi
		;;
	"status")
		gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="docker ps -a | grep nxcc-dev-container || echo 'Container not found'"
		;;
	*)
		error "Unknown container action: $action"
		return 1
		;;
	esac
}

################################################################################
# Connects to the TDX development VM via SSH.
# Arguments:
#   -- <command>  Execute a specific command on the VM (optional)
################################################################################
dev_connect_vm() {
	info "Connecting to TDX development VM..."
	check_deps gcloud ssh
	resolve_gcp_identity

	if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		error "VM ${TDX_VM_NAME} does not exist. Create it first with: ./infra.sh dev create"
		return 1
	fi

	local vm_status
	vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
	if [[ "$vm_status" != "RUNNING" ]]; then
		warn "VM is not running (status: $vm_status). Starting it..."
		gcloud compute instances start "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
		info "Waiting for VM to start..."
		while true; do
			vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
			if [[ "$vm_status" == "RUNNING" ]]; then break; fi
			sleep 3
		done
	fi

	info "Connecting to ${TDX_VM_NAME}..."
	if [[ "$#" -gt 0 && "$1" == "--" ]]; then
		shift
		gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --ssh-flag="-tt" --ssh-flag="-o StrictHostKeyChecking=no" --command="$*"
	else
		gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}"
	fi
}

################################################################################
# Destroys the TDX development VM created by infra.sh only.
################################################################################
dev_cleanup_managed_vm() {
	info "Cleaning up TDX development VM managed by infra.sh..."
	check_deps gcloud
	resolve_gcp_identity

	if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		success "No infra.sh managed TDX VM found to clean up."
		return 0
	fi

	info "Found infra.sh managed VM: ${TDX_VM_NAME} in zone ${TDX_VM_ZONE}"
	read -p "Delete this VM? [y/N] " -n 1 -r
	echo
	if [[ ! $REPLY =~ ^[Yy]$ ]]; then
		info "Cleanup cancelled."
		return 0
	fi

	info "Deleting VM: ${TDX_VM_NAME}"
	gcloud compute instances delete "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet
	success "Infra.sh managed VM cleanup completed!"
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
	if gcloud compute instances delete "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --quiet 2>/dev/null; then
		success "TDX VM ${TDX_VM_NAME} destroyed successfully!"
	else
		# VM might have been deleted by another process or doesn't exist
		warn "VM ${TDX_VM_NAME} was already deleted or doesn't exist."
	fi
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
		info "Create it with: ./infra.sh dev create"
		return 0
	fi

	info "VM Details:"
	gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="table(name,status,machineType.basename(),zone.basename(),confidentialInstanceConfig.enableConfidentialCompute)"
	local external_ip
	external_ip=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(networkInterfaces[0].accessConfigs[0].natIP)")
	if [[ -n "$external_ip" ]]; then
		info "External IP: $external_ip"
		info "Connect with: ./infra.sh dev ssh"
	fi
}

################################################################################
# Syncs local code to the TDX development VM using rsync with gitignore support.
# Arguments:
#   <source_dir>  The local directory to sync (defaults to current directory)
################################################################################
dev_push_code() {
	local source_dir="${1:-.}"
	info "Syncing code to TDX development VM..."
	check_deps gcloud rsync
	resolve_gcp_identity

	if ! gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" &>/dev/null; then
		error "VM ${TDX_VM_NAME} does not exist. Create it first with: ./infra.sh dev create"
		return 1
	fi

	local vm_status
	vm_status=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(status)")
	if [[ "$vm_status" != "RUNNING" ]]; then
		error "VM is not running (status: $vm_status). Start it first with: ./infra.sh dev ssh"
		return 1
	fi

	# Ensure the remote directory exists
	if ! gcloud compute ssh ubuntu@"${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --command="mkdir -p /home/ubuntu/nxcc"; then
		error "Failed to create remote directory"
		return 1
	fi

	info "Syncing files with rsync (respecting .gitignore)..."

	# Create rsync exclude patterns from .gitignore
	local rsync_excludes=""
	if [[ -f "$source_dir/.gitignore" ]]; then
		# Convert .gitignore patterns to rsync exclude patterns
		while IFS= read -r line; do
			# Skip empty lines and comments
			if [[ -n "$line" && ! "$line" =~ ^[[:space:]]*# ]]; then
				# Remove trailing slashes for rsync
				line="${line%/}"
				rsync_excludes="$rsync_excludes --exclude=$line"
			fi
		done <"$source_dir/.gitignore"
	fi

	# Additional excludes for common cache directories and files that should never sync
	rsync_excludes="$rsync_excludes --exclude=.git --exclude=.DS_Store --exclude=*.swp --exclude=*.tmp"

	# Get the external IP and prepare SSH configuration matching gcloud
	local external_ip
	external_ip=$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(networkInterfaces[0].accessConfigs[0].natIP)")

	if [[ -z "$external_ip" ]]; then
		error "No external IP found for VM"
		return 1
	fi

	# Use the exact SSH configuration that gcloud uses
	local ssh_key_path="$HOME/.ssh/google_compute_engine"
	local known_hosts_path="$HOME/.ssh/google_compute_known_hosts"
	local host_key_alias="compute.$(gcloud compute instances describe "${TDX_VM_NAME}" --zone="${TDX_VM_ZONE}" --project="${RESOLVED_PROJECT_ID}" --account="${RESOLVED_GCP_ACCOUNT}" --format="value(id)")"

	# Build SSH command exactly like gcloud does
	local ssh_cmd="ssh -i $ssh_key_path -o CheckHostIP=no -o HashKnownHosts=no -o HostKeyAlias=$host_key_alias -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$known_hosts_path"

	info "Using rsync with gcloud SSH configuration to $external_ip..."

	# Run rsync with the exact SSH configuration gcloud uses
	if rsync -avz --delete $rsync_excludes -e "$ssh_cmd" "$source_dir/" ubuntu@"$external_ip":/home/ubuntu/nxcc/; then
		success "Code synced successfully to VM!"
		info "Remote path: /home/ubuntu/nxcc/"
	else
		error "rsync failed to sync code to VM"
		return 1
	fi
}
