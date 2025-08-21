---
title: Identities and Policies
description: Learn to create secure on-chain identities and governance policies for your workers
---

Ready to level up your workers with secure credential management? This tutorial covers nXCC's identity system - the foundation for production-ready applications.

## What You'll Learn

- Create on-chain identities as ERC-721 NFTs
- Write policies to govern credential access
- Link workers to identities for automatic secret injection
- Implement role-based access controls

## Why Identities Matter

Without identities, workers store secrets in plaintext configuration (like API keys in `userdata`). Identities provide:

- **Secure Storage**: Secrets encrypted and distributed across TEEs
- **Access Control**: Policies determine who can use secrets
- **Auditability**: All access logged on-chain
- **Transferability**: NFT-based ownership model

## Prerequisites

- Complete [Getting Started](./getting-started.md) and [Blockchain Events](./blockchain-events.md)
- Understanding of smart contracts and NFTs
- Local nXCC node running

## Setting Up the Environment

### 1. Start Local Blockchain

```bash
# Terminal 1: Start Anvil
anvil --chain-id 31337 --port 8545
```

### 2. Deploy Identity Contract

The Identity contract manages credential NFTs:

```bash
# Clone nXCC repo if you haven't already
git clone https://github.com/nxcc-bridge/nxcc.git

# Deploy the contract
cd nxcc/contracts/evm
forge install
forge create src/Identity.sol:Identity \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --rpc-url http://localhost:8545

# Save the contract address
export IDENTITY_CONTRACT=0x... # Use the deployed address
```

## Creating Your First Identity

### 1. Mint an Identity NFT

```bash
# Install nXCC CLI if you haven't
npm install -g @nxcc/cli

# Create identity
nxcc identity create 31337 $IDENTITY_CONTRACT \
  --signer 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --gateway-url http://localhost:8545

# Save the identity ID
export IDENTITY_ID=1 # Usually starts at 1
```

This mints an ERC-721 NFT that represents your identity. The NFT owner controls the identity.

### 2. Create a Policy

Policies are special workers that decide whether to grant access to an identity's secrets. Create `policy.ts`:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  console.log(`🔐 Policy evaluating ${requests.length} request(s)`);

  return requests.map((request) => {
    // Log the request for debugging
    console.log("Request details:", {
      workerHash: request.workerHash,
      nodeAttestation: request.nodeAttestation ? "present" : "missing",
      timestamp: new Date().toISOString(),
    });

    // Simple policy: approve all requests
    // In production, you'd check:
    // - Worker code hash whitelist
    // - Node attestation validity
    // - Time-based restrictions
    // - Operator identity

    console.log("✅ Request approved");
    return true;
  });
});
```

### 3. Deploy the Policy

Create `policy-manifest.json`:

```json
{
  "bundle": {
    "source": "./dist/policy.js"
  },
  "userdata": {
    "description": "Simple allow-all policy for development",
    "version": "1.0.0"
  },
  "identities": []
}
```

Build and set the policy:

```bash
# Build policy
npx tsc policy.ts --outDir dist --target es2022 --module es2022

# Bundle policy
nxcc bundle policy-manifest.json --out policy.bundle.json

# Set policy on identity
nxcc identity set-policy 31337 $IDENTITY_CONTRACT $IDENTITY_ID policy.bundle.json \
  --signer 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --gateway-url http://localhost:8545
```

## Using Identities in Workers

### 1. Create a Worker with Identity

Now create a worker that requests access to the identity:

```typescript
// secure-worker.ts
import { worker } from "@nxcc/sdk";
import {
  createWalletClient,
  http,
  privateKeyToAccount,
  formatEther,
} from "viem";
import { anvil } from "viem/chains";

export default worker({
  async fetch(request, { secrets, userdata }) {
    console.log("🔐 Worker started with secure credentials");

    // Access the private key injected by nXCC
    const privateKey = secrets.ETHEREUM_PRIVATE_KEY;
    if (!privateKey) {
      return new Response("No private key available", { status: 500 });
    }

    // Create wallet client
    const account = privateKeyToAccount(privateKey as `0x${string}`);
    const client = createWalletClient({
      account,
      chain: anvil,
      transport: http("http://anvil:8545"), // Docker network alias
    });

    // Get balance
    const balance = await client.getBalance({ address: account.address });
    const ethBalance = formatEther(balance);

    console.log(`💰 Account: ${account.address}`);
    console.log(`💰 Balance: ${ethBalance} ETH`);

    // Example: Send a transaction
    if (request.method === "POST") {
      const { to, amount } = await request.json();

      const hash = await client.sendTransaction({
        to: to as `0x${string}`,
        value: BigInt(amount),
      });

      return new Response(
        JSON.stringify({
          success: true,
          transaction: hash,
          from: account.address,
        }),
      );
    }

    return new Response(
      JSON.stringify({
        address: account.address,
        balance: ethBalance,
      }),
    );
  },
});
```

### 2. Worker Manifest with Identity Request

Create `secure-manifest.json`:

```json
{
  "bundle": {
    "source": "./dist/secure-worker.js"
  },
  "userdata": {
    "name": "secure-wallet-worker",
    "description": "Worker with access to secure credentials"
  },
  "identities": [
    {
      "chain": 31337,
      "contract": "YOUR_IDENTITY_CONTRACT_ADDRESS",
      "id": "YOUR_IDENTITY_ID",
      "secrets": {
        "ETHEREUM_PRIVATE_KEY": "secp256k1_private_key"
      }
    }
  ],
  "events": [
    {
      "handler": "fetch",
      "kind": "http",
      "path": "/secure"
    }
  ]
}
```

### 3. Deploy and Test

```bash
# Build worker
npx tsc secure-worker.ts --outDir dist --target es2022 --module es2022

