# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Prerequisites and Setup

### Required Tools

```bash
# Install Foundry toolchain (forge, anvil, chisel)
cargo install --locked --git https://github.com/foundry-rs/foundry --force forge anvil chisel

# Install dependencies
forge soldeer install
```

### Soldeer Dependencies

This project uses [Soldeer](https://soldeer.xyz/) for dependency management instead of git submodules:

- `@openzeppelin-contracts@5.2.0`: OpenZeppelin contract library
- `forge-std@1.9.6`: Foundry testing framework

Dependencies are installed to `dependencies/` and mapped via `remappings.txt`.

## Build and Development Commands

### Core Commands

```bash
# Install/update dependencies
forge soldeer install

# Build contracts
forge build

# Format code (use appropriate formatter for the file types)
forge fmt

# Check formatting without making changes  
forge fmt --check

# Run tests
forge test

# Run tests with verbosity
forge test -vvv

# Run specific test
forge test --match-test testMintSuccess

# Run tests for specific contract
forge test --match-contract IdentityTest

# Deploy contracts (requires setup)
forge script --rpc-url <RPC_URL> --private-key <KEY> --broadcast
```

### Advanced Testing

```bash
# Generate gas report
forge test --gas-report

# Run fuzzing tests
forge test --fuzz-runs 1000

# Coverage analysis
forge coverage
```

## Project Structure

### Smart Contracts (`src/`)

- **Identity.sol**: Machine Identity NFT contract implementing ERC-721 with EIP-4907 user roles
  - Creates machine identities as NFTs
  - Supports metadata URIs for off-chain policies
  - Implements time-limited user roles via EIP-4907
  - Includes batch metadata retrieval functionality

### Tests (`test/`)

- **Identity.t.sol**: Comprehensive test suite for Identity contract
  - Tests minting, burning, transfers
  - Tests policy URL updates
  - Tests EIP-4907 user role functionality
  - Tests access controls and edge cases

### Configuration Files

- **foundry.toml**: Foundry project configuration with Soldeer integration
- **soldeer.lock**: Locked dependency versions with checksums
- **remappings.txt**: Import path remappings for dependencies

## Architecture Overview

This is the Smart Contract component of the nXCC (Network eXecutable Cross-Chain) platform. The contracts directory focuses on blockchain-based identity management.

### Key Concepts

**Machine Identity NFTs**: Each machine identity is represented as an ERC-721 NFT that:

- Can be minted by any address to create a new identity
- Stores a metadata URI pointing to off-chain policy definitions
- Supports secure transfer and burning operations
- Implements EIP-4907 for time-limited user role assignments

**Policy Management**: Identity metadata URIs typically point to policy documents that define:

- Access controls and permissions
- Execution policies for the machine
- Cross-chain operation parameters

**Integration with nXCC Platform**: These identity contracts integrate with the broader nXCC system:

- Identity NFTs represent compute nodes in the network
- Policy URLs define worker execution policies
- User roles can grant temporary access to identity operations

### Smart Contract Patterns

- Uses OpenZeppelin's battle-tested contract implementations
- Follows ERC standards (ERC-721, EIP-4907) for interoperability
- Implements proper access controls via `_checkAuthorized`
- Emits standard events for off-chain monitoring
- Supports interface detection via ERC-165

## Development Workflow

### Adding New Contracts

1. Create contract in `src/` directory
2. Add corresponding test file in `test/`
3. Update remappings if needed
4. Run `forge build` to compile
5. Run `forge test` to verify tests pass

### Modifying Identity Contract

1. Always read the existing contract first to understand current functionality
2. Follow OpenZeppelin patterns for security
3. Update tests to cover new functionality
4. Verify gas usage with `forge test --gas-report`
5. Format code using appropriate formatter for the file types
6. Run full test suite before committing

### Code Quality

After making changes, always run:

```bash
# Format code (use appropriate formatter for the component/file types)
forge fmt

# Run tests
forge test

# Optional: Check gas usage
forge test --gas-report
```

### Testing Best Practices

- Use descriptive test names (e.g., `testCannotBurnIfNotOwnerOrOperator`)
- Test both success and failure cases
- Use `vm.expectRevert` for expected failures with specific error selectors
- Test edge cases like self-transfers and expired roles
- Use `vm.prank` and `vm.startPrank` for access control testing
