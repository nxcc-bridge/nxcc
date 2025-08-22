---
title: Performance & Efficiency
description: Why nXCC's "almost consensus-free" architecture delivers superior performance and efficiency.
---

nXCC represents a fundamental shift in cross-chain infrastructure design. While traditional bridges rely on large validator sets and consensus mechanisms that waste energy and add latency, nXCC's "almost consensus-free" architecture using Trusted Execution Environments (TEEs) delivers unmatched performance and efficiency.

## Benchmark Methodology

Our performance benchmarks are conducted on standardized, cost-effective hardware to reflect realistic production deployments.

- **Hardware**: A standard container with 2 CPU cores and 2GB of RAM.
- **Setup**: To isolate the performance of the nXCC node itself, the Web3 gateway (Anvil) is co-resident with the node, eliminating external blockchain network latency as a variable.

## Key Performance Metrics

The results demonstrate that nXCC introduces minimal overhead, delivering performance suitable for demanding, real-time applications.

| Metric Category        | Metric                                 | Value               |
| :--------------------- | :------------------------------------- | :------------------ |
| **Resource Usage**     | Base Usage (Idle Node)                 | ~0% CPU, 6 MB RAM   |
|                        | Idle Worker Footprint (139 workers)    | 79 MB RAM           |
| **Web3 Event Latency** | Mean Latency                           | 14.39 ms            |
|                        | p99 Latency                            | 17 ms               |
| **Throughput**         | Web3 Event Throughput                  | 1,130.98 events/sec |
|                        | HTTP Event Throughput (100 req/s load) | 3,224.45 req/sec    |
| **Worker Capacity**    | Polling Workers (10ms interval)        | 215 workers         |

## Why nXCC Outperforms Traditional Bridges

### The Consensus Problem

Traditional cross-chain bridges face a fundamental tradeoff:

- **Security requires many validators** (50+ nodes to prevent 33% attacks)
- **Many validators require consensus** (Byzantine fault tolerance protocols)
- **Consensus adds latency and energy waste** (multiple rounds of voting)

### The nXCC Solution: TEE-First Architecture

nXCC eliminates this tradeoff using Intel TDX (Trust Domain Extensions):

1. **Hardware-guaranteed security** replaces consensus-based security
2. **Small node count** (3-5 nodes) achieves same security as 100+ validator networks
3. **No consensus overhead** for most operations
4. **Direct execution** in hardware-protected environments

### Performance Advantages

| Metric                 | Traditional Bridges    | nXCC Advantage                |
| ---------------------- | ---------------------- | ----------------------------- |
| **Latency**            | 30-120 seconds         | **Sub-second to ~14ms**       |
| **Throughput**         | 10-100 messages/sec    | **1,000+ events/sec**         |
| **Energy per message** | High (many validators) | **Orders of magnitude lower** |
| **Node requirements**  | 50-150 validators      | **3-5 TEE nodes**             |

## Energy Efficiency: A Green Alternative

nXCC's architecture delivers dramatic energy savings compared to traditional validator-based bridges:

### Comparative Energy Efficiency

| Network        | Relative Energy per Message | Architecture                          |
| :------------- | :-------------------------- | :------------------------------------ |
| **nXCC**       | **1x (baseline)**           | TEE-secured, minimal consensus        |
| Chainlink CCIP | ~4.5x more energy           | Large validator set, consensus        |
| Axelar         | ~4.8x more energy           | 150+ validators, Tendermint consensus |
| Wormhole       | ~9.8x more energy           | 19+ guardians, gossip protocol        |

### Why TEEs Are More Efficient

**Traditional Bridge Energy Usage:**

- 50-150 validators running 24/7
- Consensus rounds for every message
- Redundant computation across all validators
- Network overhead for voting protocols

**nXCC Energy Usage:**

- 3-5 specialized nodes with TEEs
- Hardware attestation replaces consensus
- Direct execution without redundant computation
- Minimal network coordination

**Result: >90% energy reduction** for equivalent security guarantees.

## Real-World Performance Impact

### For Developers

- **Faster iteration**: Sub-second feedback for testing
- **Better UX**: Near-instant cross-chain operations
- **Lower costs**: Fewer resources needed for same throughput

### For Users

- **Responsive applications**: No waiting minutes for bridge confirmations
- **Lower fees**: Efficiency savings passed on as reduced costs
- **Reliable performance**: Consistent latency regardless of network congestion

### For Operators

- **Lower infrastructure costs**: Fewer nodes, less hardware
- **Reduced energy bills**: Minimal consensus overhead
- **Simpler operations**: TEE attestation vs. complex validator coordination

## The Future is TEE-Native

nXCC proves that cross-chain infrastructure doesn't need to choose between security, performance, and sustainability. Hardware-based trust domains offer a path to:

- **Instant finality** without sacrificing security
- **Linear scalability** without consensus bottlenecks
- **Green computing** without environmental compromise
- **Cost efficiency** without operational complexity

While other protocols add more validators to increase security (and decrease efficiency), nXCC leverages hardware advances to achieve both superior security and performance with minimal environmental impact.

---

_Ready to experience the performance difference? [Get started](../guides/getting-started.md) with nXCC today._
