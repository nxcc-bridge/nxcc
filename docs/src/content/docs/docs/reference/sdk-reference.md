---
title: SDK Reference
description: Complete API reference for the @nxcc/sdk package.
---

The `@nxcc/sdk` provides TypeScript utilities for building workers and policies on the nXCC platform. This reference covers all functions, types, and utilities available in the SDK.

## Installation

```bash
npm install @nxcc/sdk
```

The SDK is automatically included in projects created with `nxcc init`.

## Core Functions

### `worker()`

Creates a worker that handles the nXCC execution protocol with automatic response conversion.

```typescript
function worker(config: WorkerConfig): WorkerObject;
```

**Parameters:**

- `config: WorkerConfig` - Configuration object with event handlers

**Returns:**

- `WorkerObject` - Object compatible with Cloudflare Workers runtime

**Example:**

```typescript
import { worker } from "@nxcc/sdk";

export default worker({
  async launch(eventPayload, context) {
    console.log("Worker starting up!", eventPayload);
  },

  async fetch(request, context) {
    return { message: "Hello from worker!" };
  },

  async customHandler(eventPayload, context) {
    return { status: "processed", data: eventPayload };
  },
});
```

### `policy()`

Creates a policy worker that handles authorization requests.

```typescript
function policy(handler: PolicyHandler): WorkerObject;
```

**Parameters:**

- `handler: PolicyHandler` - Function that processes authorization requests

**Returns:**

- `WorkerObject` - Policy worker compatible with nXCC runtime

**Example:**

```typescript
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map((request) => {
    // Your authorization logic
    return request.env_report?.attestation?.measurement === "trusted-hash";
  });
});
```

## Types and Interfaces

### `WorkerConfig`

Configuration object for creating workers with the `worker()` function.

```typescript
interface WorkerConfig {
  launch?: WorkerLaunchHandler;
  fetch?: WorkerHttpHandler;
  [handlerName: string]:
    | CustomEventHandler
    | WorkerLaunchHandler
    | WorkerHttpHandler
    | undefined;
}
```

**Properties:**

- `launch?` - Handler for launch events (worker initialization)
- `fetch?` - Handler for HTTP requests
- `[handlerName]` - Custom event handlers for specific events

### `WorkerContext`

Context object passed to all worker handlers containing environment data.

```typescript
interface WorkerContext {
  userdata: Record<string, unknown>;
  env: any;
}
```

**Properties:**

- `userdata` - User-defined configuration from the worker manifest
- `env` - Runtime environment, includes `USER_CONFIG` and other bindings

**Example usage:**

```typescript
export default worker({
  async fetch(request, { userdata, env }) {
    const config = userdata.apiSettings;
    const secret = env.MY_SECRET; // From identity binding

    return { config, hasSecret: !!secret };
  },
});
```

### Handler Function Types

#### `WorkerLaunchHandler`

Handler for worker initialization events.

```typescript
type WorkerLaunchHandler = (
  eventPayload: Record<string, unknown>,
  context: WorkerContext,
) => Promise<void | any> | void | any;
```

**Use cases:**

- One-time initialization
- Setting up polling loops
- Logging startup information

#### `WorkerHttpHandler`

Handler for HTTP requests.

```typescript
type WorkerHttpHandler<T = any> = (
  request: Request,
  context: WorkerContext,
) => Promise<T> | T;
```

**Parameters:**

- `request: Request` - Standard Web API Request object
- `context: WorkerContext` - Worker execution context

#### `CustomEventHandler`

Handler for custom events and general-purpose processing.

```typescript
type CustomEventHandler = (
  eventPayload: Record<string, unknown>,
  context: WorkerContext,
) => Promise<void | any> | void | any;
```

## Policy Types

### `PolicyHandler`

Main handler function for policy workers.

```typescript
type PolicyHandler = (
  requests: PolicyExecutionRequest[],
) => boolean[] | Promise<boolean[]>;
```

**Parameters:**

- `requests` - Array of authorization requests to evaluate

**Returns:**

- Array of boolean decisions (same length as input)

### `PolicyExecutionRequest`

Request object containing authorization context.

```typescript
interface PolicyExecutionRequest {
  env_report: {
    node_id: string;
    attestation: any; // Raw attestation report
  };
  secret_ids: string[];
  consumer: any;
  attestation_claims?: AttestationClaims;
}
```

