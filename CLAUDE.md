# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Important Instructions

- **NEVER add Claude as co-author in git commits**. Only include human contributors in commit co-author lines.

## Build and Development Commands

### Building the Project
```bash
# Build all components (Rust workspace)
cargo build

# Build in release mode
cargo build --release

# Build specific workspace members
cargo build -p nxcc-daemon -p nxcc-platform-enclave -p nxcc-workerd-vm
```

### Running the System
```bash
# Start a single NXCC node locally
./node/run.sh

# Start multiple nodes (e.g., 3 nodes for P2P testing)
./node/run.sh 3
```

### Testing
```bash
# Run Rust unit tests
cargo test

# Build and run full integration tests
cargo build && sh node/tests/integration_test.sh

# Run specific workspace tests
cargo test -p nxcc-daemon

# Run end-to-end tests (local environment)
cd e2e && ./e2e_test.sh

# Run end-to-end tests (staging environment - requires GCP setup)
cd e2e && ./e2e_test.sh --env staging
```

### TDX Attestation Test Modes

The attestation system uses environment variables to control test behavior:

**Development Mode** (default):
```bash
cargo test                              # Uses simulation for testing
TDX_TESTS_REQUIRE_HARDWARE=false cargo test  # Explicit simulation mode
```

**Production/CI Mode** (TDX hardware required):
```bash
TDX_TESTS_REQUIRE_HARDWARE=true cargo test   # Requires real TDX hardware
TDX_TESTS_REQUIRE_HARDWARE=1 cargo test      # Alternative syntax
```

Production test mode guarantees:
- Simulation is NEVER used when hardware is explicitly requested
- Panics immediately if TDX hardware unavailable but required
- Cannot be bypassed at runtime - prevents test misconfiguration  
- Use for CI/testing on TDX infrastructure

### Component-Specific Commands

**Smart Contracts** (`contracts/evm/`):
```bash
cd contracts/evm
forge soldeer install  # Install dependencies
forge build            # Build contracts
forge test              # Run tests
```

**Documentation** (`docs/`):
```bash
cd docs
pnpm install
pnpm dev                # Local development server
pnpm build              # Build static site
pnpm lint               # Check formatting
```

**Benchmarks** (`benchmarks/`):
```bash
cd benchmarks
cargo build             # Build Rust benchmarking tools
cd workers && pnpm run build  # Build JavaScript workers
```

**CLI SDK** (`sdk/cli/`):
```bash
cd sdk/cli
pnpm install
pnpm build              # Build TypeScript CLI
pnpm lint               # Check formatting
```

### Code Quality
```bash
# Rust formatting (uses edition = "2024")
cargo fmt

# Check Rust formatting
cargo fmt -- --check

# Shell script formatting (POSIX sh - not bash)
git ls-files '*.sh' | xargs shfmt -w

# Documentation linting (Prettier)
cd docs && pnpm lint
```

## Development

* Always run tests, linters, and formatters after completing a set of changes.
* Always add unit tests (and integration tests, if necessary) when adding new functionality.
* When fixing a bug, always add regression test(s).
* All shell scripts use POSIX sh syntax (not bash) for maximum compatibility.

## Architecture Overview

This is a Rust workspace implementing the NXCC (Network eXecutable Cross-Chain) platform - a confidential computing system that enables secure cross-chain applications through trusted execution environments.

### Multi-Language Components

**Core Platform** (Rust): Located in `node/` directory
- **daemon**: Main orchestrator managing P2P networking, work order orchestration, Web3 event subscriptions
- **enclave**: Trusted execution environment for secure worker execution and secret management
- **vm**: Virtual machine implementations (base abstraction + Cloudflare Workerd runtime)
- **interface**: Shared protobuf definitions for inter-service communication
- **chainlist**: Chain configuration management with 400+ EVM chain definitions

**Smart Contracts** (Solidity): Located in `contracts/evm/`
- **Identity.sol**: Machine Identity NFT contract (ERC-721 + EIP-4907 user roles)
- Uses Foundry toolchain with Soldeer dependency management

**CLI Tools** (TypeScript): Located in `sdk/cli/`
- **@nxcc/cli**: Command-line interface for interacting with NXCC nodes
- Built with Commander.js, includes esbuild for worker bundling

**Documentation** (Astro): Located in `docs/`
- Starlight-based documentation site with Vue components

**Benchmarking** (Rust + JavaScript): Located in `benchmarks/`
- Performance testing tools and JavaScript worker implementations

### Key Technical Concepts

**Work Order Execution**: Work orders contain DSSE-signed JavaScript worker bundles distributed across nodes for execution in trusted environments. Workers can:
- Handle HTTP requests routed through the daemon
- Subscribe to Web3 events across multiple EVM chains
- Perform secure cross-chain operations
- Access shared secrets via cryptographic protocols

**P2P Networking**: Uses libp2p with Kademlia DHT, GossipSub messaging, and mDNS discovery for node coordination and work distribution.

**Secure Communication**: All inter-process communication uses Unix domain sockets, with cross-node communication authenticated and encrypted.

**Multi-Chain Integration**: Built on Alloy provider abstractions supporting 400+ EVM chains via the chainlist component.

### Development Patterns

**Rust Workspace Structure**: All Rust code follows workspace conventions with shared dependencies and build configurations. Uses Rust toolchain 1.89.0 with rustfmt edition 2024.

**Protobuf Interfaces**: Inter-service communication defined in `interface/proto/` with generated Rust bindings.

**Integration Testing**: Tests simulate multi-node P2P scenarios using temporary directories and spawn real daemon/enclave/vm processes.

**JavaScript Worker Development**: Workers are bundled with esbuild and executed in isolated Cloudflare Workerd VMs with restricted capabilities.

## Component Integration

The system architecture requires coordinated startup:
1. Build all binaries with `cargo build`
2. Start daemon process with P2P and HTTP configuration
3. Start enclave process connected via Unix socket
4. Start VM process and attach to daemon via gRPC
5. Workers are deployed as signed bundles to the P2P network

Use `./node/run.sh` for automated multi-component orchestration during development.
