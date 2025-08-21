# nXCC

<div align="center">

**The Programmable Glue for the Multi-Chain World**

_Secure, off-chain orchestration engine connecting any blockchain, API, or system_

[🚀 Quick Start](#-quick-start) • [💡 Use Cases](#-use-cases) • [📖 Documentation](https://nxcc.org/docs/guides/getting-started/)

</div>

---

## 🌐 Overview

nXCC provides the essential infrastructure to bind disparate systems into cohesive applications through:

- **🔐 Confidential Computing**: JavaScript workers execute in hardware-protected Trusted Execution Environments (TEEs), ensuring code and data remain encrypted during execution
- **⛓️ Multi-Chain Native**: Built-in support for [400+ EVM-compatible networks](node/chainlist/) with extensible architecture for non-EVM chains
- **⚡ Event-Driven Architecture**: React to on-chain events, HTTP webhooks, and P2P messages with sub-20ms latency
- **🤝 Federated Trust Model**: Run your own nodes or form consortiums with cryptographic identity verification and on-chain governance

## ⚡ Performance

<div align="center">

| Metric                  | Value                   |
| ----------------------- | ----------------------- |
| **Event Latency (p99)** | < 17ms                  |
| **Event Throughput**    | 1,130+ events/sec       |
| **HTTP Throughput**     | 3,224+ req/sec          |
| **Worker Capacity**     | 130+ concurrent workers |

_Source: [Benchmark Results](benchmarks/README.md)_

</div>

## 🏗️ Architecture

nXCC uses a multi-process architecture where JavaScript workers execute inside trusted execution environments (TEEs) for confidential computation:

### Core Components

- **[Daemon](node/daemon/)**: P2P networking hub managing libp2p communications, work distribution, and blockchain connections
- **[Enclave](node/enclave/)**: TEE-based execution environment providing verifiable computation with Intel TDX/SGX support
- **[VM](node/vm/)**: Cloudflare Workerd runtime for isolated JavaScript worker execution
- **[Interface](node/interface/)**: gRPC/protobuf communication layer enabling secure inter-process communication

## 🚀 Quick Start

### Using Docker (Recommended)

```bash
# Pull the latest image
docker pull ghcr.io/nxcc-bridge/nxcc/node:latest

# Run a local node
docker run -p 3000:3000 -p 9000:9000 ghcr.io/nxcc-bridge/nxcc/node:latest

# Deploy your first worker
curl -X POST http://localhost:3000/workers \
  -H "Content-Type: application/json" \
  -d '{"code": "export default { async handle(request) { return new Response(\"Hello nXCC!\"); } }"}'
```

### Building from Source

```bash
# Clone and build
git clone https://github.com/nxcc-bridge/nxcc.git
cd nxcc/node && cargo build --release

# Start local node
./run.sh
```

**📚 [View complete getting started guide →](docs/src/content/docs/docs/guides/getting-started.md)**

## 💡 Use Cases

### 🤖 Secure AI Agents

Enable autonomous agents to interact with multiple blockchains while protecting valuable models and decision logic within confidential computing environments.

### 🏦 Cross-Chain DeFi

Build sophisticated trading strategies that aggregate liquidity across networks while keeping proprietary algorithms confidential.

**📖 [Explore more examples →](e2e/)**

## 🛠️ Key Capabilities

**Cross-Chain Orchestration**: Define complex workflows that span multiple blockchains, automatically handling event subscription, transaction execution, and state synchronization.

**Secret Management**: Built-in distributed secret sharing using peer-to-peer TEE-to-TEE protocols, governed by on-chain policies for API keys and sensitive data.

**Hardware-Enforced Privacy**: Intel TDX/SGX trusted execution environments provide memory encryption and remote attestation for verifiable computation.

## 📁 Project Structure

```
nxcc/
├── 🗂️ node/                    # Core Rust workspace
│   ├── 📦 daemon/              # P2P networking & orchestration
│   ├── 🔒 enclave/             # TEE execution environment
│   ├── ⚙️ vm/                  # JavaScript runtime (Workerd)
│   ├── 📡 interface/           # gRPC/protobuf definitions
│   ├── ⛓️ chainlist/           # 400+ blockchain configurations
│   ├── 🔐 attestation/         # TDX/SGX attestation logic
│   └── 🧪 tests/               # Integration test suite
├── 📑 contracts/               # Smart contracts (Solidity)
├── 🏗️ infra/                   # Kubernetes & deployment tools
├── 🚀 benchmarks/              # Performance testing suite
├── 📖 docs/                    # Documentation site
├── 🔍 e2e/                     # End-to-end test scenarios
└── 🛠️ sdk/                     # Client SDKs & tooling
```

## 🧪 Development & Testing

### Prerequisites

- Rust 1.89.0+ with edition 2024
- Node.js 18+ (for JavaScript workers)
- Docker (for containerized deployment)
- Intel TDX/SGX hardware (for production attestation)

### Testing

```bash
cd node

# Unit tests
cargo test

# Integration tests with multi-node P2P scenarios
cargo build && sh tests/integration_test.sh

# End-to-end tests with full workflow execution
cd ../e2e && ./e2e_test.sh

# Performance benchmarks
cd ../benchmarks && ./run.sh
```

**📋 [View testing guide →](node/tests/README.md)**

## 📚 Documentation

- **[📖 Getting Started Guide](docs/src/content/docs/docs/guides/getting-started.md)** - Getting started with nXCC development
- **[🏗️ Core Concepts](docs/src/content/docs/docs/reference/core-concepts.md)** - System design and component interaction
- **[🔧 CLI Reference](docs/src/content/docs/docs/reference/cli.md)** - Command-line interface documentation
- **[🚀 Running a Node](docs/src/content/docs/docs/reference/running-a-node.md)** - Node deployment instructions
- **[🧪 Testing Guide](node/tests/README.md)** - Testing strategies and best practices
- **[⚡ Performance Analysis](benchmarks/README.md)** - Benchmark results and optimization tips

## 🤝 Contributing

nXCC is open source and welcomes contributions! We follow modern development practices:

- **Rust Edition 2024** with comprehensive testing
- **Component-based architecture** with clear separation of concerns
- **Docker-first deployment** for consistent environments
- **Automated CI/CD** with quality gates

**📋 Contributing guidelines coming soon**

## 🏛️ Funding & Acknowledgments

<div align="center">

<img src="https://trustchain.ngi.eu/wp-content/uploads/2023/01/NGI-trustchain.png" width="400" alt="NGI TrustChain">

This project is supported by the [**Next Generation Internet (NGI) TrustChain**](https://www.ngi.eu/ngi-projects/ngi-trustchain/) initiative, part of the European Union's commitment to building a more open, trustworthy, and human-centric internet with a focus on digital sovereignty and privacy by design.

</div>

## 📄 License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

---

<div align="center">

**🌟 Ready to connect your multi-chain infrastructure? 🌟**

[📖 Get Started](docs/) • [🐙 GitHub](https://github.com/nxcc-bridge/nxcc)

_Made with ❤️ by the nXCC community_

</div>
