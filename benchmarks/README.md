# nXCC Benchmarks

Performance testing suite for nXCC platform components.

## Benchmark Results

| Metric                          | Value                  |
| ------------------------------- | ---------------------- |
| **Event Latency (p99)**         | 17ms                   |
| **Event Latency (Mean)**        | 14.39ms                |
| **Web3 Event Throughput**       | 1,130+ events/sec      |
| **HTTP Throughput**             | 3,224+ req/sec         |
| **Worker Capacity (Realistic)** | 130 concurrent workers |
| **Base Memory Usage**           | 6MB RAM                |

## Running Benchmarks

### Prerequisites

- Docker and Docker Compose
- Rust toolchain
- Node.js and pnpm (for JavaScript workers)

### Quick Start

```bash
# Run full benchmark suite
./run.sh

# Custom resource limits
./run.sh --cpus 4 --memory 4g
```

### Manual Setup

```bash
# Build components
cargo build --release
cd workers && pnpm install && pnpm run build

# Run specific benchmark
./target/release/benchmarks web3-latency
./target/release/benchmarks http-throughput
./target/release/benchmarks realistic
```

## Benchmark Types

- **`idle`**: Base resource usage measurements
- **`cpu`**: CPU-bound active worker capacity testing
- **`io`**: I/O-bound active worker capacity testing
- **`realistic`**: Mixed CPU + I/O workload with realistic duty cycles
- **`polling --interval-ms <ms>`**: Worker polling performance at specified intervals
- **`web3-latency`**: On-chain event processing latency measurement
- **`web3-throughput`**: Maximum Web3 event processing rate
- **`http-throughput`**: HTTP request handling capacity for single worker

## Architecture

- **Rust CLI** (`src/main.rs`): Benchmark orchestration and metrics collection
- **JavaScript Workers** (`workers/`): Test workloads for performance measurement
- **Docker Integration**: Containerized nXCC node for consistent testing
- **Anvil Integration**: Local Ethereum testnet for Web3 benchmarks

## Environment Setup

The benchmark harness automatically:

- Builds optimized nXCC node Docker image
- Deploys local Ethereum testnet (Anvil)
- Configures isolated Docker network
- Manages container lifecycle and cleanup

## Development

### Adding New Benchmarks

1. Add benchmark variant to `src/main.rs`
2. Implement measurement logic
3. Create corresponding JavaScript worker if needed
4. Update benchmark runner script

### Performance Notes

- **Memory limits**: Workers become memory-bound before CPU-bound
- **Network isolation**: Uses dedicated Docker network for consistent results
- **Container restart**: Fresh node state between benchmark runs
- **Resource constraints**: Configure appropriate CPU/memory limits

### Metrics Collection

Uses `hdrhistogram` for accurate latency percentiles and `indicatif` for progress tracking. Results are formatted for both human consumption and CI integration.
