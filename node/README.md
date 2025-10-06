# nXCC Node

Core Rust workspace implementing the nXCC (Network eXecutable Cross-Chain) platform.

## Architecture

nXCC is built as a multi-process system with secure communication channels:

**Core Architecture:**

- **Daemon**: P2P networking, work orchestration, HTTP APIs
- **Enclave**: TEE-based execution environment (Intel TDX/SGX)
- **VM**: JavaScript worker runtime (Cloudflare Workerd)
- **Interface**: gRPC/protobuf communication layer

**Communication:**

- Unix Domain Sockets for inter-process communication
- gRPC with protobuf serialization for structured calls
- Event queues for asynchronous processing

## Components

### Core Services

- **[daemon/](daemon/)**: P2P networking, work orchestration, and API endpoints
- **[enclave/](enclave/)**: Trusted execution environment with Intel TDX/SGX support
- **[vm/](vm/)**: JavaScript worker runtime based on Cloudflare Workerd
- **[interface/](interface/)**: Shared gRPC/protobuf communication protocols

### Support Libraries

- **[chainlist/](chainlist/)**: 400+ blockchain network configurations
- **[attestation/](attestation/)**: TDX/SGX attestation and verification logic

## Quick Start

### Prerequisites

- Rust 1.89.0+ with edition 2024
- Docker (recommended for deployment)
- Cloudflare `workerd` runtime for the JavaScript VM:
  - macOS (x86_64 / arm64): `brew install cloudflare/workers/workerd`
  - Linux x86_64:
    ```bash
    BASE=https://github.com/cloudflare/workerd/releases/latest/download
    curl -fsSLo workerd.gz "$BASE/workerd-linux-64.gz"
    gunzip workerd.gz
    chmod +x workerd
    sudo mv workerd /usr/local/bin/
    ```
  - Linux arm64 (aarch64):
    ```bash
    BASE=https://github.com/cloudflare/workerd/releases/latest/download
    curl -fsSLo workerd.gz "$BASE/workerd-linux-arm64.gz"
    gunzip workerd.gz
    chmod +x workerd
    sudo mv workerd /usr/local/bin/
    ```
  - If you keep the binary elsewhere, export `NXCC_WORKERD_BINARY_PATH=/path/to/workerd` (or `WORKERD_BIN_PATH`) before running `./run.sh`.
- Intel TDX/SGX hardware (for production attestation)

### Development

```bash
# Build all components
cargo build

# Run unit tests
cargo test

# Start local node (all components)
# Requires workerd on PATH or NXCC_WORKERD_BINARY_PATH to be set
./run.sh

# Start multi-node cluster
./run.sh 3
```

### Docker Deployment

```bash
# Build production image
docker build -t nxcc-node .

docker run --rm \
  --add-host=host.docker.internal:host-gateway \
  -p 127.0.0.1:6922:6922 \
  -p 127.0.0.1:9000:9000 \
  nxcc-node
```

## Development Workflow

### Building

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Build specific component
cargo build -p nxcc-daemon
```

### Testing

```bash
# Unit tests
cargo test

# Integration tests (multi-node P2P)
cargo build && sh tests/integration_test.sh

# Component-specific tests
cargo test -p nxcc-enclave
```

### Code Quality

```bash
# Format code (Rust edition 2024)
cargo fmt

# Check formatting
cargo fmt -- --check

# Run linter
cargo clippy --all-features --profile test
```

## Configuration

### Environment Variables

```bash
# Logging
export RUST_LOG="nxcc_daemon=debug"

# Network configuration
export DAEMON_P2P_LISTEN_ADDR="/ip4/0.0.0.0/tcp/9000"
export DAEMON_GRPC_TARGET_ADDR="0.0.0.0:50051"

# TEE configuration
export ENCLAVE_UDS_SOCKET="/run/nxcc/enclave.sock"
export WORKERD_BIN_PATH="/usr/local/bin/workerd"
```

### TDX Attestation Modes

```bash
# Development mode (simulation allowed)
cargo test

# Production mode (hardware required)
TDX_TESTS_REQUIRE_HARDWARE=true cargo test
```

## Inter-Process Communication

Components communicate via:

- **Unix Domain Sockets**: Enclave ↔ Daemon, VM ↔ Daemon
- **gRPC**: Structured service calls with protobuf serialization
- **Event Queues**: Asynchronous event delivery and processing

### Protocol Definitions

All inter-service protocols defined in [interface/proto/](interface/proto/):

- `interface.proto`: Core platform APIs
- `enclave.proto`: TEE-specific operations
- Generated Rust bindings in `interface/src/`

## Security Model

### Trusted Execution Environments

- **TDX/SGX Enclaves**: Workers execute in hardware-protected memory domains
- **Remote Attestation**: TDX Quote Verification Library (QVL) provides cryptographic proof of genuine TEE execution
- **Memory Encryption**: Hardware encrypts all code and data within the enclave
- **Ephemeral Key Exchange**: X25519 keypairs generated within TEE for secure inter-enclave communication

### Cryptographic Security

- **AES-256-GCM-SIV**: Symmetric encryption for secrets and worker data
- **X25519 ECDH**: Key exchange protocol for establishing secure channels between enclaves
- **SHA-256**: Hashing for attestation binding and authorization IDs
- **Ed25519**: Digital signatures for P2P node identity verification

### Communication Security

- **Unix Domain Sockets**: Local inter-process communication between daemon, enclave, and VM
- **gRPC with TLS**: Encrypted remote procedure calls for structured communication
- **Attestation Binding**: All secrets cryptographically bound to specific TEE measurements

### Access Control

- **Policy-Based Authorization**: On-chain smart contracts define who can access which secrets
- **Time-Limited Tokens**: EIP-4907 user roles provide temporary access delegation
- **Distributed Secret Sharing**: Secrets distributed across multiple TEE nodes using threshold cryptography

## Performance

For detailed performance metrics and benchmarking, see [benchmark results](../benchmarks/).

## Deployment

### Local Development

```bash
./run.sh                    # Single node
./run.sh 3                  # 3-node cluster
```

### Docker

```bash
docker build -t nxcc-node .
docker run -p 9000:9000 -p 50051:50051 nxcc-node
```

### Kubernetes

See [infrastructure documentation](../infra/) for production deployment.

## Integration

### Platform Integration Points

- **[Smart Contracts](../contracts/)**: On-chain identity and policy management
- **[SDK/CLI](../sdk/)**: Worker development and deployment tools
- **[Benchmarks](../benchmarks/)**: Performance testing and validation
- **[E2E Tests](../e2e/)**: Full-stack integration testing

### External Dependencies

- **libp2p**: Decentralized networking and peer discovery
- **Alloy**: Ethereum provider abstractions and Web3 integration
- **Workerd**: Cloudflare's JavaScript runtime for worker execution
- **Tokio**: Async runtime and networking foundations

## Development Notes

### TEE Configuration

- **Testing Modes**: Use `TDX_TESTS_REQUIRE_HARDWARE=true` for hardware-only testing
- **Device Access**: TEE operations require `/dev/tdx_guest` or similar device files

### Network Configuration

- **P2P Ports**: Default P2P port 9000 must be accessible for node communication
- **Discovery**: mDNS discovery requires multicast-enabled networks

For detailed component documentation, see individual component READMEs.
