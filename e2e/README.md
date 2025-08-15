# NXCC End-to-End Testing

This directory contains the comprehensive end-to-end test suite for the NXCC platform.

## Overview

The E2E test suite validates the complete NXCC workflow by testing the entire pipeline from infrastructure setup to worker deployment and verification across multiple environments (local, staging, production).

### What the E2E Tests Accomplish

1. **🏗️ Infrastructure Setup**: Creates and configures Kubernetes clusters (local kind or GKE)
2. **📦 Project Initialization**: Uses the NXCC CLI to create a new worker project from templates
3. **⚡ Worker Development**: Creates HTTP handlers with echo functionality for comprehensive testing
4. **🔨 Build & Bundle**: Compiles TypeScript workers and creates deployment bundles
5. **🚀 Deployment**: Deploys workers to NXCC nodes using the CLI
6. **✅ Verification**: Tests HTTP endpoints and verifies worker logs to ensure functionality
7. **🌍 Multi-Environment**: Supports local (kind), staging (GKE), and production (GKE) environments

## Files Structure

```
e2e/
├── e2e_test.sh              # Main test orchestration script
├── README.md                # This documentation
└── lib/
    ├── common.sh            # Shared utilities, logging, and HTTP testing
    ├── cluster.sh           # Cluster setup, teardown, and connectivity
    └── worker.sh            # Worker creation, deployment, and testing
```

## Usage

### Quick Start

From the project root, run the complete local test:
```bash
# From project root
./e2e/e2e_test.sh

# Or change to e2e directory first
cd e2e
./e2e_test.sh
```

### Options

```bash
./e2e_test.sh [options]

Options:
  --env local|staging|prod    Environment to test (default: local)
  --skip-cluster-setup        Skip cluster creation (assumes cluster exists)
  --skip-cleanup              Skip cleanup at the end
  --test-staging              Also test staging deployment after local
  --verbose                   Enable verbose logging
  --force-cleanup             Force cleanup of cluster resources
  --debug                     Use debug builds for faster development (default)
  --release                   Use release builds for production testing
  --help                      Show help message
```

### Examples

```bash
# Run complete local test with verbose output
cd e2e && ./e2e_test.sh --verbose

# Test existing local cluster without setup/cleanup  
cd e2e && ./e2e_test.sh --skip-cluster-setup --skip-cleanup

# Test local then staging environments
cd e2e && ./e2e_test.sh --test-staging

# Test only staging environment (requires GCP setup)
cd e2e && ./e2e_test.sh --env staging

# Test with release builds for performance testing
cd e2e && ./e2e_test.sh --release --env staging

# Force cleanup after test
cd e2e && ./e2e_test.sh --force-cleanup
```

### Environment Variables

The test script supports several environment variables for customization:

```bash
# Build configuration
export BUILD_MODE="debug"              # Use debug builds (default for e2e)
export BUILD_SINGLE_ARCH="true"        # Single arch builds for faster testing

# Timeout configuration (in seconds)
export E2E_DOCKER_BUILD_TIMEOUT="900"  # Docker build timeout (15 minutes)
export E2E_WORKER_DEPLOY_TIMEOUT="300" # Worker deployment timeout (5 minutes) 
export E2E_HTTP_TEST_TIMEOUT="180"     # HTTP test timeout (3 minutes)
export HELM_TIMEOUT="10m"              # Helm operation timeout

# Testing configuration  
export E2E_VERBOSE="true"              # Enable verbose logging
export E2E_TEST_TEXT="Custom message"  # Override test echo message
```

## What the Tests Are Expected to Do

### Complete End-to-End Validation

The e2e tests simulate a real-world developer workflow by:

1. **🔍 Validating Prerequisites**: Ensuring all required tools are available (docker, kind, kubectl, etc.)

2. **🚀 Setting Up Infrastructure**: 
   - **Local**: Creates a kind cluster and deploys NXCC in debug mode
   - **Staging/Prod**: Connects to GKE cluster and deploys NXCC with ingress

3. **📝 Creating a Test Project**: Uses `nxcc init` to create a new worker project with:
   - TypeScript worker with HTTP handlers
   - Echo functionality that responds with test messages
   - Proper build configuration and bundling

4. **⚙️ Building and Deploying**: 
   - Compiles TypeScript to JavaScript
   - Creates worker bundles using the NXCC CLI
   - Deploys workers to the target environment
   - Verifies deployment success through logs

