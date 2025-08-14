# @nxcc/sdk

SDK for interacting with the NXCC platform, providing utilities and helper functions.

## Installation

```bash
npm install @nxcc/sdk
```

## Modules

### Crypto

The crypto module provides cryptographic utilities for secure operations.

#### Key Derivation

```typescript
import { crypto } from "@nxcc/sdk";

// Create or import a master key
const masterKey = await crypto.subtle.importKey(
  "raw",
  crypto.getRandomValues(new Uint8Array(32)), // 256-bit key
  { name: "HKDF" },
  false,
  ["deriveBits"],
);

// Derive a key for encryption
const encryptionKey = await crypto.deriveKey(masterKey, "encryption", ["user-123", "document-456"]);
```

#### Advanced Key Derivation

```typescript
// Custom key length and hash algorithm
const sessionKey = await crypto.deriveKey(masterKey, "session", ["session-789"], {
  length: 16, // 128 bits
  hash: "SHA-512",
});

// Using salt for additional randomness
const salt = crypto.getRandomValues(new Uint8Array(16));
const key = await crypto.deriveKey(masterKey, "file-encryption", ["file.txt"], {
  salt,
});

// Hierarchical key derivation
const deptKey = await crypto.deriveKey(masterKey, "department", ["engineering"]);
const deptCryptoKey = await crypto.subtle.importKey("raw", deptKey, { name: "HKDF" }, false, [
  "deriveBits",
]);
const teamKey = await crypto.deriveKey(deptCryptoKey, "team", ["backend", "team-alpha"]);
```

## API Reference

### `crypto.deriveKey(base, purpose, path, options?)`

Securely derives a key using HKDF with configurable parameters.

#### Parameters

- **`base`** (`CryptoKey`): A strong CryptoKey with `deriveBits` capability
- **`purpose`** (`string`): Non-empty string describing the key's purpose (for domain separation)
- **`path`** (`Array<string | Uint8Array>`): Array of domain separation values
- **`options`** (`object`, optional):
  - **`length`** (`number`): Desired key length in bytes (1 to 255 × hash_length). Defaults to hash output size
  - **`hash`** (`"SHA-256" | "SHA-384" | "SHA-512"`): Hash algorithm. Defaults to `"SHA-256"`
  - **`salt`** (`Uint8Array`): Optional salt for additional randomness

#### Returns

`Promise<Uint8Array>` - The derived key

#### Throws

- `Error` - If inputs are invalid or key derivation fails
