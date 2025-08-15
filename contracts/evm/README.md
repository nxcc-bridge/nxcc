# nXCC Smart Contracts

Machine identity NFTs with deterministic cross-chain deployment for the nXCC platform.

## Overview

This directory contains Solidity smart contracts that provide blockchain-based identity management for nXCC nodes. The core functionality includes:

- **[Identity.sol](src/Identity.sol)**: ERC-721 NFT contract representing machine identities
- **EIP-4907 User Roles**: Time-limited access delegation
- **Policy Management**: On-chain metadata URIs for off-chain policy documents
- **CREATE2 Deployment**: Deterministic addresses across all EVM chains

## Prerequisites

```bash
# Install Foundry toolchain
cargo install --locked --git https://github.com/foundry-rs/foundry --force forge anvil chisel
```

### Setup

```bash
# Install dependencies via Soldeer
forge soldeer install

## Build

```bash
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

The Identity contract uses CREATE2 for deterministic addresses across all chains.

**Deterministic Address:** `0x843f604F71dDaaaE82a82551d6b19571E6C6E23A`
- **Salt:** `0x0` (zero bytes32)
- **Compiler:** Solc 0.8.28 with full release optimizations
- **Deployer:** Arachnid's Deterministic Deployment Proxy (`0x4e59b44847b379578588920cA78FbF26c0B4956C`)
- **Deployment Method:** CREATE2 via DDP (no specific deployer address required)

### Quick Start: Deploy to a New Chain

1. **Set environment variables:**
```bash
export PRIVATE_KEY=your_private_key_here
export RPC_URL=https://your-rpc-endpoint.com
```

2. **Deploy to the new chain:**
```bash
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL --broadcast --verify
```

The Identity contract will be deployed to the **exact same address** on every chain.

### How It Works

The deployment uses Foundry's built-in CREATE2 support with deterministic salts:

1. **Identity** contract is deployed directly using CREATE2 with a fixed salt
2. Same deployer address + same salt = same contract address on any EVM chain

This ensures the Identity contract has deterministic addresses across all chains.

### Custom Salt (Optional)

To use a custom salt for the Identity deployment:

```bash
export IDENTITY_SALT=0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL --broadcast
```

### Predicting Addresses

You can predict contract addresses before deployment:

```bash
# Simulate deployment to see addresses
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL
```

### Verification on Block Explorers

The script automatically attempts to verify contracts. For manual verification:

```bash
# Verify Identity contract
forge verify-contract <IDENTITY_ADDRESS> src/Identity.sol:Identity --rpc-url $RPC_URL
```

### Supported Networks

The deterministic deployment works on any EVM-compatible chain. The same addresses will be used on:

- Ethereum Mainnet
- Polygon
- Arbitrum
- Optimism
- Base
- Any other EVM chain

### Advanced Usage

**Check if already deployed:**
```bash
# The script will detect existing deployments and skip them
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL
```

**Deploy to testnet first:**
```bash
# Test on Sepolia
export RPC_URL=https://sepolia.infura.io/v3/YOUR_KEY
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL --broadcast
```

**Use with different wallets:**
```bash
# Deploy with Ledger
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL --ledger --sender 0xYourAddress --broadcast

# Deploy with keystore
forge script script/DeployIdentity.s.sol --rpc-url $RPC_URL --keystore /path/to/keystore --broadcast
```
