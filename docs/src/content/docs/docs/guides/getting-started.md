---
title: Getting Started with nXCC
description: A step-by-step guide to building your first cross-chain application with nXCC.
---

Welcome to nXCC! This guide will walk you through building a complete, event-driven, cross-chain application. You'll learn the core concepts of nXCC by creating a secure off-chain "worker" that listens for an event on a blockchain and triggers a transaction in response.

By the end of this tutorial, you will have:

- Set up a local development environment with a blockchain and an nXCC node.
- Understood the core concepts: Identities, Policies, and Workers.
- Deployed smart contracts to a local blockchain.
- Created an on-chain identity for your worker.
- Authored and set a security policy for your identity.
- Written, deployed, and triggered a cross-chain worker.

Let's get started!

## Prerequisites

Before you begin, make sure you have the following tools installed:

- **Node.js**: Version 18 or higher. [Install Node.js](https://nodejs.org/)
- **Docker**: To run the nXCC node and a local blockchain. [Install Docker](https://www.docker.com/products/docker-desktop/)
- **Foundry**: A smart contract development toolchain. We'll use `anvil` for a local blockchain and `forge` and `cast` to interact with it.
  ```bash
  curl -L https://foundry.paradigm.xyz | bash
  foundryup
  ```
- **nXCC CLI**: The command-line interface for interacting with nXCC.
  ```bash
  npm install -g @nxcc/cli
  ```

## 1. Core Concepts

Let's quickly cover the three main concepts in nXCC:

- **Identities**: An Identity is an on-chain asset, represented as an ERC-721 NFT, that acts as a secure anchor for an off-chain process. It's the root of trust for your worker.
- **Policies**: A Policy is a special type of worker that governs an Identity. It's a programmable gatekeeper that runs inside a secure enclave and decides if the Identity's secrets can be accessed or shared.
- **Workers**: A Worker is your off-chain logic, written in JavaScript or TypeScript. It runs inside a secure, serverless environment (a Trusted Execution Environment or TEE) and can react to on-chain events, HTTP requests, or timers to perform actions like calling APIs or submitting transactions.

The security model relies on these components: the on-chain Identity NFT points to a Policy URL. The nXCC node fetches and runs this Policy inside a TEE to make authorization decisions. Application Workers also run in TEEs, and the Policy determines if they can access the secrets associated with an Identity.

## 2. Environment Setup

We'll set up a local development environment using Docker, consisting of a local blockchain (Anvil) and an nXCC node.

### Step 2.1: Create a Docker Network

First, create a dedicated Docker network so our containers can communicate easily.

```bash
docker network create nxcc-net
```

### Step 2.2: Start the Anvil Blockchain

Next, start an Anvil container. We'll give it a network alias `anvil` so the nXCC node can find it.

Open a new terminal window for this command, as it will run in the foreground.

```bash
# In Terminal 1
docker run --rm -it --name anvil-node -p 8545:8545 --network nxcc-net --network-alias anvil ghcr.io/foundry-rs/foundry:latest anvil --host 0.0.0.0 --chain-id 31337
```

Anvil will start and print a list of available accounts and their private keys. **Copy one of the private keys**; we'll need it throughout this guide.

### Step 2.3: Start the nXCC Node

Now, start the nXCC node container. It will connect to the same Docker network.

Open another new terminal window for this command.

```bash
# In Terminal 2
docker run -d --rm \
  --name nxcc-node \
  -p 6922:6922 \
  --network nxcc-net \
  -e RUST_LOG=info,nxcc_daemon=debug \
  -e NXCC_HTTP_API_ENABLED=true \
  -e NXCC_HTTP_API_CORS_ALLOWED_ORIGINS='*' \
  ghcr.io/nxcc-bridge/nxcc/node:latest
```

This command starts the nXCC node in the background. It exposes the HTTP API on port `6922`, which our CLI will use.

You can check the logs to see it start up:

```bash
docker logs -f nxcc-node
```

Press `Ctrl+C` to exit the logs. The node will continue running in the background.

## 3. Project Initialization

Now, let's create a new project for our worker and policy code.

Open a third terminal window for all the following commands.

```bash
# In Terminal 3
mkdir my-nxcc-project
cd my-nxcc-project
nxcc init .
```

This command scaffolds a new project with the following structure:

```
.
├── package.json
├── tsconfig.json
├── policies/
│   ├── default-policy.ts
│   └── manifest.template.json
└── workers/
    ├── manifest.template.json
    └── my-worker.ts
```

Install the dependencies:

```bash
npm install
```

## 4. The `Identity` Contract

The on-chain `Identity` contract manages the creation of identities and stores the link to their policies. We need to deploy this contract to our Anvil chain.

First, clone the nXCC repository to get the contract source code.

```bash
git clone https://github.com/nxcc-bridge/nxcc.git
```

Now, deploy the contract using `forge`:

```bash
# Set environment variables for convenience
export SIGNER_PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 # Replace with your Anvil private key
export RPC_URL=http://localhost:8545

# Navigate to the contracts directory and install dependencies
cd nxcc/contracts/evm
forge install

# Deploy the Identity contract
forge create --rpc-url $RPC_URL --private-key $SIGNER_PK src/Identity.sol:Identity
```

`forge` will output the address of the deployed contract. **Copy this address** and save it to an environment variable.

```bash
export IDENTITY_CONTRACT_ADDRESS=<your-deployed-contract-address>

# Go back to your project directory
cd ../../../
```

## 5. Create an Identity

With the contract deployed, we can now create an identity for our worker. This will mint an ERC-721 NFT to our Anvil account.

```bash
nxcc identity create 31337 $IDENTITY_CONTRACT_ADDRESS --signer $SIGNER_PK --gateway-url $RPC_URL
```

The CLI will output the details of the newly created identity, including the `id` (which is the NFT's `tokenId`). **Copy the `id`** and save it to an environment variable.

```bash
export IDENTITY_ID=<your-identity-id>
```

## 6. Configure the Policy

Every identity is governed by a policy. A policy is a special worker that decides whether to approve requests. The `nxcc init` command already created a default policy that approves all requests.

### Step 6.1: Review the Generated Policy

The generated policy is located at `policies/default-policy.ts`. You can examine it to see how policies work:

```typescript
// policies/default-policy.ts - already generated by nxcc init
export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const handlerName = url.pathname.startsWith("/") ? url.pathname.substring(1) : url.pathname;

    // Policy workers must handle the "_policy" endpoint
    if (handlerName !== "_policy") {
      console.error(`Policy worker received unexpected handler: ${handlerName}`);
      return new Response(`Policy worker: unexpected handler ${handlerName}`, { status: 400 });
    }

    try {
      // Parse the policy execution requests
      const contextsArray = await request.json();
      console.log(`Processing ${contextsArray.length} policy execution requests`);

      // Default policy: approve all requests
      const resultsArray = contextsArray.map(() => true);

      return new Response(JSON.stringify(resultsArray), {
        status: 200,
        headers: { "content-type": "application/json; charset=utf-8" },
      });
    } catch (err) {
      console.error("Policy worker error:", err);
      return new Response(
        JSON.stringify({
          error: "Policy worker execution failed",
          message: err instanceof Error ? err.message : String(err),
        }),
        {
          status: 500,
          headers: { "content-type": "application/json; charset=utf-8" },
        },
      );
    }
  },
};
```

### Step 6.2: Review the Policy Manifest

The policy manifest at `policies/manifest.template.json` is already configured correctly. Note that a policy manifest **must not** request any `identities`:

```json
// policies/manifest.template.json - already generated by nxcc init
{
  "userdata": {
    "description": "Default policy that accepts all authorization requests",
    "version": "1.0.0"
  },
  "identities": []
}
```

### Step 6.3: Build and Bundle the Policy

First, update the `tsconfig.json` to include the policy file in compilation:

```json
// tsconfig.json
{
  // ...
  "include": ["workers/my-worker.ts", "policies/default-policy.ts"]
}
```

Compile the TypeScript to JavaScript:

```bash
npm run build
```

This creates `dist/default-policy.js`. Now, update the policy manifest to point to the compiled code and bundle it:

```json
// policies/manifest.template.json - update the bundle source
{
  "bundle": {
    "source": "./dist/default-policy.js"
  },
  "userdata": {
    "description": "Default policy that accepts all authorization requests",
    "version": "1.0.0"
  },
  "identities": []
}
```

Bundle the manifest and compiled code into a single, self-contained JSON file:

```bash
nxcc bundle policies/manifest.template.json --out policies/policy.bundle.json
```

### Step 6.4: Set the Policy

Finally, link this policy bundle to the identity you created. The CLI will automatically convert the file path to a `data:` URL and set it on your identity NFT.

```bash
nxcc identity set-policy 31337 $IDENTITY_CONTRACT_ADDRESS $IDENTITY_ID policies/policy.bundle.json --signer $SIGNER_PK --gateway-url $RPC_URL
```

Your identity is now set up and governed by the allow-all policy!

## 7. The Application Contract

Our worker will listen for events from one smart contract and call a function on another. For simplicity in this guide, we'll use the same contract for both.

Let's deploy the `TestEvents` contract from the nXCC repository:

```bash
# From your project root
cd nxcc/node/tests
forge create --rpc-url $RPC_URL --private-key $SIGNER_PK contracts/TestEvents.sol:TestEvents
```

`forge` will output the deployed contract address. **Copy this address** and save it.

```bash
export EVENT_CONTRACT_ADDRESS=<your-deployed-event-contract-address>

# Go back to your project directory
cd ../../
```

## 8. Create a Cross-Chain Worker

Now for the main event: the cross-chain worker. This worker will listen for a `ValueChanged` event from our `TestEvents` contract and call the `updateState` function on the same contract in response.

### Step 8.1: Write the Worker Code

First, install `viem`, a powerful TypeScript library for interacting with Ethereum.

```bash
npm install viem
```

Create a new file `workers/cross-chain-worker.ts` with the following code:

```typescript
// workers/cross-chain-worker.ts
import {
  createPublicClient,
  http,
  createWalletClient,
  privateKeyToAccount,
  parseAbiItem,
  decodeEventLog,
  Hex,
} from "viem";
import { anvil } from "viem/chains";

// This is a simple contract ABI for the function we want to call.
const contractAbi = [
  parseAbiItem("function updateState(uint256 newValue, bytes calldata data)"),
  parseAbiItem("event ValueChanged(uint256 indexed newValue, bytes data)"),
];

export default {
  async fetch(request: Request, env: any): Promise<Response> {
    // The 'env' object contains secrets and userdata.
    // 'USER_CONFIG' is from the manifest's userdata.
    const { rpcUrl, contractAddress, signerPrivateKey } = env.USER_CONFIG;

    if (!rpcUrl || !contractAddress || !signerPrivateKey) {
      return new Response(
        "Missing required configuration in manifest's userdata",
        { status: 500 },
      );
    }

    // The event payload from the nXCC node is the request body.
    const eventPayload = await request.json();

    // Decode the event log.
    const decodedLog = decodeEventLog({
      abi: contractAbi,
      eventName: "ValueChanged",
      data: eventPayload.data as Hex,
      topics: eventPayload.topics as [Hex, ...Hex[]],
    });

    const { newValue, data } = decodedLog.args;
    console.log(
      `Worker received ValueChanged event: newValue=${newValue}, data=${data}`,
    );

    // Set up viem clients to send a transaction.
    const account = privateKeyToAccount(signerPrivateKey as Hex);
    const publicClient = createPublicClient({
      chain: anvil,
      transport: http(rpcUrl),
    });
    const walletClient = createWalletClient({
      account,
      chain: anvil,
      transport: http(rpcUrl),
    });

    try {
      console.log(
        `Calling updateState(${newValue}, "${data}") on ${contractAddress}`,
      );
      const { request: txRequest } = await publicClient.simulateContract({
        address: contractAddress as Hex,
        abi: contractAbi,
        functionName: "updateState",
        args: [newValue, data],
        account,
      });

      const hash = await walletClient.writeContract(txRequest);
      console.log(`Transaction sent: ${hash}`);
      await publicClient.waitForTransactionReceipt({ hash });
      console.log(`Transaction confirmed: ${hash}`);

      return new Response(JSON.stringify({ success: true, txHash: hash }));
    } catch (e: any) {
      console.error(`Transaction failed: ${e.message}`);
      return new Response(
        JSON.stringify({ success: false, error: e.message }),
        { status: 500 },
      );
    }
  },
};
```

### Step 8.2: Configure the Worker Manifest

Create a manifest file for this worker at `workers/worker.manifest.json`. This manifest defines what the worker does and what it needs to run.

- `bundle.source`: Points to the compiled worker code.
- `userdata`: Contains configuration passed to the worker, like RPC URLs and contract addresses.
- `events`: Tells the nXCC node to listen for specific on-chain events and trigger this worker.

```json
// workers/worker.manifest.json
{
  "bundle": {
    "source": "./dist/cross-chain-worker.js"
  },
  "identities": [],
  "userdata": {
    "name": "cross-chain-demo",
    "rpcUrl": "http://anvil:8545",
    "contractAddress": "YOUR_EVENT_CONTRACT_ADDRESS",
    "signerPrivateKey": "YOUR_SIGNER_PRIVATE_KEY"
  },
  "events": [
    {
      "handler": "fetch",
      "kind": "web3_event",
      "chain": 31337,
      "address": ["YOUR_EVENT_CONTRACT_ADDRESS"],
      "topics": [
        ["0x35c2b3b04a37f2752491485a4b51c863265557ac8152345842775344ba3a017b"]
      ]
    }
  ]
}
```

**Important:**

1.  Replace `YOUR_EVENT_CONTRACT_ADDRESS` with the address of the `TestEvents` contract you deployed.
2.  Replace `YOUR_SIGNER_PRIVATE_KEY` with the Anvil private key you've been using.
3.  The `rpcUrl` is `http://anvil:8545` because the worker code runs inside the `nxcc-node` Docker container, which can reach the Anvil container via its network alias `anvil`.

> **Security Note:** In a real application, you would **never** put a private key in `userdata`. Instead, you would request an `identity` in the manifest, and the nXCC node would securely inject a generated secret into the worker's environment. We use `userdata` here for simplicity in this local-only tutorial.

### Step 8.3: Build the Worker

Finally, build the new worker. You'll need to edit your `tsconfig.json` to include the new file. Add `"workers/cross-chain-worker.ts"` to the `include` array.

```json
// tsconfig.json
{
  // ...
  "include": ["policies/default-policy.ts", "workers/my-worker.ts", "workers/cross-chain-worker.ts"]
}
```

Now, run the build command:

```bash
npm run build
```

## 9. Deploy and Run!

You have everything in place. Let's deploy the worker and see it in action.

### Step 9.1: Deploy the Worker

Deploy the worker to your local nXCC node. This sends a "work order" to the node, telling it to start the worker and listen for the configured events.

```bash
nxcc worker deploy workers/worker.manifest.json --rpc-url http://localhost:6922
```

The CLI will confirm that the worker was deployed successfully.

### Step 9.2: Trigger the Event

Now, trigger the `ValueChanged` event on the `TestEvents` contract using `cast`.

```bash
cast send $EVENT_CONTRACT_ADDRESS "triggerEvent(uint256,bytes)" 42 "0xbeef" --private-key $SIGNER_PK --rpc-url $RPC_URL
```

### Step 9.3: Observe the Logs

The nXCC node will detect the event and invoke your worker. Watch the logs from the node container to see it happen in real-time.

```bash
# In Terminal 2 (or a new one)
docker logs -f nxcc-node
```

You should see output from your worker, like:

```
INFO nxcc_daemon::services::work_order_orchestrator: Requesting enclave to run worker for work order...
INFO nxcc_daemon::services::work_order_orchestrator: Enclave started worker ...
...
DEBUG nxcc_daemon::web3::listener: Received log for work_order_id: ...
...
INFO nxcc_workerd_vm::vmm: stdout: Worker received ValueChanged event: newValue=42, data=0xbeef
INFO nxcc_workerd_vm::vmm: stdout: Calling updateState(42, "0xbeef") on 0x...
INFO nxcc_workerd_vm::vmm: stdout: Transaction sent: 0x...
INFO nxcc_workerd_vm::vmm: stdout: Transaction confirmed: 0x...
```

### Step 9.4: Verify the Result

The worker's log shows it sent a transaction to call `updateState`. Let's verify that the state of the contract has changed.

Check the `value`:

```bash
cast call $EVENT_CONTRACT_ADDRESS "value()(uint256)" --rpc-url $RPC_URL
```

Output should be `42`.

Check the `data`:

```bash
cast call $EVENT_CONTRACT_ADDRESS "data()(bytes)" --rpc-url $RPC_URL
```

Output should be `0xbeef`.

**Congratulations!** You've successfully built a secure, event-driven, cross-chain application with nXCC.

## Conclusion

In this guide, you learned how to:

- Set up a complete local development environment for nXCC.
- Create on-chain identities and govern them with policies.
- Write, deploy, and trigger workers that react to blockchain events.

You've built a powerful piece of automation that securely connects on-chain events to off-chain actions. From here, you can explore more complex workers that interact with external APIs, manage secrets, and orchestrate workflows across multiple chains.
