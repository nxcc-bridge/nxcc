---
title: Worker Manifest Reference
description: A detailed specification for the worker.manifest.json file.
---

The `worker.manifest.json` file is the blueprint for your worker. It tells the nXCC node what code to run, what secrets it needs, and how to configure it.

## Structure

A manifest is a JSON object with three top-level properties: `bundle`, `identities`, and `userdata`.

```json
{
  "bundle": {
    "source": "./dist/worker.js"
  },
  "identities": [],
  "userdata": {
    "name": "my-first-worker"
  }
}
```

Note: Event triggers are specified separately in the work order when deploying the worker, not in the manifest itself.

### `bundle` (Object, Required)

The `bundle` object points to your worker's code. This code must be packaged in a self-contained, signed file called a "bundle".

- **`source`** (String, Required): A URL pointing to the worker bundle file. The `@nxcc/cli` tool typically handles creating this bundle and can use several URL schemes:
  - `file://path/to/bundle.json`: A local file path.
  - `data:application/json;base64,...`: The entire bundle, base64-encoded. The CLI's `--bundle` flag uses this to create portable work orders.
  - `http(s)://...`: A URL to a publicly accessible bundle file.

- **`hash`** (String, Optional): The SHA-512 hash of the bundle file for content integrity verification.

### `identities` (Array, Optional)

The `identities` array is used to request access to secrets associated with on-chain identities. **Policies are not allowed to request identities.**

It is an array of tuples, where each tuple contains a `SecretId` object and a `string` representing the name the secret will be bound to in the worker's environment.

```json
"identities": [
  [
    {
      "chain": 1,
      "identity_address": "0x...",
      "identity_id": "123"
    },
    "ETHEREUM_SIGNER"
  ]
]
```

In this example, the worker requests the secret for identity `123`. If the identity's policy approves the request, the secret will be available in the worker's runtime as `env.ETHEREUM_SIGNER`.

- **`SecretId` Object**:
  - `chain` (Number): The chain ID where the identity exists.
  - `identity_address` (String): The address of the `Identity.sol` contract.
  - `identity_id` (String): The token ID of the identity NFT.

### `userdata` (Object, Optional)

The `userdata` object is a place to put arbitrary, non-sensitive JSON configuration that will be passed to your worker.

**Manifest:**

```json
"userdata": {
  "rpcUrl": "https://mainnet.infura.io/v3/...",
  "retryCount": 3
}
```

**Worker Code (Manual Implementation):**

```typescript
export default {
  async fetch(request, env) {
    const { rpcUrl, retryCount } = env.USER_CONFIG;
    // ...
  },
};
```

**Worker Code (Using @nxcc/sdk):**

```typescript
import { worker } from "@nxcc/sdk";

export default worker({
  async fetch(request, { userdata }) {
    const { rpcUrl, retryCount } = userdata;
    // userdata is the same as env.USER_CONFIG
    // ...
  },
});
```

The entire `userdata` object is made available to the worker inside the `env` object at `env.USER_CONFIG`, and also as `context.userdata` when using the `@nxcc/sdk`.

## Work Order vs Manifest

When deploying a worker, the `nxcc worker deploy` command creates a **work order** that contains both your manifest and the event triggers. The work order structure includes:

```json
{
  "id": "worker-unique-id",
  "worker": {
    "bundle": { "source": "..." },
    "identities": [],
    "userdata": { ... }
  },
  "events": [
    {
      "handler": "fetch",
      "kind": "launch"
    }
  ]
}
```

See the [Event Triggers](./event-triggers) reference for a detailed guide on all available event kinds (`launch`, `http_request`, `web3_event`).

## Policy Manifests

Policy workers use the same manifest structure as regular workers, but with some important differences:

### Policy-Specific Considerations

1. **No identities**: Policy workers cannot request access to identities (they evaluate access for others)
2. **Special bundle**: Policy bundles are typically deployed to IPFS or served via HTTP
3. **SDK usage**: Policies can use the `@nxcc/sdk` policy helper for simplified development

### Policy Manifest Example

```json
{
  "bundle": {
    "source": "./dist/policy.js"
  },
  "identities": [],
  "userdata": {
    "policyName": "measurement-based-policy",
    "trustedMeasurements": ["0x1234567890abcdef...", "0xfedcba0987654321..."],
    "debugMode": false
  }
}
```

### Policy Implementation Options

**Using the SDK (Recommended):**

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    // Your authorization logic using userdata
    const trustedHashes = userdata.trustedMeasurements;
    const measurement = request.env_report?.attestation?.measurement;
    return trustedHashes.includes(measurement);
  });
});
```

**Manual Implementation:**

```typescript
export default {
  async fetch(request, env) {
    if (new URL(request.url).pathname.substring(1) !== "_policy") {
      return new Response("Invalid endpoint", { status: 400 });
    }

    const requests = await request.json();
    const { trustedMeasurements } = env.USER_CONFIG;

    const decisions = requests.map((req) => {
      const measurement = req.env_report?.attestation?.measurement;
      return trustedMeasurements.includes(measurement);
    });

    return new Response(JSON.stringify(decisions), {
      headers: { "Content-Type": "application/json" },
    });
  },
};
```