5. **🌐 Testing HTTP Functionality**:
   - **GET /w/health** - Health check endpoint
   - **GET /w/echo** - Echo endpoint returning test message
   - **POST /w/echo** - Echo endpoint that returns posted data
   - **GET /w/** - Default handler with available endpoints
   - **GET /w/unknown** - Unknown path handler

6. **✅ Verification and Cleanup**:
   - Confirms worker is running and responsive
   - Validates expected responses from all endpoints
   - Retrieves and displays worker logs
   - Cleans up temporary resources

### Expected Success Criteria

A successful e2e test run should:

- ✅ **All HTTP endpoints respond correctly** (5/5 tests pass)
- ✅ **Worker logs show successful launch** with test messages
- ✅ **Echo functionality works** - returns "Hello from NXCC E2E Test!"
- ✅ **Build and deployment complete** without errors
- ✅ **Infrastructure setup succeeds** for target environment
- ✅ **Cleanup completes** leaving no hanging resources

### Environment-Specific Behavior

#### Local Environment (`--env local`)
- Uses **localhost port-forwarding** for all connections
- Deploys to **debug namespace** in kind cluster
- Uses **debug builds** for faster iteration
- **Expected outcome**: Worker accessible at `localhost:6922/w/*`

#### Staging Environment (`--env staging`)  
- Uses **public ingress IP** for HTTP testing (no localhost)
- Deploys to **staging namespace** in GKE cluster
- Uses **debug builds** with caching for faster e2e testing
- **Expected outcome**: Worker accessible at `http://<INGRESS-IP>/w/*`

#### Production Environment (`--env prod`)
- Uses **public ingress IP** for HTTP testing (no localhost)  
- Deploys to **prod namespace** in GKE cluster
- Uses **release builds** by default for optimized performance
- **Expected outcome**: Worker accessible at `http://<INGRESS-IP>/w/*`

## Test Workflow

### 1. Dependency Check
- Verifies Docker, kind, kubectl, curl, jq, node, npm
- Builds NXCC CLI from source if not available

### 2. Cluster Setup
- **Local**: Creates kind cluster, builds Docker image, deploys to debug namespace
- **Staging/Prod**: Creates GKE cluster, builds and pushes to registry, deploys

### 3. Project Creation
- Creates temporary directory
- Initializes NXCC project using CLI
- Creates echo worker with HTTP handlers
- Builds TypeScript and bundles worker

### 4. Worker Testing
- Deploys worker to target environment
- Sets up port forwarding for remote environments
- Tests multiple HTTP endpoints:
  - `GET /w/echo` - Echo test message
  - `POST /w/echo` - Echo JSON data
  - `GET /w/health` - Health check
  - `GET /w/` - Default handler

### 5. Verification
- Retrieves worker logs
- Tests HTTP request/response cycles
- Verifies expected data in responses

### 6. Cleanup
- Removes temporary project directory
- Kills port-forward processes
- Optionally cleans up cluster resources

## Test Worker

The test creates an echo worker with the following handlers:

```typescript
// GET /w/echo - Returns test message and metadata
{
  "message": "Hello from NXCC E2E Test!",
  "timestamp": "2024-01-01T00:00:00.000Z",
  "method": "GET",
  "path": "/echo"
}

// POST /w/echo - Echoes back received data
{
  "message": "Echo received",
  "received": { /* posted data */ },
  "testMessage": "Hello from NXCC E2E Test!",
  "timestamp": "2024-01-01T00:00:00.000Z"
}

// GET /w/health - Health check
{
  "status": "healthy",
  "timestamp": "2024-01-01T00:00:00.000Z",
  "uptime": 123.45
}
```

## Environment Configuration

### Local Environment (`--env local`)
- Uses kind cluster with `nxcc-debug` name in `debug` namespace
- Port-forwarding for both deployment and HTTP testing: `localhost:6922`
- Debug builds for faster development
- Automatic Docker image loading into cluster
- Single architecture (amd64) builds for Intel TDX TEE

### Staging Environment (`--env staging`)  
- Uses GKE cluster in `staging` namespace
- Requires GCP authentication and project setup
- Port-forwarding for worker deployment (`nxcc worker deploy`)
- **Public ingress IP** for HTTP testing (no localhost)
- Debug builds with image caching for faster e2e testing
- Automatic ingress IP detection with fallback to port-forwarding

### Production Environment (`--env prod`)
- Uses GKE cluster in `prod` namespace  
- Requires GCP authentication and project setup
- Port-forwarding for worker deployment (`nxcc worker deploy`)
- **Public ingress IP** for HTTP testing (no localhost)
- Release builds by default (override with `BUILD_MODE=debug`)
- Automatic ingress IP detection with fallback to port-forwarding

## Dependencies

### Required Tools
- Docker
- kind (for local testing)
- kubectl
- curl
- jq
- node (v18+)
- npm

### Optional Tools
- gcloud (for GKE environments)
- helm (installed by infra scripts)

## Future Extensions

The test framework is designed to support:

1. **Contract Testing**: Deploy contracts to anvil/testnet
2. **Web3 Event Testing**: Test event listening and dispatch
3. **Cross-Chain Testing**: Multi-chain worker scenarios
4. **Performance Testing**: Load and stress testing
5. **Security Testing**: Identity and policy validation

## Troubleshooting

### Common Issues

1. **CLI Not Found**: Script will build from source automatically
2. **Port Conflicts**: Uses dynamic port allocation
3. **Cluster Access**: Verifies kubectl context before testing
4. **Image Build**: Rebuilds on failure with verbose output

### Debug Mode

Enable verbose logging to see detailed output:
```bash
export E2E_VERBOSE=true
cd e2e && ./e2e_test.sh --verbose
```

### Manual Cleanup

If the test fails and leaves resources:
```bash
# Clean up port forwards
pkill -f "kubectl port-forward"

# Clean up kind cluster
kind delete cluster --name nxcc-debug

# Clean up temp directories
rm -rf /tmp/tmp.*
```