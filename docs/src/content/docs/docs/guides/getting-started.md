---
title: Getting Started with nXCC
description: Build your first cross-chain worker in 10 minutes
---

Welcome to nXCC! This guide will get you up and running with your first cross-chain worker in just a few steps.

## Quick Start

The fastest way to try nXCC is using our all-in-one local setup:

```bash
# Clone the repo and start everything
git clone https://github.com/nxcc-bridge/nxcc.git
cd nxcc/node
./run.sh
```

That's it! You now have a local nXCC node running at `http://localhost:6922`.

## Your First Worker

Create a simple HTTP worker that runs in a secure environment:

```bash
# Create a new project
mkdir my-nxcc-app && cd my-nxcc-app

# Create a simple worker
cat > worker.js << 'EOF'
export default {
  async fetch(request) {
    return new Response("Hello from nXCC! 🚀");
  }
}
EOF

# Deploy it
curl -X POST http://localhost:6922/workers \
  -H "Content-Type: application/json" \
  -d "$(cat worker.js | jq -Rs '{"code": .}')"
```

Your worker is now running! Test it:

```bash
curl http://localhost:6922/workers/latest
```

## What Just Happened?

1. **Secure Execution**: Your JavaScript code runs inside a Trusted Execution Environment (TEE)
2. **No Infrastructure**: No need to manage servers, containers, or VMs
3. **Cross-Chain Ready**: Your worker can listen to events from 400+ blockchains

## Next Steps

### Build an Event-Driven Worker

Create a worker that reacts to blockchain events:

```bash
# Install the CLI for advanced features
npm install -g @nxcc/cli

# Initialize a new project with TypeScript
nxcc init my-event-worker
cd my-event-worker && npm install
```

Edit `workers/my-worker.ts`:

```typescript
import { worker } from "@nxcc/sdk";

export default worker({
  async fetch(event, { userdata }) {
    console.log("Received blockchain event:", event);

    // Your cross-chain logic here
    // - Call APIs
    // - Send transactions to other chains
    // - Access secrets securely

    return new Response("Event processed!");
  },
});
```

Build and deploy:

```bash
npm run build
nxcc worker deploy workers/manifest.json --rpc-url http://localhost:6922
```

### Connect to Real Blockchains

To listen to mainnet events, just update your manifest:

```json
{
  "events": [
    {
      "chain": 1,
      "address": ["0x..."],
      "topics": [["0x..."]]
    }
  ]
}
```

### Use Docker for Production

For production deployments:

```bash
docker run -p 6922:6922 ghcr.io/nxcc-bridge/nxcc/node:latest
```

## Core Concepts

**Workers**: JavaScript/TypeScript code that runs in secure environments and can:

- React to blockchain events across 400+ chains
- Handle HTTP requests
- Access secrets and make API calls
- Send transactions

**Secure by Default**: All code runs in Trusted Execution Environments (TEEs) with memory encryption and remote attestation.

**Multi-Chain Native**: Built-in support for Ethereum, Polygon, Arbitrum, and 400+ other EVM chains.

## What's Next?

Follow our progressive tutorial series:

1. **[Blockchain Events](./blockchain-events.md)** ← Start here next
   Build workers that react to on-chain events across multiple blockchains

2. **[Identities & Policies](./identities-policies.md)**  
   Add secure credential management and access controls

## Reference Docs

- **[Core Concepts](../reference/core-concepts.md)** - System architecture deep dive
- **[Worker Runtime](../reference/worker-runtime.md)** - Complete JavaScript API reference
- **[CLI Reference](../reference/cli.md)** - All CLI commands and options

## Need Help?

- 📖 [Full Documentation](../reference/)
- 🐛 [GitHub Issues](https://github.com/nxcc-bridge/nxcc/issues)
- 💬 [Community Discord](https://discord.gg/nxcc)

Ready to build the future of cross-chain applications? Let's go! 🚀
