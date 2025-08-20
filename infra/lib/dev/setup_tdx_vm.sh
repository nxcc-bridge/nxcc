#!/bin/bash
#
# NXCC TDX Development VM Setup Script
# This script handles all the setup logic for the TDX development environment
#
# This script is designed to be:
# - Testable and lintable
# - Idempotent (can be run multiple times safely)
# - Self-contained with clear error handling

set -e
set -o pipefail

# Configuration
SETUP_COMPLETE_FILE="/home/ubuntu/.nxcc-setup-complete"
LOG_FILE="/var/log/nxcc-setup.log"
SCRIPT_DIR="$(dirname "${BASH_SOURCE[0]}")"

# Logging function
log() {
	echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

error() {
	log "ERROR: $*" >&2
	exit 1
}

success() {
	log "SUCCESS: $*"
}

info() {
	log "INFO: $*"
}

# Check if setup is already complete
if [[ -f "$SETUP_COMPLETE_FILE" ]]; then
	log "✅ Setup already completed previously"
	cat "$SETUP_COMPLETE_FILE"
	exit 0
fi

log "=== Starting NXCC TDX Development VM Setup ==="

# Ensure we're running as root for system setup
if [[ $EUID -ne 0 ]]; then
	error "This script must be run as root (use sudo)"
fi

# Step 1: Verify TDX environment
info "Step 1: Verifying TDX guest environment..."
if grep -q "tdx_guest" /proc/cpuinfo; then
	success "TDX guest environment detected in CPU flags"
else
	error "TDX guest not detected in CPU flags - not in proper TDX environment"
fi

# Check TDX device availability
TDX_DEVICE=""
if [[ -c /dev/tdx_guest ]]; then
	TDX_DEVICE="/dev/tdx_guest"
	success "TDX device available: $TDX_DEVICE"
elif [[ -c /dev/tdx-guest ]]; then
	TDX_DEVICE="/dev/tdx-guest"
	success "TDX device available: $TDX_DEVICE"
else
	error "No TDX device found (/dev/tdx_guest or /dev/tdx-guest)"
fi

# Check TSM configfs
if [[ -d /sys/kernel/config/tsm/report ]]; then
	success "TSM configfs interface available"
else
	error "TSM configfs interface not found"
fi

# Step 2: Install additional development tools
info "Step 2: Installing additional development dependencies..."
apt-get update -q
apt-get install -y -q \
	"linux-headers-$(uname -r)" \
	linux-modules-extra-gcp \
	curl \
	wget \
	rsync \
	htop \
	tree \
	unzip

# Step 3: Create ubuntu user directories and permissions
info "Step 3: Setting up user directories..."
mkdir -p /home/ubuntu/nxcc
chown -R ubuntu:ubuntu /home/ubuntu/nxcc

# Step 4: Install TDX verification tools
info "Step 4: Installing TDX verification tools..."

# Copy TDX verification Python script
cp "$SCRIPT_DIR/tdx_verification.py" /home/ubuntu/tdx_verification.py

# Copy development container script
cp "$SCRIPT_DIR/dev-container.sh" /home/ubuntu/dev-container.sh

# Copy NXCC repository setup script
cp "$SCRIPT_DIR/setup-nxcc.sh" /home/ubuntu/setup-nxcc.sh

# Step 5: Set proper permissions
info "Step 5: Setting file permissions..."
chmod +x /home/ubuntu/dev-container.sh
chmod +x /home/ubuntu/setup-nxcc.sh
chmod +x /home/ubuntu/tdx_verification.py
chown ubuntu:ubuntu /home/ubuntu/tdx_verification.py
chown ubuntu:ubuntu /home/ubuntu/dev-container.sh
chown ubuntu:ubuntu /home/ubuntu/setup-nxcc.sh

# Step 6: Test TDX functionality
info "Step 6: Running TDX verification test..."
if python3 /home/ubuntu/tdx_verification.py; then
	success "TDX verification completed successfully"
else
	error "TDX verification failed - environment may not support TDX properly"
fi

# Step 7: Create setup completion marker
info "Step 7: Creating setup completion marker..."
cat >"$SETUP_COMPLETE_FILE" <<EOF
NXCC TDX Development VM Setup Complete
=====================================
Completed: $(date)
Ubuntu Version: $(lsb_release -d | cut -f2)
Kernel Version: $(uname -r)
TDX Device: $TDX_DEVICE

Available Scripts:
  sudo python3 tdx_verification.py  # Re-run TDX hardware verification
  ./setup-nxcc.sh        # Clone/setup NXCC repository  
  ./dev-container.sh     # Start development container

TDX Development Commands:
  cargo build                                    # Development mode
  cargo build --features tdx-hardware-required  # Production mode
  cargo test -p nxcc-attestation                # Attestation tests

Environment Details:
  CPU: $(lscpu | grep "Model name" | cut -d: -f2 | xargs)
  TDX Device: $(ls -la $TDX_DEVICE)
EOF

# Start development container as part of setup
log "Starting NXCC development container..."
container_script="${CONTAINER_SETUP_SCRIPT:-${SCRIPT_DIR}/setup_container.sh}"

if [ -f "$container_script" ]; then
	log "Running container setup script: $container_script"

	# Stream container setup output to both log and stdout
	log "=== Container Setup Output ==="
	if bash "$container_script" 2>&1 | tee -a "$LOG_FILE"; then
		success "Development container started successfully!"
		log "=== Container Setup Complete ==="
	else
		error "Failed to start development container"
		log "=== Container Setup Failed ==="
		exit 1
	fi
else
	# Fallback to inline container setup if script not found
	log "Container setup script not found, using fallback method..."
	CONTAINER_IMAGE="${NXCC_DEV_IMAGE:-ghcr.io/nxcc-bridge/dev:latest}"
	CONTAINER_NAME="nxcc-dev-container"

	log "=== Container Setup Output (Fallback) ==="
	log "📦 Pulling NXCC development container: $CONTAINER_IMAGE"
	docker pull "$CONTAINER_IMAGE" 2>&1 | tee -a "$LOG_FILE" || log "Warning: Could not pull container image"

	log "🛑 Stopping existing container if running..."
	docker stop "$CONTAINER_NAME" 2>/dev/null || true
	docker rm "$CONTAINER_NAME" 2>/dev/null || true

	log "🚀 Starting development container..."
	if docker run -d \
		--name "$CONTAINER_NAME" \
		--privileged \
		--device /dev/tdx_guest:/dev/tdx_guest \
		-v /home/ubuntu/nxcc:/workspace \
		-v /sys/kernel/config:/sys/kernel/config:ro \
		-w /workspace \
		"$CONTAINER_IMAGE" \
		sleep infinity 2>&1 | tee -a "$LOG_FILE"; then

		log "🔍 Verifying container status..."
		if docker ps | grep -q "$CONTAINER_NAME"; then
			success "Development container started successfully (fallback method)!"
			log "📁 Code mounted at: /workspace"
			log "🔒 TDX device available in container"
			log "🌐 Container name: $CONTAINER_NAME"
		else
			error "Container started but verification failed"
			log "Container status:"
			docker ps -a | grep "$CONTAINER_NAME" 2>&1 | tee -a "$LOG_FILE" || true
			exit 1
		fi
	else
		error "Failed to start development container (fallback method)"
		exit 1
	fi
	log "=== Container Setup Complete (Fallback) ==="
fi

chown ubuntu:ubuntu "$SETUP_COMPLETE_FILE"

success "NXCC TDX Development VM setup completed successfully!"
log "🚀 Environment ready for NXCC confidential computing development"
log "📝 Setup details saved to: $SETUP_COMPLETE_FILE"

exit 0
