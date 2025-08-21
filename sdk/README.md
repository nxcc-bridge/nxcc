# nXCC SDKs

Client libraries and command-line tools for interacting with nXCC platform.

## Packages

- **[@nxcc/cli](cli/)**: Command-line interface for worker deployment and management
- **[@nxcc/sdk](lib/)**: TypeScript SDK with cryptographic utilities and helper functions

## Quick Start

### CLI Usage

```bash
# Build from source and link globally
cd cli && pnpm build && pnpm link --global

# Use the CLI
nxcc init my-worker
```

### SDK Usage

```typescript
import { crypto } from "@nxcc/sdk";

// Create a master key
const masterKey = await crypto.subtle.importKey(
  "raw",
  crypto.getRandomValues(new Uint8Array(32)),
  { name: "HKDF" },
  false,
  ["deriveBits"],
);

// Derive keys for different purposes
const encryptionKey = await crypto.deriveKey(masterKey, "encryption", ["user-123", "document-456"]);
```

## Development

### Prerequisites

- Node.js 18+
- pnpm (for workspace management)

### Setup

```bash
# Install all dependencies
pnpm install

# Build all packages
pnpm build

# Run tests
pnpm test

# Link CLI globally for development
cd cli && pnpm link --global
```

### Workspace Commands

```bash
# Build specific package
pnpm --filter @nxcc/cli build
pnpm --filter @nxcc/sdk build

# Test specific package
pnpm --filter @nxcc/cli test
pnpm --filter @nxcc/sdk test

# Run CLI in development
pnpm --filter @nxcc/cli dev init my-test-worker
```

## Architecture

### CLI (`cli/`)

TypeScript-based command-line tool providing:

- Project scaffolding and templates
- Worker bundling and deployment
- Identity and policy management
- Integration with nXCC platform APIs

### SDK Library (`lib/`)

Reusable TypeScript library with:

- Cryptographic utilities (key derivation, HKDF)
- Type definitions and interfaces
- Worker development helpers
- Policy management utilities
