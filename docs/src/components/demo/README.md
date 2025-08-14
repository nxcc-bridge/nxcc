# NXCC NFT Cross-Chain Demo

This demo showcases how NXCC workers can handle cross-chain NFT transfers without requiring direct blockchain transactions from the frontend.

## Architecture

The demo consists of:

- **Vue Component** (`NFTDemo.vue`): Interactive UI demonstrating NFT movement between chains
- **Smart Contract** (`contracts/DemoNFT.sol`): ERC-721 NFT contract with cross-chain move functionality
- **NXCC Worker** (`worker/nft-mover.js`): Handles the cross-chain transfer logic in a trusted environment
- **Policy Configuration** (`policy/nft-demo.toml`): Defines worker permissions and event triggers

## How It Works

1. **User Interface**: The demo presents a simple UI with an NFT and a dropdown to select target chains
2. **Worker Communication**: Instead of making blockchain transactions, the frontend communicates with the NXCC worker via HTTP
3. **Cross-Chain Logic**: The worker handles the complexity of burning the NFT on the source chain and minting on the target chain
4. **Trusted Execution**: All cross-chain operations happen in the NXCC trusted execution environment

## Usage

The demo is accessible at `/demo` on the docs site. It's statically imported as a Vue component in the Astro page.

## Supported Chains

- Ethereum
- Polygon
- Arbitrum
- Optimism

## Files Structure

```
src/components/demo/
├── NFTDemo.vue        # Main Vue component with interactive demo
├── contracts/         # Smart contracts
│   └── DemoNFT.sol   # ERC-721 contract with cross-chain functionality
├── worker/           # NXCC worker code
│   └── nft-mover.js  # Cross-chain transfer logic
└── policy/           # NXCC policy configuration
    └── nft-demo.toml # Worker permissions and triggers
```

## Integration

The demo component is imported directly in `src/pages/demo.astro`:

```astro
---
import NFTDemo from "../components/demo/NFTDemo.vue";
---

<NFTDemo client:only="vue" />
```

This approach follows the same pattern as the previous demo implementation in commit 5d95327.
