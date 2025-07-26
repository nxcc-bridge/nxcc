---
title: Event Triggers
description: Learn how to trigger nXCC workers with on-chain events, HTTP requests, and more.
---

nXCC workers are event-driven. You define what triggers your worker in the `events` array of your `worker.manifest.json` file. When a configured event occurs, the nXCC node invokes your worker by sending a `POST` request to its `fetch` handler.

## The `events` Array

The `events` array in your manifest contains a list of event trigger objects. Each object specifies the event `kind`, the `handler` function in your worker to call (which is typically just `"fetch"`), and any parameters specific to that event kind.

```json
// worker.manifest.json
{
  // ...
  "events": [
    {
      "handler": "fetch",
      "kind": "launch"
    },
    {
      "handler": "fetch",
      "kind": "http_request"
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