# Deploy worker
nxcc worker deploy secure-manifest.json --rpc-url http://localhost:6922
```

Test the secure worker:

```bash
# Get wallet info
curl http://localhost:6922/secure

# Send a transaction (if implemented)
curl -X POST http://localhost:6922/secure \
  -H "Content-Type: application/json" \
  -d '{"to": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8", "amount": "1000000000000000000"}'
```

## Advanced Policy Patterns

### Time-Based Access

```typescript
export default policy((requests) => {
  return requests.map((request) => {
    const now = new Date();
    const hour = now.getHours();

    // Only allow access during business hours
    if (hour < 9 || hour > 17) {
      console.log("❌ Access denied: outside business hours");
      return false;
    }

    return true;
  });
});
```

### Worker Whitelist

```typescript
const allowedWorkers = new Set([
  "0x1234...", // Hash of approved worker code
  "0x5678...", // Another approved worker
]);

export default policy((requests) => {
  return requests.map((request) => {
    if (!allowedWorkers.has(request.workerHash)) {
      console.log("❌ Access denied: worker not whitelisted");
      return false;
    }

    return true;
  });
});
```

### Multi-Signature Policy

```typescript
const requiredSigners = 2;
const approvedSigners = new Set([
  "0xabc...", // Signer 1 address
  "0xdef...", // Signer 2 address
  "0x123...", // Signer 3 address
]);

export default policy((requests) => {
  return requests.map((request) => {
    // In a real implementation, you'd verify signatures
    // from multiple parties before approving access

    const signatures = request.multiSigApprovals || [];
    const validSignatures = signatures.filter((sig) =>
      approvedSigners.has(sig.signer),
    );

    if (validSignatures.length < requiredSigners) {
      console.log("❌ Access denied: insufficient signatures");
      return false;
    }

    return true;
  });
});
```

## Identity Management

### Transfer Ownership

Since identities are NFTs, you can transfer ownership:

```bash
# Transfer identity to another address
cast send $IDENTITY_CONTRACT \
  "transferFrom(address,address,uint256)" \
  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
  0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  $IDENTITY_ID \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --rpc-url http://localhost:8545
```

### Update Policy

Update the policy URL on an existing identity:

```bash
nxcc identity set-policy 31337 $IDENTITY_CONTRACT $IDENTITY_ID new-policy.bundle.json \
  --signer 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --gateway-url http://localhost:8545
```

### Revoke Access

Create a policy that denies all access:

```typescript
export default policy((requests) => {
  return requests.map(() => {
    console.log("❌ Identity revoked - all access denied");
    return false;
  });
});
```

## Production Considerations

### Hardware Attestation

In production, policies should verify TEE attestations:

```typescript
export default policy((requests) => {
  return requests.map((request) => {
    // Verify the node is running in a genuine TEE
    if (!request.nodeAttestation) {
      return false;
    }

    // Verify attestation signature and certificate chain
    const isValidTEE = verifyAttestation(request.nodeAttestation);
    if (!isValidTEE) {
      return false;
    }

    return true;
  });
});
```

### Governance Integration

Integrate with DAO governance:

```typescript
import { createPublicClient, http } from "viem";

const governanceContract = "0x...";

export default policy(async (requests) => {
  const client = createPublicClient({
    transport: http("https://eth.llamarpc.com"),
  });

  return Promise.all(
    requests.map(async (request) => {
      // Check if worker is approved by DAO vote
      const isApproved = await client.readContract({
        address: governanceContract,
        abi: [
          /* governance ABI */
        ],
        functionName: "isWorkerApproved",
        args: [request.workerHash],
      });

      return isApproved;
    }),
  );
});
```

## Next Steps

- **[Worker Runtime Reference](../reference/worker-runtime.md)** - Complete API docs
- **[CLI Reference](../reference/cli.md)** - All CLI commands

## Common Identity Patterns

### Service Account Pattern

```json
{
  "identities": [
    {
      "secrets": {
        "DATABASE_URL": "string",
        "API_KEY": "string",
        "SIGNING_KEY": "secp256k1_private_key"
      }
    }
  ]
}
```

### Multi-Chain Pattern

```json
{
  "identities": [
    {
      "chain": 1,
      "secrets": { "ETHEREUM_KEY": "secp256k1_private_key" }
    },
    {
      "chain": 137,
      "secrets": { "POLYGON_KEY": "secp256k1_private_key" }
    }
  ]
}
```

You now have the foundation for building secure, production-ready workers with proper credential management! 🔐
