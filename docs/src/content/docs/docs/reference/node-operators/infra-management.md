---
title: Infrastructure Management
description: Complete guide to the infra.sh script for deploying and managing nXCC infrastructure.
---

The `infra.sh` script is the comprehensive infrastructure management tool for nXCC. It handles everything from local Docker builds to production GCP deployments, TDX development VMs, and CI/CD setup.

## Quick Reference

```bash
# General usage
./infra/infra.sh [-y] <command> <subcommand> [args]

# Build and test locally
./infra/infra.sh image build --debug
./infra/infra.sh image push kind
./infra/infra.sh cluster create kind
./infra/infra.sh k8s deploy debug

# Production GCP deployment
./infra/infra.sh image build --release
./infra/infra.sh image push gcp
./infra/infra.sh cluster create gke
./infra/infra.sh k8s deploy staging

# TDX development environment
./infra/infra.sh dev create
./infra/infra.sh dev ssh
```

## Commands Overview

### Image Commands

Manage Docker images with multi-registry support.

#### `image build`

Builds source images locally:

```bash
./infra/infra.sh image build --debug    # Fast debug builds
./infra/infra.sh image build --release  # Optimized release builds
./infra/infra.sh image build --tag=custom  # Custom local tag
```

- **Modes**: Debug (fast iteration) or Release (optimized)
- **Platform**: Defaults to amd64 for TDX compatibility
- **Output**: Local images tagged as `nxcc-node:debug` or `nxcc-node:latest`

#### `image push`

Push local images to deployment targets:

```bash
./infra/infra.sh image push kind         # Load into KinD cluster
./infra/infra.sh image push gcp          # Push to GCP Artifact Registry
./infra/infra.sh image push gcp --source=debug --tag=staging  # Custom push
```

- **Targets**: `kind`, `gcp`, `aws`, `azure`
- **Options**: `--source=TAG` (local source), `--tag=TAG` (target tag)
- **Requirements**: Target-specific authentication (e.g., GCP setup)

#### `image list`

List images in registries:

```bash
./infra/infra.sh image list          # List GCP registry images (default)
./infra/infra.sh image list local    # List local Docker images
./infra/infra.sh image list gcp      # List GCP Artifact Registry
```

### Cluster Management

Create and destroy Kubernetes clusters for nXCC deployment.

#### `cluster create kind`

Creates a local Kubernetes cluster using KinD:

```bash
./infra/infra.sh cluster create kind
```

- **Use case**: Local development and testing
- **Requirements**: Docker and KinD installed
- **Features**: Pre-configured for nXCC deployment
- **Resources**: Uses local Docker resources

#### `cluster create gke`

Creates a Google Kubernetes Engine cluster:

```bash
./infra/infra.sh cluster create gke
```

- **Use case**: Production deployments
- **Features**: TDX-enabled node pools, auto-scaling
- **Requirements**: GCP project with billing enabled
- **Configuration**: Optimized for confidential computing workloads

#### `cluster destroy <env>`

Destroys the specified cluster:

```bash
./infra/infra.sh cluster destroy kind
./infra/infra.sh cluster destroy gke
```

**⚠️ Warning**: This permanently deletes the cluster and all data.

### Application Deployment

Deploy the nXCC application to Kubernetes clusters using Helm.

#### `k8s deploy <env>`

Deploys or upgrades the nXCC application:

```bash
# Local development deployment
./infra/infra.sh k8s deploy debug

# Staging environment
./infra/infra.sh k8s deploy staging

# Production environment
./infra/infra.sh k8s deploy prod
```

**Environment configurations**:

- **debug**: Local development with debug logging
- **staging**: Pre-production testing environment
- **prod**: Production deployment with optimizations

#### `k8s destroy <env>`

Uninstalls the application from the cluster:

```bash
./infra/infra.sh k8s destroy staging
```

#### `k8s dump-debug <env>`

Dumps diagnostic information for failed deployments:

```bash
./infra/infra.sh k8s dump-debug staging
```

Outputs:

- Pod status and logs
- Service configurations
- Ingress status
- Node information

### CI/CD Management

Set up and manage CI/CD infrastructure on Google Cloud.

#### `ci setup`

Creates all CI/CD resources:

```bash
./infra/infra.sh ci setup
```

Creates:

- **Service Account**: For GitHub Actions authentication
- **Workload Identity Federation**: Secure keyless authentication
- **Artifact Registry**: Container image storage
- **IAM bindings**: Proper permissions for CI/CD

#### `ci teardown`

Deletes all CI/CD resources:

```bash
./infra/infra.sh ci teardown
```

**⚠️ Warning**: This removes all CI/CD infrastructure and stored images.

### Testing

Test connectivity and functionality of deployed nodes.

#### `test <env>`

Tests HTTP connectivity to the deployed nXCC node:

```bash
./infra/infra.sh test staging
./infra/infra.sh test prod
```

Performs:

- Health endpoint checks
- API availability tests
- Basic functionality verification

### TDX Development Environment

Manage TDX-enabled development VMs for real hardware testing.

#### `dev create`

Creates a complete TDX development environment:

```bash
# Create with preemptible instance (cost-effective)
./infra/infra.sh dev create

# Create with dedicated instance (guaranteed availability)
./infra/infra.sh dev create --dedicated
```

**What it creates**:

- TDX-enabled Google Cloud VM
- All development dependencies installed
- Docker and development tools configured
- nXCC codebase prepared for development

**Instance types**:

