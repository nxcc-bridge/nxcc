# NXCC TDX Development Environment

This directory contains tools for managing TDX (Intel Trust Domain Extensions) development VMs on Google Cloud Platform. The TDX environment enables confidential computing development with hardware-based trusted execution environments.

## What This Does

The TDX development environment provides:

1. **Automated TDX VM Creation**: Provisions GCP VMs with TDX confidential computing enabled
2. **Complete Development Setup**: Installs Docker, Python, build tools, and development dependencies
3. **TDX Verification**: Automatically tests TDX functionality (TDREPORT generation and TSM quote generation)
4. **Containerized Development**: Pulls and runs the NXCC development container with TDX support
5. **Code Synchronization**: Automatically syncs your local NXCC repository to the VM
6. **Ready-to-Use Environment**: Provides immediate access to a fully configured TDX development environment

## Quick Start

```bash
# Create and configure a complete TDX development environment
./infra.sh dev create

# Connect to your development environment
./infra.sh dev ssh

# Sync code changes from local to VM
./infra.sh dev push

# Check VM status
./infra.sh dev status

# Clean up when done
./infra.sh dev destroy
```

## Commands

### `./infra.sh dev create [--dedicated]`

Creates a complete TDX development environment:

- Provisions TDX-enabled GCP VM (c3-standard-4 with confidential computing)
- Uses **preemptible instances by default** for cost savings
- Installs Docker, Python, build tools, and development dependencies
- Verifies TDX functionality (TDREPORT and quote generation)
- Pulls NXCC development container image
- Syncs local repository to VM
- Starts development container with code mounted
- Returns ready-to-use development environment

**Options:**

- `--dedicated`: Use dedicated (non-preemptible) instances for guaranteed availability

### `./infra.sh dev ssh [-- command]`

Connect to the TDX development VM:

```bash
# Interactive shell
./infra.sh dev ssh

# Run specific command
./infra.sh dev ssh -- 'docker ps'

# Access development container
./infra.sh dev ssh -- 'docker exec -it nxcc-dev-container bash'
```

### `./infra.sh dev push [source_dir]`

Sync local code to the development VM:

```bash
# Sync current directory
./infra.sh dev push

# Sync specific directory
./infra.sh dev push /path/to/nxcc
```

### `./infra.sh dev status`

Show VM status, IP address, and connection information.

### `./infra.sh dev destroy`

Delete the TDX development VM and clean up resources.

### `./infra.sh dev container [--detached]`

Start or restart the development container:

```bash
# Interactive container
./infra.sh dev container

# Background container
./infra.sh dev container --detached
```

## Configuration

### Environment Variables

- `TDX_VM_NAME`: VM name (default: `nxcc-tdx-dev`)
- `TDX_VM_ZONE`: GCP zone (default: `europe-west4-a`)
- `TDX_VM_MACHINE_TYPE`: Machine type (default: `c3-standard-4`)
- `TDX_VM_PREEMPTIBLE`: Use preemptible instances (default: `true`)
- `NXCC_DEV_IMAGE`: Development container image (default: `ghcr.io/nxcc-bridge/dev:latest`)
- `GCP_ACCOUNT`: Specific GCP account to use
- `GCP_PROJECT`: Specific GCP project to use

### Example with Custom Configuration

```bash
# Create dedicated (non-preemptible) VM
./infra.sh dev create --dedicated

# Use different machine type and custom dev image
TDX_VM_MACHINE_TYPE=c3-standard-8 NXCC_DEV_IMAGE=my-registry/nxcc-dev:custom ./infra.sh dev create

# Force dedicated instances via environment variable
TDX_VM_PREEMPTIBLE=false ./infra.sh dev create

# Use specific GCP account/project
GCP_ACCOUNT=myaccount@example.com GCP_PROJECT=my-project ./infra.sh dev create
```

## TDX Requirements

### Hardware Requirements

- Intel 4th Gen Xeon Scalable processors (Sapphire Rapids) with TDX support
- GCP C3 machine family with confidential computing enabled

### GCP Setup Requirements

