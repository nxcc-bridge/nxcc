# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

### Building the Project
```bash
# Build all workspace members in debug mode
cargo build

# Build all workspace members in release mode  
cargo build --release

# Build specific packages
cargo build -p nxcc-daemon -p nxcc-platform-enclave -p nxcc-workerd-vm
```

### Running the System
```bash
# Start a single NXCC node locally
./run.sh

# Start multiple nodes (e.g., 3 nodes)
./run.sh 3
```

### Testing
```bash
# Build and run full integration tests
cargo build && sh tests/integration_test.sh

# Run unit tests
cargo test

# Run tests for specific workspace member
cargo test -p nxcc-daemon
```

### Development Tools
- Use `grpcurl` for interacting with gRPC services during development
- Integration tests use `foundry` (forge/cast/anvil) for smart contract testing
- JavaScript workers are built with `npm run build` in the `tests/js_workers` directory

## Architecture Overview

This is a Rust workspace implementing the NXCC (Network eXecutable Cross-Chain) platform - a confidential computing system for cross-chain applications.

### Core Components

**daemon** (`nxcc-daemon`): The main orchestrator that:
- Manages P2P networking via libp2p (Kademlia DHT, GossipSub, mDNS)
- Handles work order submission and orchestration
- Manages Web3 event subscriptions and gateway connections
- Provides HTTP and gRPC APIs for external interaction
- Coordinates with enclave for secure execution

**enclave** (`nxcc-platform-enclave`): Trusted execution environment that:
- Executes workers in isolated environments
- Manages secrets and cryptographic operations
- Handles secure inter-node secret sharing
- Implements policy enforcement for worker execution

**vm** (base and workerd): Virtual machine implementations:
- `vm/base`: Core VM abstraction and client/server interfaces
- `vm/workerd`: Cloudflare Workerd-based JavaScript runtime for workers

**interface**: Shared protobuf definitions for inter-service communication

**chainlist**: Chain configuration management with JSON chain definitions

### Key Workflows

1. **Work Order Execution**: Work orders contain DSSE-signed worker bundles and are distributed across nodes for execution in TEEs

2. **Secret Sharing**: Secrets are generated in enclaves and shared across nodes using P2P protocols with cryptographic guarantees

3. **Cross-Chain Events**: Workers can subscribe to Web3 events across multiple chains and trigger cross-chain operations

4. **HTTP Workers**: Workers can handle HTTP requests routed through the daemon's HTTP server

### Development Notes

- The system uses Unix domain sockets for secure inter-process communication
- Integration tests simulate multi-node P2P scenarios with temporary directories
- Workers are JavaScript code executed in isolated Workerd VMs
- All cross-node communication is authenticated and encrypted
- Web3 integration supports multiple EVM chains via Alloy provider abstractions
