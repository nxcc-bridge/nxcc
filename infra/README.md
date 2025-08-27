# nXCC Infrastructure

Deployment and infrastructure management tools for nXCC platform.

## Overview

This directory provides comprehensive infrastructure automation for:

- **🏗️ Build Systems**: Docker image builds and CI/CD pipelines
- **☸️ Kubernetes Deployment**: Helm charts and cluster management
- **🔧 Development Environment**: TDX-enabled development VMs
- **🌍 Multi-Environment Support**: Local, staging, and production deployments

## Quick Start

### Prerequisites

- Docker
- gcloud CLI (for GCP deployments)
- kubectl (for Kubernetes management)
- Helm (installed automatically)

### Local Development

```bash
# Build and deploy locally with KinD
./infra.sh build local
./infra.sh cluster create local

# Create TDX development VM
./infra.sh dev create
```

### Production Deployment

```bash
# Set up CI/CD infrastructure
./infra.sh ci setup

# Build and push production images
./infra.sh build gcp

# Deploy to staging/production
./infra.sh cluster create staging
./infra.sh k8s deploy staging
```

## Commands

### Build Management

```bash
./infra.sh build <local|gcp>           # Build Docker images
./infra.sh ci <setup|teardown>         # Manage CI/CD resources
```

### Cluster Operations

```bash
./infra.sh cluster <create|destroy> <gke|kind>       # Manage Kubernetes clusters
./infra.sh k8s <deploy|destroy|dump-debug> <debug|staging|prod>  # Deploy applications
```

### Development Environment

```bash
./infra.sh dev <create|destroy|ssh|push|status|container|local>  # TDX development VMs
./infra.sh test <env>                  # Test HTTP connectivity
```

## Architecture

### Build System (`lib/build.sh`)

- Multi-architecture Docker builds (amd64/arm64)
- TDX-optimized release builds
- GCP Artifact Registry integration
- Build caching and optimization

### Cluster Management (`lib/cluster.sh`, `lib/k8s.sh`)

- KinD for local development
- GKE for cloud deployments
- Automatic ingress and networking
- Multi-environment isolation

### Helm Charts (`charts/nxcc-node/`)

Kubernetes deployment templates for:

- nXCC daemon pods with P2P networking
- Enclave containers with TEE support
- Service meshes and ingress configuration
- Resource limits and security policies

### Development Tools (`lib/dev/`)

- **[TDX Development VMs](lib/dev/)**: Intel TDX confidential computing environment
- Automated VM provisioning and setup
- Container-based development workflow
- Code synchronization and testing tools

## Environments

### Local (`local`)

- **Platform**: KinD (Kubernetes in Docker)
- **Build Mode**: Debug (faster iteration)
- **Network**: Port-forwarding for access
- **Use Case**: Development and testing

### Staging (`staging`)

- **Platform**: GKE (Google Kubernetes Engine)
- **Build Mode**: Debug with caching
- **Network**: Public ingress with load balancer
- **Use Case**: Integration testing and validation

### Production (`prod`)

- **Platform**: GKE with enhanced security
- **Build Mode**: Release (optimized)
- **Network**: Production ingress with CDN
- **Use Case**: Live deployment

## Configuration

### Environment Variables

```bash
# GCP Configuration
export GCP_PROJECT_ID="your-project-id"
export GCP_ACCOUNT="your-email@domain.com"
export GCP_AR_LOCATION="europe"              # Artifact Registry location
export GCP_GKE_REGION="europe-west4"       # GKE cluster region

# Build Configuration
export AUTO_YES="true"                      # Skip confirmation prompts
export IMAGE_REPO_OVERRIDE="custom-repo"    # Override image repository
export IMAGE_TAG_OVERRIDE="custom-tag"      # Override image tag

# TDX Development VM
export TDX_VM_NAME="nxcc-tdx-dev"          # VM instance name
export TDX_VM_ZONE="europe-west4-a"         # GCP zone
export TDX_VM_MACHINE_TYPE="c3-standard-4" # Machine type
export TDX_VM_PREEMPTIBLE="true"           # Use preemptible instances
export NXCC_DEV_IMAGE="ghcr.io/nxcc-bridge/dev:latest"  # Dev container image
```

### Customization

- **Helm Values**: Edit `charts/nxcc-node/values.yaml`
- **Build Args**: Modify `lib/build.sh` Docker arguments
- **VM Config**: Adjust TDX settings in `lib/dev/vm.sh`

## Development Workflow

1. **Local Setup**:

   ```bash
   ./infra.sh build local
   ./infra.sh cluster create local
   ```

2. **TDX Development**:

   ```bash
   ./infra.sh dev create
   ./infra.sh dev ssh
   ```

3. **Testing & Validation**:

   ```bash
   ./infra.sh test integration local
   ./infra.sh test e2e staging
   ```

4. **Production Deployment**:
   ```bash
   ./infra.sh build gcp
   ./infra.sh k8s deploy prod
   ```

For detailed TDX development environment usage, see the [development environment documentation](lib/dev/).