- GCP account with Compute Engine API enabled
- Authenticated gcloud CLI (535.0.0+ recommended)
- Sufficient compute quotas for confidential computing VMs

### Automatic Verification

The setup automatically verifies:

- TDX guest environment detection
- TDREPORT generation via `/dev/tdx_guest` ioctl
- TSM (Trusted Security Module) configfs interface
- Quote generation capabilities
- Memory encryption activation

### Manual Verification

You can re-run TDX verification anytime on the VM:

```bash
# Connect to VM and run verification
./infra.sh dev ssh -- 'sudo python3 tdx_verification.py'

# Or run directly on VM
ssh ubuntu@<vm-ip>
sudo python3 tdx_verification.py
```

## Development Workflow

1. **Initial Setup**:

   ```bash
   ./infra.sh dev create
   ```

2. **Development Cycle**:

   ```bash
   # Make code changes locally
   vim src/main.rs

   # Sync changes to VM
   ./infra.sh dev push

   # Build and test in TDX environment
   ./infra.sh dev ssh -- 'docker exec -it nxcc-dev-container bash'
   ```

3. **Container Management**:

   ```bash
   # Restart development container if needed
   ./infra.sh dev ssh -- './dev-container.sh --detached'

   # Check container status
   ./infra.sh dev ssh -- 'docker ps'
   ```

4. **Cleanup**:
   ```bash
   ./infra.sh dev destroy
   ```

## Container Environment

The development container includes:

- Rust toolchain with TDX attestation support
- Development and production compilation modes
- Complete NXCC build environment
- TDX testing and verification tools
- Code mounted at `/workspace`

### Development vs Production Modes

**Development Mode** (default in container):

```bash
cargo build                    # Allows simulation fallback
cargo test                     # Tests pass on non-TDX systems
```

**Production Mode** (TDX hardware required):

```bash
cargo build --features tdx-hardware-required    # Hardware-only, no simulation
cargo test --features tdx-hardware-required     # Requires real TDX
```

## Troubleshooting

### VM Creation Issues

```bash
# Check gcloud authentication
gcloud auth list

# Verify project and quotas
gcloud compute project-info describe --project=YOUR_PROJECT

# Check available machine types
gcloud compute machine-types list --zones=europe-west4-a --filter="name~c3"
```

### TDX Verification Issues

```bash
# Check TDX status on VM
./infra.sh dev ssh -- 'sudo dmesg | grep -i tdx'
./infra.sh dev ssh -- './test-tdx.sh'

# Verify TDX device
./infra.sh dev ssh -- 'ls -la /dev/tdx*'
```

### Container Issues

```bash
# Check container status
./infra.sh dev ssh -- 'docker ps -a'

# Restart container
./infra.sh dev ssh -- './dev-container.sh --detached'

# Check logs
./infra.sh dev ssh -- 'docker logs nxcc-dev-container'
```

### Networking Issues

```bash
# Check VM external IP
./infra.sh dev status

# Verify SSH connectivity
gcloud compute ssh ubuntu@nxcc-tdx-dev --zone=europe-west4-a
```

## Security Considerations

- VMs are created with confidential computing enabled
- TDX provides hardware-based memory encryption and attestation
- Development container runs with limited privileges
- SSH access uses generated key pairs stored in `~/.ssh/nxcc-tdx-dev`
- Code is synced using git-tracked files only

## Cost Management

- **Preemptible instances by default**: Significantly reduced costs (~70% savings)
- C3 machine types with confidential computing may incur additional charges
- VMs are not automatically stopped - use `./infra.sh dev destroy` when done
- **Preemptible limitations**: May be terminated with 30-second notice, not suitable for long-running tasks
- Use `--dedicated` flag for guaranteed availability when needed

## Files Created

- `~/.ssh/nxcc-tdx-dev*`: SSH key pair for VM access
- `/home/ubuntu/nxcc/`: Synced repository code on VM
- `/home/ubuntu/*.sh`: TDX testing and setup scripts
- `/var/log/nxcc-setup.log`: Setup process logs
