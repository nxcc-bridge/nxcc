---
title: Worker Runtime APIs
description: Learn about the execution environment and APIs available to your nXCC workers.
---

nXCC workers execute in a secure, serverless environment powered by Cloudflare's `workerd` runtime. This provides a robust, web-standard API surface that will be familiar to developers who have worked with Cloudflare Workers, Deno, or modern web browsers.

## Worker Entrypoint

Your worker's entrypoint must be a module that exports a default object with an `async fetch` method. This method is the universal handler for all incoming events, whether they are from an HTTP request, a Web3 event, or a launch trigger.

```typescript
// worker.ts
import { worker } from "@nxcc/sdk";

export default worker({
  // Launch handlers can return void - automatically becomes 204 No Content
  async launch(eventPayload, { userdata }) {
    console.log("Worker launched!", eventPayload, userdata);
    // No return needed - automatically returns 204
  },

  // Fetch handlers: return objects are automatically JSON stringified with 200
  async fetch(request, { userdata }) {
    console.log(
      "HTTP request received:",
      request.method,
      new URL(request.url).pathname,
    );

    // Return object automatically becomes JSON response with 200 status
    return {
      message: "Hello from nXCC worker!",
      method: request.method,
      path: new URL(request.url).pathname,
    };
  },

  // Custom event handlers - return values are automatically converted
  async myCustomEvent(eventPayload, { userdata }) {
    console.log("Custom event received:", eventPayload);

    // Return object automatically becomes JSON response with 200 status
    return { handled: true, timestamp: Date.now(), payload: eventPayload };
  },
});
```

### Automatic Response Conversion

The nXCC worker SDK automatically converts different return types from your handlers into appropriate HTTP responses:

- **`Response` objects**: Returned as-is, giving you full control over status codes and headers
- **Objects/Arrays**: Automatically JSON stringified with `200 OK` status and `Content-Type: application/json` header
- **`undefined`/`null`/void**: Automatically returns `204 No Content` status
- **Errors**: Automatically converted to `500 Internal Server Error` with the error message

This means you can focus on your business logic without manually constructing Response objects for common cases:

```typescript
export default worker({
  async fetch(request, { userdata }) {
    // Return an object - becomes JSON with 200 status
    return { message: "Hello", timestamp: Date.now() };
  },

  async launch(eventPayload, context) {
    console.log("Worker started");
    // No return - automatically becomes 204 No Content
  },

  async customHandler(eventPayload, context) {
    // Still works - returned as-is for full control
    return new Response("Custom response", {
      status: 201,
      headers: { "X-Custom": "value" },
    });
  },

  async errorHandler(eventPayload, context) {
    throw new Error("Something went wrong");
    // Automatically becomes 500 with error message
  },
});
```

### How Worker Invocation Works

The nXCC runtime uses two different mechanisms to invoke your worker:

1. **Event-based invocation** (via `invoke_worker`): The runtime creates a POST request with a JSON payload containing:
   - `handler`: The name of the event handler to invoke (e.g., `"launch"`, `"web3_event"`)
   - `event_payload`: The specific event data for that trigger type

2. **HTTP invocation** (via `invoke_http`): The runtime passes through the original HTTP request directly to your `fetch` handler.

Your worker should detect which invocation type is being used and route accordingly.

### `fetch` Handler Arguments

- **`request`** (`Request`): A standard [Fetch API `Request`](https://developer.mozilla.org/en-US/docs/Web/API/Request) object.
  - For HTTP invocations, this is the original incoming HTTP request.
  - For event invocations, this is a POST request with a JSON body containing the handler name and event payload.
- **`env`** (`any`): An object containing environment variables, secrets, and user-configured data. See below for details.
- **`ctx`** (`any`): An execution context object provided by the `workerd` runtime.

## The `env` Object

The `env` object is your worker's primary interface to its configuration and secrets.

### `env.USER_CONFIG`

This property contains the entire `userdata` object from your `worker.manifest.json` file. It's the standard way to pass non-sensitive configuration, like RPC URLs or API endpoints, to your worker.

```typescript
// Accessing userdata
const { rpcUrl, contractAddress } = env.USER_CONFIG;
```

### `env.YOUR_SECRET_NAME`

If your worker requests access to an identity in its manifest and the identity's policy approves the request, the associated secret is injected into the `env` object. The property name matches the name you provided in the `identities` array.

For security, the raw secret is **not** exposed directly. Instead, it is provided as a standard [Web Crypto `CryptoKey`](https://developer.mozilla.org/en-US/docs/Web/API/CryptoKey) object. This key is non-extractable and its usage is restricted to derivation functions like `deriveKey` and `deriveBits`. This prevents the worker code (or any compromised dependency) from leaking the root secret.

```typescript
// Example: Deriving an Ethereum-compatible key from an identity secret
const rootKey = env.ETHEREUM_SIGNER; // This is a CryptoKey

const derivedKey = await crypto.subtle.deriveKey(
  {
    name: "HKDF",
    hash: "SHA-256",
    salt: new Uint8Array(),
    info: new TextEncoder().encode("eth-child-key"),
  },
  rootKey,
  { name: "ECDSA", namedCurve: "P-256" },
  false, // not extractable
  ["sign", "verify"],
);
```

## Standard Web APIs

The `workerd` runtime provides a wide range of standard web APIs. You can use them just as you would in a modern browser or other serverless JavaScript environments.

- **`fetch()`**: Make outbound HTTP requests to any API.
- **`crypto.subtle`**: Access the Web Crypto API for hashing, signing, and encryption.
- **`setTimeout` / `setInterval`**: Schedule asynchronous tasks.
- **`Request` / `Response` / `Headers`**: Construct and handle HTTP primitives.

For a comprehensive list of available APIs, please refer to the [Cloudflare Workers runtime documentation](https://developers.cloudflare.com/workers/runtime-apis/).
