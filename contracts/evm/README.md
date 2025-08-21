# nXCC Smart Contracts

Machine identity NFTs and on-chain policy management for the nXCC platform.

## Overview

This directory contains Solidity smart contracts that provide blockchain-based identity management for nXCC nodes. The core functionality includes:

- **[Identity.sol](src/Identity.sol)**: ERC-721 NFT contract representing machine identities
- **EIP-4907 User Roles**: Time-limited access delegation
- **Policy Management**: On-chain metadata URIs for off-chain policy documents

## Quick Start

### Prerequisites

```bash
# Install Foundry toolchain
cargo install --locked --git https://github.com/foundry-rs/foundry --force forge anvil chisel
```

### Setup

```bash
# Install dependencies via Soldeer
forge soldeer install

# Build contracts
forge build

# Run tests
forge test -vvv
```

## Architecture

### Identity Contract

The `Identity` contract implements machine identity management with:

```solidity
// Mint new identity NFT to caller
function mint(string calldata policyURL)
    external returns (uint256 tokenId)

// Update policy metadata (owner only)
function setPolicyURL(uint256 tokenId, string calldata newPolicyURL)
    external

// EIP-4907 user role delegation
function setUser(uint256 tokenId, address user, uint64 expires)
    external

// Burn identity (owner/approved only)
function burn(uint256 tokenId) external
```

### Token Details

- **Name**: "nXCC Identity"
- **Symbol**: "nxccid"
- **Standards**: ERC-721, ERC-721URIStorage, ERC-721Burnable, EIP-4907
- **Metadata**: Stores policy URLs as token URIs
- **User Roles**: Time-limited delegation via EIP-4907

## Development

### Testing

```bash
# Run all tests
forge test

# Test specific contract
forge test --match-contract IdentityTest

# Generate gas report
forge test --gas-report

# Fuzz testing
forge test --fuzz-runs 1000
```

### Adding New Contracts

1. Create contract in `src/`
2. Add comprehensive test in `test/`
3. Update imports and remappings if needed
4. Verify with `forge build && forge test`

### Dependencies

Uses [Soldeer](https://soldeer.xyz/) for dependency management:

- `@openzeppelin-contracts@5.2.0`: Battle-tested contract libraries
- `forge-std@1.9.6`: Foundry testing framework

Dependencies install to `dependencies/` with mappings in `remappings.txt`.

## Deployment

The Identity contract can be deployed to any EVM-compatible network:

```bash
# Deploy with Foundry
forge script --rpc-url <RPC_URL> --private-key <PRIVATE_KEY> --broadcast

# Or deploy programmatically with specific constructor args
forge create src/Identity.sol:Identity --rpc-url <RPC_URL> --private-key <PRIVATE_KEY>
```

For production deployments and network configuration, see the [infrastructure directory](../../infra/).
