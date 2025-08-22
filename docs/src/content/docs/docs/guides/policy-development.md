---
title: Policy Development Tutorial
description: Learn to write authorization policies that control access to secrets and identities.
---

Policies are the gatekeepers of the nXCC platform. They're special workers that authorize access to identity secrets by evaluating attestation data and other context. This tutorial covers writing effective policies from simple allow-all rules to sophisticated attestation-based authorization.

## What Are Policies?

Policies are workers that:

- Run in secure enclaves (TEEs) for tamper-proof execution
- Receive `PolicyExecutionRequest` objects containing attestation data
- Return boolean decisions (`true` = allow, `false` = deny)
- Control access to identity secrets and capabilities

Unlike regular workers, policies handle the special `_policy` endpoint and must return authorization decisions.

## Quick Start: Your First Policy

### 1. Create a Policy Project

```bash
# Create a project with policy template
nxcc init my-policy-project
cd my-policy-project

# Look at the generated policy
cat policies/default-policy.ts
```

The generated policy uses the SDK helper:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  // Default policy: approve all requests
  return requests.map(() => true);
});
```

### 2. Build and Deploy the Policy

```bash
# Build the TypeScript
npm run build

# Create a policy bundle
nxcc bundle policies/manifest.template.json --out policies/policy-bundle.json

# The policy bundle can be deployed to IPFS, served via HTTP, or used as data: URL
```

### 3. Link Policy to an Identity

```bash
# First, create an identity (requires local Anvil)
npx anvil &

# Deploy the Identity contract
cd contracts/evm
forge create src/Identity.sol:Identity --rpc-url http://localhost:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Create an identity NFT
nxcc identity create 31337 <contract-address> --signer 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Set the policy URL (use file:// or data: URL for testing)
nxcc identity set-policy 31337 <contract-address> <token-id> policies/policy-bundle.json --signer 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
```

## SDK Policy Helper vs Manual Implementation

### Using the SDK Helper (Recommended)

The `@nxcc/sdk` provides a `policy()` helper that handles the boilerplate:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    // Your authorization logic here
    return evaluateRequest(request);
  });
});
```

Benefits:

- Automatic `_policy` endpoint handling
- JSON parsing and response formatting
- Error handling and validation
- Type safety with TypeScript

### Manual Implementation

For full control, implement the policy manually:

```typescript
interface PolicyExecutionRequest {
  secret_ids: string[];
  consumer: any;
  env_report: {
    attestation: {
      measurement: string;
      user_data: Uint8Array;
    };
  };
  attestation_claims?: AttestationClaims;
}

export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const handler = url.pathname.startsWith("/")
      ? url.pathname.substring(1)
      : url.pathname;

    if (handler !== "_policy") {
      return new Response("Invalid endpoint", { status: 400 });
    }

    try {
      const requests: PolicyExecutionRequest[] = await request.json();
      const decisions = requests.map((req) => evaluateRequest(req));

      return new Response(JSON.stringify(decisions), {
        headers: { "Content-Type": "application/json" },
      });
    } catch (error) {
      return new Response("Policy error", { status: 500 });
    }
  },
};
```

## Policy Patterns

### 1. Allow-All Policy

Useful for development and testing:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  console.log(`Authorizing ${requests.length} requests`);
  return requests.map(() => true);
});
```

### 2. Measurement-Based Policy

Authorize based on TEE measurements (production security):

```typescript
import { policy } from "@nxcc/sdk";

// Trusted measurement hashes for your application
const TRUSTED_MEASUREMENTS = [
  "0x1234567890abcdef...", // Production build measurement
  "0xfedcba0987654321...", // Staging build measurement
];

export default policy((requests) => {
  return requests.map((request) => {
    const measurement = request.env_report?.attestation?.measurement;

    if (!measurement) {
      console.log("No measurement provided - denying");
      return false;
    }

    const isValid = TRUSTED_MEASUREMENTS.includes(measurement);
    console.log(`Measurement ${measurement}: ${isValid ? "ALLOW" : "DENY"}`);

    return isValid;
  });
});
```

### 3. Attestation Claims Policy

Use standardized attestation claims for detailed authorization:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    const claims = request.attestation_claims;

    if (!claims) {
      console.log("No attestation claims - denying");
      return false;
    }

    // Check debug status (0 = production, 4 = debug)
    if (claims.dbgstat !== 0) {
      console.log("Debug mode detected - denying");
      return false;
    }

    // Check software name
    if (claims.swname !== "nxcc-platform") {
      console.log("Invalid software name - denying");
      return false;
    }

    // Check hardware model (optional)
    if (claims.hwmodel && !claims.hwmodel.startsWith("Intel")) {
      console.log("Non-Intel hardware - denying");
      return false;
    }

    console.log("All attestation checks passed - allowing");
    return true;
  });
});
```

### 4. Consumer-Based Policy

