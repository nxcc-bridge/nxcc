---
title: Performance & Efficiency
description: Learn about nXCC's performance benchmarks and resource efficiency.
---

nXCC is engineered from the ground up for high performance, low latency, and exceptional resource efficiency. This focus ensures that the platform is not the bottleneck in your cross-chain workflows and provides a cost-effective and environmentally friendly solution.

## Benchmark Methodology

Our performance benchmarks are conducted on standardized, cost-effective hardware to reflect realistic production deployments.

-   **Hardware**: A standard container with 2 CPU cores and 2GB of RAM.
-   **Setup**: To isolate the performance of the nXCC node itself, the Web3 gateway (Anvil) is co-resident with the node, eliminating external blockchain network latency as a variable.

## Key Performance Metrics

The results demonstrate that nXCC introduces minimal overhead, delivering performance suitable for demanding, real-time applications.

| Metric Category      | Metric                                 | Value                  |
| :------------------- | :------------------------------------- | :--------------------- |
| **Resource Usage**   | Base Usage (Idle Node)                 | ~0% CPU, 6 MB RAM      |
|                      | Idle Worker Footprint (139 workers)    | 79 MB RAM              |
| **Web3 Event Latency** | Mean Latency                           | 14.39 ms               |
|                      | p99 Latency                            | 17 ms                  |
| **Throughput**       | Web3 Event Throughput                  | 1,130.98 events/sec    |
|                      | HTTP Event Throughput (100 req/s load) | 3,224.45 req/sec       |
| **Worker Capacity**  | Polling Workers (10ms interval)        | 215 workers            |

## A Greener Alternative for Web3

A core design goal of nXCC is to provide a high-performance, low-energy alternative to existing cross-chain solutions, aligning with the goals of the EU Green Deal.

Our "almost consensus-free" architecture, which relies on a small number of TEE-secured nodes rather than a large network of validators, is orders of magnitude more energy-efficient per message than traditional interoperability protocols.

### Comparative Energy Efficiency

The following table compares the estimated energy cost per message for nXCC against major incumbent networks.

| Network          | Relative Energy per Message |
| :--------------- | :-------------------------- |
| **nXCC (Ours)**  | **1x**                      |
| Chainlink CCIP   | ~4.5x more                  |
| Axelar           | ~4.8x more                  |
| Wormhole         | ~9.8x more                  |

*This analysis is based on our internal benchmarks and publicly available data on network size and throughput for incumbent protocols, assuming standard server hardware power consumption.*
