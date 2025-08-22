---
title: Identities & Policies
description: A deep dive into nXCC's on-chain identity and programmable policy system.
---

Identities and Policies are the foundation of nXCC's security model. They provide a verifiable, on-chain root of trust for all off-chain operations, ensuring that only authorized workers can access sensitive data and perform actions.

## Identities: On-Chain Root of Trust

An nXCC Identity is a standard **ERC-721 NFT** deployed on an EVM-compatible blockchain. This provides several key advantages:

- **Ownership**: Each identity has a clear owner, which can be an Externally Owned Account (EOA) or a smart contract like a multisig or DAO.
- **Verifiability**: Anyone can verify the existence and ownership of an identity on-chain.
- **Composability**: As a standard NFT, identities can be managed with existing wallets, marketplaces, and smart contract tooling.

The core of the identity is the `Identity.sol` smart contract. Its most important function is `setPolicyURL`, which allows the owner of the NFT to set its `tokenURI`. In nXCC, this URI points directly to the **Policy** that governs the identity.

### Managing Identities

You can manage identities using the [`@nxcc/cli`](/docs/reference/cli) or any standard smart contract interaction tool like Foundry's `cast`.

**Example: Transferring an Identity with `cast`**

```bash
# Assuming environment variables are set
cast send $IDENTITY_CONTRACT_ADDRESS "safeTransferFrom(address,address,uint256)" \
  $OWNER_ADDRESS $RECIPIENT_ADDRESS $IDENTITY_ID \
  --private-key $SIGNER_PK --rpc-url $RPC_URL
```

## Policies: Programmable Access Control

A Policy is a special type of worker that acts as a programmable gatekeeper for an Identity's secrets. When a process (like another worker) wants to access an identity's secrets, an nXCC node fetches the policy from the URL stored on-chain and executes it inside a secure TEE.

### How Policies Work

A policy is a stateless worker that must implement a `fetch` handler. It receives a `POST` request containing a JSON array of `PolicyExecutionRequest` contexts. For each context, it must return a boolean decision (`true` for allow, `false` for deny).

**Example: A Simple Allow-All Policy**

This policy approves every request it receives.

```typescript
// policy.ts
export default {
  async fetch(request: Request): Promise<Response> {
    if (request.method !== "POST") {
      return new Response("Invalid method", { status: 405 });
    }
    try {
      const contexts = await request.json();
      if (!Array.isArray(contexts)) {
        return new Response("Expected an array of contexts", { status: 400 });
      }
      // This is a simple allow-all policy. It returns `true` for every request.
      const decisions = contexts.map(() => true);
      return new Response(JSON.stringify(decisions), {
        headers: { "Content-Type": "application/json" },
      });
    } catch (e) {
      return new Response("Invalid JSON payload", { status: 400 });
    }
  },
};
```

### The `PolicyExecutionRequest` Context

The real power of policies comes from the context they receive. The `PolicyExecutionRequest` object contains verifiable information about the requester that the policy can use to make a decision.

- `secret_ids`: An array of `SecretId`s being requested.
- `consumer`: Information about the worker requesting access, including the hash of its code bundle (`bundle_hash`).
- `env_report`: The TEE attestation report from the node making the request. This report cryptographically proves that the requester is running genuine nXCC software in a secure, un-tampered environment.

**Example: An Attestation-Based Policy**

A more secure policy would inspect the `env_report` to ensure the request is coming from a trusted TEE.

```typescript
// A more secure policy that checks the TEE measurement.
export default {
  async fetch(request: Request): Promise<Response> {
    const contexts: PolicyExecutionRequest[] = await request.json();

    const TRUSTED_MEASUREMENT = "0x..."; // The expected TEE measurement hash

    const decisions = contexts.map((ctx) => {
      // Check if the requester's TEE measurement matches our trusted value.
      return ctx.env_report.attestation.measurement === TRUSTED_MEASUREMENT;
    });

    return new Response(JSON.stringify(decisions));
  },
};
```

By combining on-chain identity NFTs with off-chain, TEE-executed policy workers, nXCC provides a flexible and powerful framework for building secure, decentralized applications.
