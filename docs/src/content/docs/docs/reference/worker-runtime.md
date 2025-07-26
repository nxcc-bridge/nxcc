---
title: Worker Runtime APIs
description: Learn about the execution environment and APIs available to your nXCC workers.
---

nXCC workers execute in a secure, serverless environment powered by Cloudflare's `workerd` runtime. This provides a robust, web-standard API surface that will be familiar to developers who have worked with Cloudflare Workers, Deno, or modern web browsers.

## Worker Entrypoint

Your worker's entrypoint must be a module that exports a default object with an `async fetch` method. This method is the universal handler for all incoming events, whether they are from an HTTP request, a Web3 event, or a launch trigger.

```typescript
// worker.ts
export default {
  async fetch(request: Request, env: any, ctx: any): Promise<Response> {
    console.log("Worker invoked!");

    // The request body contains the event payload.
    // For non-HTTP events, it's a JSON object describing the trigger.
    const eventPayload = await request.json();

    // ... your logic here ...

    return new Response("Hello from nXCC!");
  },
};
```

### `fetch` Handler Arguments

-   **`request`** (`Request`): A standard [Fetch API `Request`](https://developer.mozilla.org/en-US/docs/Web/API/Request) object.
    -   For `http_request` triggers, this is the original incoming HTTP request.
    -   For all other triggers (e.g., `web3_event`, `launch`), the nXCC node constructs a `POST` request where the body contains a JSON payload describing the event.
-   **`env`** (`any`): An object containing environment variables, secrets, and user-configured data. See below for details.
-   **`ctx`** (`any`): An execution context object provided by the `workerd` runtime.

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
  { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(), info: new TextEncoder().encode("eth-child-key") },
  rootKey,
  { name: "ECDSA", namedCurve: "P-256" },
  false, // not extractable
  ["sign", "verify"]
);
```

## Standard Web APIs

The `workerd` runtime provides a wide range of standard web APIs. You can use them just as you would in a modern browser or other serverless JavaScript environments.

-   **`fetch()`**: Make outbound HTTP requests to any API.
-   **`crypto.subtle`**: Access the Web Crypto API for hashing, signing, and encryption.
-   **`setTimeout` / `setInterval`**: Schedule asynchronous tasks.
-   **`Request` / `Response` / `Headers`**: Construct and handle HTTP primitives.

For a comprehensive list of available APIs, please refer to the [Cloudflare Workers runtime documentation](https://developers.cloudflare.com/workers/runtime-apis/).
