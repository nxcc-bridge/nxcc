---
title: Worker Manifest Reference
description: A detailed specification for the worker.manifest.json file.
---

The `worker.manifest.json` file is the blueprint for your worker. It tells the nXCC node what code to run, what triggers to listen for, what secrets it needs, and how to configure it.

## Structure

A manifest is a JSON object with four top-level properties: `bundle`, `identities`, `userdata`, and `events`.

```json
{
  "bundle": {
    "source": "./dist/worker.js"
  },
  "identities": [],
  "userdata": {
    "name": "my-first-worker"
  },
  "events": [
    {
      "handler": "fetch",
      "kind": "launch"
    }
  ]
}
```

### `bundle` (Object, Required)

The `bundle` object points to your worker's code. This code must be packaged in a self-contained, signed file called a "bundle".

-   **`source`** (String, Required): A URL pointing to the worker bundle file. The `@nxcc/cli` tool typically handles creating this bundle and can use several URL schemes:
    -   `file://path/to/bundle.json`: A local file path.
    -   `data:application/json;base64,...`: The entire bundle, base64-encoded. The CLI's `--bundle` flag uses this to create portable work orders.
    -   `http(s)://...`: A URL to a publicly accessible bundle file.

-   **`hash`** (String, Optional): The SHA-512 hash of the bundle file for content integrity verification.

### `identities` (Array, Optional)

The `identities` array is used to request access to secrets associated with on-chain identities. **Policies are not allowed to request identities.**

It is an array of tuples, where each tuple contains a `SecretId` object and a `string` representing the name the secret will be bound to in the worker's environment.

```json
"identities": [
  [
    {
      "chain_id": 1,
      "identity_address": "0x...",
      "identity_id": "123"
    },
    "ETHEREUM_SIGNER"
  ]
]
```

In this example, the worker requests the secret for identity `123`. If the identity's policy approves the request, the secret will be available in the worker's runtime as `env.ETHEREUM_SIGNER`.

-   **`SecretId` Object**:
    -   `chain_id` (Number): The chain ID where the identity exists.
    -   `identity_address` (String): The address of the `Identity.sol` contract.
    -   `identity_id` (String): The token ID of the identity NFT.

### `userdata` (Object, Optional)

The `userdata` object is a place to put arbitrary, non-sensitive JSON configuration that will be passed to your worker.

**Manifest:**
```json
"userdata": {
  "rpcUrl": "https://mainnet.infura.io/v3/...",
  "retryCount": 3
}
```

**Worker Code:**
```typescript
export default {
  async fetch(request, env) {
    const { rpcUrl, retryCount } = env.USER_CONFIG;
    // ...
  }
}
```

The entire `userdata` object is made available to the worker inside the `env` object at `env.USER_CONFIG`.

### `events` (Array, Optional)

The `events` array defines what triggers your worker. See the [Event Triggers](./event-triggers) reference for a detailed guide on all available event kinds (`launch`, `http_request`, `web3_event`).