Authorize based on worker characteristics:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    const consumer = request.consumer;

    // Check bundle hash against allowlist
    const allowedBundles = [
      "sha256:abcd1234...", // Your trusted worker bundle
      "sha256:5678efgh...", // Another approved worker
    ];

    const bundleHash = consumer.bundle_hash;
    const isAllowed = allowedBundles.includes(bundleHash);

    console.log(
      `Consumer bundle ${bundleHash}: ${isAllowed ? "ALLOW" : "DENY"}`,
    );
    return isAllowed;
  });
});
```

### 5. Time-Based Policy

Add temporal restrictions:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    const now = new Date();
    const hour = now.getUTCHours();

    // Only allow during business hours (UTC)
    const businessHours = hour >= 9 && hour <= 17;

    if (!businessHours) {
      console.log(
        `Access denied outside business hours (current: ${hour}:00 UTC)`,
      );
      return false;
    }

    // Additional checks...
    return evaluateOtherCriteria(request);
  });
});
```

### 6. Multi-Factor Policy

Combine multiple authorization criteria:

```typescript
import { policy } from "@nxcc/sdk";

const TRUSTED_MEASUREMENTS = ["0x1234..."];
const ALLOWED_BUNDLES = ["sha256:abcd..."];

export default policy((requests) => {
  return requests.map((request) => {
    const checks = {
      measurement: false,
      bundle: false,
      attestation: false,
    };

    // Check 1: TEE measurement
    const measurement = request.env_report?.attestation?.measurement;
    checks.measurement = TRUSTED_MEASUREMENTS.includes(measurement);

    // Check 2: Worker bundle
    const bundleHash = request.consumer?.bundle_hash;
    checks.bundle = ALLOWED_BUNDLES.includes(bundleHash);

    // Check 3: Attestation claims
    const claims = request.attestation_claims;
    checks.attestation = claims?.dbgstat === 0; // Production mode

    // All checks must pass
    const decision = checks.measurement && checks.bundle && checks.attestation;

    console.log(`Authorization checks:`, checks, `Result: ${decision}`);
    return decision;
  });
});
```

## Advanced Policy Development

### Error Handling

Robust error handling for production policies:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    try {
      // Your authorization logic
      return performAuthorizationChecks(request);
    } catch (error) {
      console.error("Policy error:", error);

      // Fail-safe: deny on errors
      return false;
    }
  });
});

function performAuthorizationChecks(request) {
  // Validate required fields
  if (!request.env_report?.attestation?.measurement) {
    throw new Error("Missing measurement data");
  }

  // Your authorization logic here
  return true;
}
```

### Logging for Audit Trails

Add comprehensive logging for compliance:

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  const timestamp = new Date().toISOString();

  return requests.map((request, index) => {
    const requestId = `${timestamp}-${index}`;

    console.log(`[${requestId}] Policy evaluation started`);
    console.log(`[${requestId}] Secrets requested:`, request.secret_ids);
    console.log(
      `[${requestId}] Consumer bundle:`,
      request.consumer?.bundle_hash,
    );

    const decision = evaluateRequest(request);

    console.log(`[${requestId}] Decision: ${decision ? "ALLOW" : "DENY"}`);
    return decision;
  });
});
```

### Testing Policies

Create test harnesses for policy development:

```typescript
// test-policy.js
import policy from "./policies/my-policy.js";

const mockRequest = {
  secret_ids: ["test-secret"],
  consumer: { bundle_hash: "sha256:abcd..." },
  env_report: {
    attestation: {
      measurement: "0x1234...",
    },
  },
  attestation_claims: {
    dbgstat: 0,
    swname: "nxcc-platform",
  },
};

// Test the policy
const decisions = await policy.handleRequests([mockRequest]);
console.log("Policy decision:", decisions[0]);
```

## Deployment and Management

### Policy Bundle Creation

```bash
# Build and bundle your policy
npm run build
nxcc bundle policies/manifest.template.json --out policy-bundle.json

# Sign the bundle (optional but recommended)
nxcc bundle policies/manifest.template.json --out policy-bundle.json --signer 0x...
```

### Policy Updates

```bash
# Update an existing identity's policy
nxcc identity set-policy <chain> <contract> <token-id> ./new-policy-bundle.json --signer 0x...
```

### IPFS Deployment

```bash
# Deploy to IPFS for decentralized hosting
ipfs add policy-bundle.json
# Use the returned IPFS hash as the policy URL
```

## Security Best Practices

1. **Always validate attestation data** - Don't trust requests without verification
2. **Use measurement-based authorization** - Ensure only trusted code can access secrets
3. **Implement comprehensive logging** - Track all authorization decisions
4. **Test thoroughly** - Use mock data to verify policy logic
5. **Fail securely** - Deny access when in doubt or on errors
6. **Keep policies simple** - Complex logic increases attack surface
7. **Regular updates** - Update trusted measurements when deploying new code

## Next Steps

- **[Identity Management](../reference/identities-and-policies.md)** - Learn about identity NFTs and on-chain management
- **[Worker Runtime](../reference/worker-runtime.md)** - Understand the execution environment
- **[Development Workflow](./development-workflow.md)** - Master local testing and debugging

## Common Pitfalls

- **Forgetting the `_policy` endpoint** - Policies must handle this specific endpoint
- **Not validating attestation claims** - Always check that attestation data exists
- **Overly permissive policies** - Start restrictive and gradually relax as needed
- **Poor error handling** - Always fail securely when something goes wrong
- **Insufficient logging** - Make debugging and auditing easier with good logs
