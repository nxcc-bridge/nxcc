---
title: Building Event-Driven Workers
description: Learn to create workers that react to blockchain events across multiple chains
---

Now that you've created your first worker, let's build something more powerful: workers that automatically respond to blockchain events.

## What You'll Build

A worker that:

- Listens for token transfers on Ethereum
- Automatically logs the transaction details
- Can trigger actions on other blockchains

## Prerequisites

- Complete the [Getting Started](/docs/guides/getting-started/) guide
- nXCC node running locally (`./run.sh`)
- Basic understanding of blockchain events

## Setting Up Event Listening

### 1. Create an Event Worker

Create a new project for your event-driven worker:

```bash
mkdir event-worker && cd event-worker
nxcc init .
npm install viem
```

### 2. Write the Event Handler

Edit `workers/my-worker.ts`:

```typescript
import { worker } from "@nxcc/sdk";
import { decodeEventLog, parseAbiItem, formatEther, Hex } from "viem";

const transferAbi = parseAbiItem(
  "event Transfer(address indexed from, address indexed to, uint256 value)",
);

export default worker({
  async fetch(event, { userdata }) {
    // Decode the Transfer event
    const decoded = decodeEventLog({
      abi: [transferAbi],
      data: event.data as Hex,
      topics: event.topics as [Hex, ...Hex[]],
    });

    const { from, to, value } = decoded.args;
    const ethAmount = formatEther(value);

    console.log(`🎯 Transfer detected!`);
    console.log(`  From: ${from}`);
    console.log(`  To: ${to}`);
    console.log(`  Amount: ${ethAmount} ETH`);
    console.log(`  Block: ${event.blockNumber}`);
    console.log(`  Tx: ${event.transactionHash}`);

    // Here you could:
    // - Send notifications
    // - Trigger transactions on other chains
    // - Update databases
    // - Call external APIs

    return new Response(
      JSON.stringify({
        success: true,
        transfer: { from, to, amount: ethAmount },
        processedAt: new Date().toISOString(),
      }),
    );
  },
});
```

### 3. Configure Event Subscription

Create `workers/event-manifest.json`:

```json
{
  "bundle": {
    "source": "./dist/my-worker.js"
  },
  "userdata": {
    "name": "ethereum-transfer-monitor",
    "description": "Monitors ETH transfers and logs details"
  },
  "events": [
    {
      "handler": "fetch",
      "kind": "web3_event",
      "chain": 1,
      "address": ["0xA0b86a33E6441B8066D18CE51C39194AEacAFB9B"],
      "topics": [
        ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
      ]
    }
  ]
}
```

> **Note**: This example listens to transfers from a specific contract. Use `"address": []` to listen to all Transfer events on Ethereum.

### 4. Build and Deploy

```bash
npm run build
nxcc worker deploy workers/event-manifest.json --rpc-url http://localhost:6922
```

## Testing with Local Blockchain

For testing, let's use a local blockchain instead of mainnet:

### 1. Start Anvil

```bash
# In a new terminal
anvil --chain-id 31337 --port 8545
```

### 2. Deploy a Test Token

```bash
# Install foundry if needed
curl -L https://foundry.paradigm.xyz | bash && foundryup

# Create a simple ERC20 token
cat > TestToken.sol << 'EOF'
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract TestToken {
    mapping(address => uint256) public balanceOf;

    event Transfer(address indexed from, address indexed to, uint256 value);

    constructor() {
        balanceOf[msg.sender] = 1000000 * 10**18;
    }

    function transfer(address to, uint256 amount) public {
        require(balanceOf[msg.sender] >= amount, "Insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
    }
}
EOF

# Deploy it
forge create TestToken --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --rpc-url http://localhost:8545
```

### 3. Update Worker Configuration

Update your manifest to listen to the local chain:

```json
{
  "bundle": {
    "source": "./dist/my-worker.js"
  },
  "userdata": {
    "name": "local-transfer-monitor"
  },
  "events": [
    {
      "handler": "fetch",
      "kind": "web3_event",
      "chain": 31337,
      "address": ["YOUR_TOKEN_CONTRACT_ADDRESS"],
      "topics": [
        ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
      ]
    }
  ]
}
```