**Properties:**

- `env_report` - TEE attestation data for the requesting node
- `secret_ids` - Array of secret identifiers being requested
- `consumer` - Information about the requesting worker
- `attestation_claims?` - Standardized attestation claims (when available)

### `AttestationClaims`

Standardized attestation claims following IETF EAT (RFC 9711).

```typescript
interface AttestationClaims {
  iat: number; // Issued at time
  eat_nonce?: Uint8Array; // Verifier challenge
  ueid?: Uint8Array; // Device identity
  oemid?: string; // Manufacturer ID
  hwmodel?: string; // Hardware model
  hwversion?: string; // Hardware version
  dbgstat: number; // Debug status (0=production, 4=debug)
  oemboot?: boolean; // Secure boot status
  swname?: string; // Software name
  swversion?: string; // Software version
  measurements: Array<{
    val: Uint8Array; // Hash value
    alg: string; // Hash algorithm
    measurement_type?: string; // Category
    vendor?: string; // Vendor info
    version?: string; // Version info
  }>;
  cnf?: {
    // Proof-of-possession key
    jwk?: {
      kty: string; // Key type
      crv?: string; // Curve
      x?: string; // X coordinate
      y?: string; // Y coordinate
    };
    cose_key?: Uint8Array;
  };
  intuse?: number; // Intended use
  uptime?: number; // Seconds since boot
  bootcount?: number; // Boot count
  bootseed?: Uint8Array; // Boot seed
  eat_profile: string; // Profile identifier
}
```

## Response Conversion

The SDK automatically converts handler return values to appropriate HTTP responses:

### Automatic Conversions

| Return Value                   | HTTP Response                                           |
| ------------------------------ | ------------------------------------------------------- |
| `Response` object              | Returned as-is (full control)                           |
| `object` or `array`            | JSON with `200 OK` and `Content-Type: application/json` |
| `undefined`, `null`, or `void` | `204 No Content`                                        |
| `Error` object                 | `500 Internal Server Error` with error message          |

### Examples

```typescript
export default worker({
  // Returns 204 No Content
  async launch() {
    console.log("Started");
    // No return = 204
  },

  // Returns JSON with 200 OK
  async fetch(request) {
    return { message: "Hello", timestamp: Date.now() };
  },

  // Returns custom Response
  async customHandler() {
    return new Response("Custom content", {
      status: 201,
      headers: { "X-Custom": "header" },
    });
  },

  // Returns 500 on error
  async errorHandler() {
    throw new Error("Something went wrong");
  },
});
```

## Advanced Usage

### Multiple Handler Types

Workers can handle different event types with specific handlers:

```typescript
export default worker({
  // Launch event (worker initialization)
  async launch(eventPayload, { userdata }) {
    console.log("Worker initialized with config:", userdata);
  },

  // HTTP requests
  async fetch(request, { userdata }) {
    const url = new URL(request.url);
    return {
      method: request.method,
      path: url.pathname,
      config: userdata,
    };
  },

  // Blockchain event handler
  async handleTransfer(eventPayload, { userdata }) {
    const { from, to, value } = eventPayload.args;
    console.log(`Transfer: ${from} -> ${to} (${value})`);

    return { processed: true, transfer: { from, to, value } };
  },

  // Scheduled event handler
  async tick(eventPayload, { userdata }) {
    const timestamp = new Date().toISOString();
    console.log(`Scheduled tick at ${timestamp}`);

    return { tick: timestamp, config: userdata };
  },
});
```

### Error Handling Patterns

```typescript
export default worker({
  async fetch(request, { userdata }) {
    try {
      const data = await request.json();

      // Validate input
      if (!data.required_field) {
        return new Response("Missing required field", { status: 400 });
      }

      // Process data
      const result = await processData(data);
      return { success: true, result };
    } catch (error) {
      console.error("Processing error:", error);

      // Return structured error response
      return new Response(
        JSON.stringify({
          error: "Processing failed",
          message: error.message,
          timestamp: new Date().toISOString(),
        }),
        {
          status: 500,
          headers: { "Content-Type": "application/json" },
        },
      );
    }
  },
});
```

### Policy Authorization Patterns