- **Preemptible** (default): Cost-effective but may be interrupted
- **Dedicated**: Guaranteed availability but higher cost

#### `dev ssh`

SSH into the development VM:

```bash
# Interactive SSH session
./infra/infra.sh dev ssh

# Run specific command
./infra/infra.sh dev ssh -- 'cd nxcc && cargo build'
```

#### `dev push`

Sync local code changes to the development VM:

```bash
./infra/infra.sh dev push
```

- **Scope**: Only git-tracked files are synced
- **Speed**: Incremental sync for fast updates
- **Use case**: Develop locally, test on real TDX hardware

#### `dev container`

Start or restart the development container on the VM:

```bash
# Interactive container
./infra/infra.sh dev container

# Background container
./infra/infra.sh dev container --detached
```

#### `dev local`

Run a local development container with all tools pre-installed:

```bash
# Default platform
./infra/infra.sh dev local

# Specific platform
./infra/infra.sh dev local --platform linux/amd64

# Force rebuild
./infra/infra.sh dev local --build
```

#### `dev status`

Show VM status and connection information:

```bash
./infra/infra.sh dev status
```

Outputs:

- VM status (running/stopped)
- External IP address
- SSH connection command
- Container status

#### `dev destroy` / `dev cleanup`

Destroy the TDX development VM:

```bash
./infra/infra.sh dev destroy
# OR
./infra/infra.sh dev cleanup
```

**⚠️ Warning**: This permanently deletes the VM and all data.

## Configuration and Authentication

### GCP Identity Resolution

The script automatically resolves your GCP identity for `ci` and `gke` commands. Override with environment variables:

```bash
export GCP_ACCOUNT="your-email@domain.com"
export GCP_PROJECT_ID="your-project-id"
./infra/infra.sh ci setup
```

### Interactive Confirmations

Use the `-y` flag to automatically answer 'yes' to all confirmation prompts:

```bash
./infra/infra.sh -y cluster destroy gke
```

## Common Workflows

### Local Development Setup

```bash
# 1. Build debug images
./infra/infra.sh image build --debug

# 2. Create local cluster
./infra/infra.sh cluster create kind

# 3. Load images into cluster
./infra/infra.sh image push kind

# 4. Deploy to local cluster
./infra/infra.sh k8s deploy debug

# 4. Test the deployment
./infra/infra.sh test debug
```

### Production Deployment

```bash
# 1. Setup CI/CD (one-time)
./infra/infra.sh ci setup

# 2. Build and push images
./infra/infra.sh image build --release
./infra/infra.sh image push gcp

# 3. Create production cluster
./infra/infra.sh cluster create gke

# 4. Deploy application
./infra/infra.sh k8s deploy prod

# 5. Test deployment
./infra/infra.sh test prod
```

### TDX Development Workflow

```bash
# 1. Create TDX development VM
./infra/infra.sh dev create

# 2. Push your code
./infra/infra.sh dev push

# 3. SSH and build/test
./infra/infra.sh dev ssh -- 'cd nxcc/node && cargo build'

# 4. Check status
./infra/infra.sh dev status

# 5. Clean up when done
./infra/infra.sh dev destroy
```

## Troubleshooting

### Common Issues

**Permission denied on GCP operations**:

```bash
# Ensure you're authenticated
gcloud auth login
gcloud config set project YOUR-PROJECT-ID
```

**KinD cluster creation fails**:

```bash
# Check Docker is running
docker info

# Clean up any existing clusters
kind delete cluster --name nxcc-local
```

**TDX VM creation fails**:

```bash
# Check quota in the region
gcloud compute project-info describe --project=YOUR-PROJECT

# Try a different region
export GOOGLE_CLOUD_REGION="us-west1"
```

**Build failures**:

```bash
# Check disk space
df -h

# Clean Docker cache
docker system prune -f
```

### Debug Information

**Get cluster info**:

```bash
kubectl cluster-info
kubectl get nodes -o wide
```

**Check application status**:

```bash
kubectl get pods -n nxcc
kubectl logs -n nxcc deployment/nxcc-daemon
```

**VM diagnostics**:

```bash
./infra/infra.sh dev ssh -- 'dmesg | grep -i tdx'
./infra/infra.sh dev ssh -- 'lscpu | grep -i tdx'
```

## Advanced Configuration

### Environment Variables

- **`GCP_ACCOUNT`**: Override GCP account
- **`GCP_PROJECT_ID`**: Override GCP project
- **`GOOGLE_CLOUD_REGION`**: Set deployment region
- **`BUILD_PLATFORMS`**: Override build platforms
- **`BUILD_MODE`**: Override build mode (debug/release)

### Custom Resource Limits

Edit the Helm values files for custom resource configurations:

```bash
# Location of Helm charts and values
ls infra/k8s/charts/nxcc/values-*.yaml
```

### Network Configuration

For custom networking requirements, modify the Kubernetes manifests:

```bash
# Location of Kubernetes manifests
ls infra/k8s/manifests/
```

## Security Considerations

- **TDX VMs**: Use dedicated instances for production TDX testing
- **GCP IAM**: Follow principle of least privilege for service accounts
- **Secrets**: Never commit GCP credentials to version control
- **Network**: Use private clusters for production deployments
- **Images**: Regularly update base images for security patches

## Next Steps

- **[Running a Node](/docs/reference/node-operators/running-a-node/)** - Learn about local development node setup
- **[Local Development Workflow](/docs/guides/development-workflow/)** - Master the development cycle
- **[CLI Reference](/docs/reference/cli/)** - Understand the developer CLI tools