### 4. Trigger Events

```bash
# Make a transfer to trigger your worker
cast send YOUR_TOKEN_CONTRACT_ADDRESS \
  "transfer(address,uint256)" \
  0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  1000000000000000000 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --rpc-url http://localhost:8545
```

Watch your nXCC node logs to see the worker process the transfer!

## Multi-Chain Event Handling

One of nXCC's superpowers is handling events from multiple blockchains in a single worker:

```json
{
  "events": [
    {
      "handler": "fetch",
      "kind": "web3_event",
      "chain": 1,
      "address": ["0x..."],
      "topics": [["0x..."]]
    },
    {
      "handler": "fetch",
      "kind": "web3_event",
      "chain": 137,
      "address": ["0x..."],
      "topics": [["0x..."]]
    },
    {
      "handler": "fetch",
      "kind": "web3_event",
      "chain": 42161,
      "address": ["0x..."],
      "topics": [["0x..."]]
    }
  ]
}
```

Your worker will receive events from Ethereum, Polygon, and Arbitrum in the same `fetch` function!

## Advanced Event Patterns

### Filter by Event Parameters

Listen only to large transfers:

```typescript
// In your worker
const { value } = decoded.args;
const ethAmount = formatEther(value);

// Only process transfers > 10 ETH
if (parseFloat(ethAmount) < 10) {
  return new Response("Transfer too small, ignoring");
}

console.log(`🚨 Large transfer detected: ${ethAmount} ETH`);
```

### Cross-Chain Reactions

React to an event on one chain by triggering an action on another:

```typescript
import { createWalletClient, http, privateKeyToAccount } from "viem";
import { polygon } from "viem/chains";

export default worker({
  async fetch(event, { userdata }) {
    // Event happened on Ethereum (chain 1)
    console.log("Ethereum event detected, triggering Polygon transaction...");

    // Create Polygon client
    const account = privateKeyToAccount(userdata.privateKey as Hex);
    const polygonClient = createWalletClient({
      account,
      chain: polygon,
      transport: http(userdata.polygonRpcUrl),
    });

    // Send transaction on Polygon
    const hash = await polygonClient.sendTransaction({
      to: userdata.targetAddress as Hex,
      value: parseEther("0.1"),
    });

    return new Response(
      JSON.stringify({
        ethereumEvent: event.transactionHash,
        polygonTx: hash,
      }),
    );
  },
});
```

## Event Sources

nXCC supports various event sources:

- **Blockchain Events**: EVM event logs from 400+ chains
- **HTTP Webhooks**: External API callbacks
- **Timers**: Scheduled execution
- **P2P Messages**: Direct worker-to-worker communication

Example manifest with multiple event types:

```json
{
  "events": [
    {
      "handler": "handleBlockchainEvent",
      "kind": "web3_event",
      "chain": 1,
      "topics": [["0x..."]]
    },
    {
      "handler": "handleWebhook",
      "kind": "http",
      "path": "/webhook"
    },
    {
      "handler": "handleScheduled",
      "kind": "timer",
      "schedule": "0 */10 * * * *"
    }
  ]
}
```

## Next Steps

- **[Identities and Policies](/docs/guides/identities-policies/)** - Learn secure credential management
- **[Worker Runtime Reference](/docs/reference/worker-runtime/)** - Complete API documentation

## Common Patterns

### Event Aggregation

```typescript
// Track events in memory or external storage
const eventStore = new Map();

export default worker({
  async fetch(event) {
    const key = `${event.address}-${event.blockNumber}`;
    eventStore.set(key, event);

    // Process when we have enough events
    if (eventStore.size >= 10) {
      await processBatch(Array.from(eventStore.values()));
      eventStore.clear();
    }
  },
});
```

### Retry Logic

```typescript
async function withRetry(fn: () => Promise<any>, maxRetries = 3) {
  for (let i = 0; i < maxRetries; i++) {
    try {
      return await fn();
    } catch (error) {
      if (i === maxRetries - 1) throw error;
      await new Promise((resolve) => setTimeout(resolve, 1000 * (i + 1)));
    }
  }
}
```

You're now ready to build sophisticated event-driven applications that span multiple blockchains! 🚀