```typescript
// Simple allowlist policy
export default policy((requests) => {
  const allowedBundles = ["sha256:abc123...", "sha256:def456..."];

  return requests.map((request) => {
    return allowedBundles.includes(request.consumer?.bundle_hash);
  });
});

// Multi-factor authorization
export default policy((requests) => {
  return requests.map((request) => {
    const claims = request.attestation_claims;

    // Check multiple criteria
    const validMeasurement = claims?.measurements?.[0]?.val === expectedHash;
    const productionMode = claims?.dbgstat === 0;
    const trustedSoftware = claims?.swname === "nxcc-platform";

    return validMeasurement && productionMode && trustedSoftware;
  });
});

// Time-based restrictions
export default policy((requests) => {
  const now = new Date();
  const isBusinessHours = now.getUTCHours() >= 9 && now.getUTCHours() <= 17;

  return requests.map((request) => {
    if (!isBusinessHours) {
      console.log("Access denied outside business hours");
      return false;
    }

    // Additional authorization logic...
    return performAdditionalChecks(request);
  });
});
```

## Environment Integration

### Accessing Userdata

Userdata from the worker manifest is available in the context:

```typescript
// worker.manifest.json
{
  "userdata": {
    "apiUrl": "https://api.example.com",
    "timeout": 5000,
    "features": ["feature1", "feature2"]
  }
}

// worker.ts
export default worker({
  async fetch(request, { userdata }) {
    const apiUrl = userdata.apiUrl;
    const timeout = userdata.timeout;
    const features = userdata.features;

    return { apiUrl, timeout, features };
  }
});
```

### Runtime Environment

The full runtime environment is available via `context.env`:

```typescript
export default worker({
  async fetch(request, { userdata, env }) {
    return {
      userConfig: env.USER_CONFIG, // Same as userdata
      secrets: {
        hasEthSigner: !!env.ETH_SIGNER,
        hasApiKey: !!env.API_KEY,
      },
      environment: "worker runtime",
    };
  },
});
```

## Best Practices

### Worker Development

1. **Use TypeScript** for better type safety and IDE support
2. **Handle errors gracefully** with try-catch and structured error responses
3. **Log comprehensively** for debugging and monitoring
4. **Validate inputs** before processing
5. **Return structured data** for consistent API responses

### Policy Development

1. **Start restrictive** and gradually relax permissions as needed
2. **Always validate attestation data** before making decisions
3. **Use multiple authorization factors** for critical resources
4. **Log authorization decisions** for audit trails
5. **Test thoroughly** with mock data before deployment

### Performance

1. **Minimize dependencies** to reduce bundle size
2. **Use async/await** appropriately for I/O operations
3. **Cache expensive computations** when possible
4. **Return early** from handlers when possible
5. **Monitor resource usage** in logs

## Migration Guide

### From Manual Workers

If you have existing workers without the SDK:

```typescript
// Before (manual)
export default {
  async fetch(request) {
    return new Response(JSON.stringify({ message: "hello" }), {
      headers: { "Content-Type": "application/json" },
    });
  },
};

// After (with SDK)
import { worker } from "@nxcc/sdk";

export default worker({
  async fetch(request) {
    return { message: "hello" }; // Automatic JSON conversion
  },
});
```

### From Manual Policies

```typescript
// Before (manual)
export default {
  async fetch(request) {
    const requests = await request.json();
    const decisions = requests.map(() => true);
    return new Response(JSON.stringify(decisions));
  },
};

// After (with SDK)
import { policy } from "@nxcc/sdk";

export default policy((requests) => {
  return requests.map(() => true);
});
```

## Troubleshooting

### Common Issues

**SDK not found**:

```bash
npm install @nxcc/sdk
```

**TypeScript errors**:

```typescript
// Ensure proper imports
import { worker, type WorkerContext } from "@nxcc/sdk";
```

**Handler not called**:

- Check handler name matches the event handler in work order
- Verify the worker deployed successfully
- Check node logs for runtime errors

**Policy not working**:

- Ensure the policy handles the `_policy` endpoint
- Verify the policy bundle is accessible via the identity's tokenURI
- Check policy logs for authorization decisions

For more troubleshooting help, see the [Development Workflow](../guides/development-workflow.md) guide.
