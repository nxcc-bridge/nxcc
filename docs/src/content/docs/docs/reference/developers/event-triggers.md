---
title: Event Triggers
description: Learn how to trigger nXCC workers with on-chain events, HTTP requests, and more.
---

nXCC workers are event-driven. You define what triggers your worker in the `events` array of your **work order** when deploying with `nxcc worker deploy`. When a configured event occurs, the nXCC node invokes your worker by calling the specified handler function.

## The `events` Array

The `events` array in your work order contains a list of event trigger objects. Each object specifies the event `kind`, the `handler` function in your worker to call, and any parameters specific to that event kind.

```json
// Work order structure (created by nxcc worker deploy)
{
  "id": "worker-id",
  "worker": { ... }, // Your manifest
  "events": [
    {
      "handler": "launch",
      "kind": "launch"
    },
    {
      "handler": "fetch",
      "kind": "http_request"
    },
    {
      "handler": "tick",
      "kind": "scheduled",
      "period_ms": 60000
    }
  ]
}
```

### `launch`

The `launch` event triggers your worker exactly once, immediately after it has been successfully deployed and started by the nXCC node. This is useful for initialization tasks or for starting long-running processes like a polling loop.

**Manifest Configuration:**

```json
{
  "handler": "fetch",
  "kind": "launch"
}
```

When triggered, the `fetch` handler will receive a request with a JSON body identifying the event.

### `http_request`

The `http_request` event exposes your worker as a public HTTP endpoint. This allows you to trigger your worker from webhooks, user interfaces, or any other HTTP client.

**Manifest Configuration:**

```json
{
  "handler": "fetch",
  "kind": "http_request"
}
```

**Endpoint URL:**

The worker becomes available at a unique URL based on the hash of its work order:
`http://<node-url>/w/<work-order-hash>`

- `<node-url>`: The address of the nXCC node (e.g., `localhost:6922`).
- `<work-order-hash>`: The SHA-256 hash (base64url encoded) of the DSSE envelope used to deploy the worker. The `nxcc worker deploy` command returns this hash.

When a request hits this endpoint, the nXCC node forwards the full HTTP `Request` object to your worker's `fetch` handler.

### `web3_event`

The `web3_event` trigger is the core of nXCC's cross-chain capabilities. It instructs the node to listen for a specific smart contract event on an EVM-compatible blockchain and invoke your worker when that event is emitted.

**Manifest Configuration:**

```json
{
  "handler": "fetch",
  "kind": "web3_event",
  "chain": 31337,
  "address": ["0x5fbdb2315678afecb367f032d93f642f64180aa3"],
  "topics": [
    ["0x35c2b3b04a37f2752491485a4b51c863265557ac8152345842775344ba3a017b"]
  ]
}
```

- `chain`: The ID of the target blockchain.
- `address`: An array of contract addresses to monitor.
- `topics`: An array of topic filters, following the [JSON-RPC specification](https://docs.alchemy.com/reference/eth_getlogs#topics-array). The first topic is typically the signature hash of the event. You can generate this using tools like `cast` or `viem`.

When the specified event is detected, the `fetch` handler receives a request with a JSON body containing the full `Web3Log` object, including topics and data. Your worker can then decode this payload to perform its logic.

### `scheduled`

The `scheduled` event trigger enables your worker to run at regular intervals based on a configurable schedule. This is useful for periodic tasks like data aggregation, health checks, or time-based automation.

**Manifest Configuration:**

```json
{
  "handler": "fetch",
  "kind": "scheduled",
  "period_ms": 60000
}
```

**Basic Parameters:**

- `period_ms`: The interval between scheduled executions in milliseconds. The daemon enforces a minimum interval (default: 10ms) to prevent excessive resource usage.

**Advanced Configuration:**

For more control over scheduling behavior, you can specify additional parameters:

```json
{
  "handler": "fetch",
  "kind": "scheduled",
  "period_ms": 30000,
  "phase_ms": 5000,
  "start_at": "2024-01-01T12:00:00Z",
  "end_at": "2024-12-31T23:59:59Z",
  "max_occurrences": 100,
  "policy": {
    "catch_up": "skip",
    "max_lateness_ms": 5000,
    "jitter_budget_ms": 100
  }
}
```

**Advanced Parameters:**

- `phase_ms`: Offset from the start boundary in milliseconds (default: 0).
- `start_at`: ISO 8601 timestamp when scheduling should begin (default: immediate).
- `end_at`: ISO 8601 timestamp when scheduling should stop (default: never).
- `max_occurrences`: Maximum number of times the event should fire (default: unlimited).
- `policy`: Optional scheduling policy configuration:
  - `catch_up`: What to do with late/missed ticks. Options: `"skip"` (default), `"coalesce"`, `"queue"`.
  - `max_lateness_ms`: Drop ticks that are later than this many milliseconds.
  - `jitter_budget_ms`: Used for monitoring/SLA purposes only.

**Example Use Cases:**

```json
// Health check every 5 minutes
{
  "handler": "fetch",
  "kind": "scheduled",
  "period_ms": 300000
}

// Daily report at midnight UTC
{
  "handler": "fetch",
  "kind": "scheduled",
  "period_ms": 86400000,
  "start_at": "2024-01-01T00:00:00Z"
}

// Limited-time campaign with 100 executions max
{
  "handler": "fetch",
  "kind": "scheduled",
  "period_ms": 3600000,
  "max_occurrences": 100
}
```

When a scheduled event fires, your worker's `fetch` handler receives a request with a JSON body identifying the scheduled event. The worker can maintain state across invocations to track execution count or implement more complex logic.
