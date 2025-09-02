#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

// Path to the contracts build output
const contractsPath = path.resolve(
  __dirname,
  "../../../contracts/evm/out/Identity.sol/Identity.json",
);
const outputPath = path.resolve(__dirname, "../src/contracts/identity.ts");

try {
  // Read the compiled contract JSON
  const contractData = JSON.parse(fs.readFileSync(contractsPath, "utf8"));

  // Extract the necessary data
  const abi = contractData.abi;
  const bytecode = contractData.bytecode.object;

  // Create the TypeScript file content
  const tsContent = `// Auto-generated file - do not edit manually
// Generated from contracts/evm/out/Identity.sol/Identity.json

export const IDENTITY_ABI = ${JSON.stringify(abi, null, 2)} as const;

export const IDENTITY_BYTECODE = "${bytecode}" as const;

// Arachnid's Deterministic Deployment Proxy address
export const DDP_DEPLOYER = "0x4e59b44847b379578588920cA78FbF26c0B4956C" as const;

// Default salt for consistent deployment addresses across chains
export const DEFAULT_SALT = "0x0000000000000000000000000000000000000000000000000000000000000000" as const;
`;

  // Ensure the output directory exists
  const outputDir = path.dirname(outputPath);
  if (!fs.existsSync(outputDir)) {
    fs.mkdirSync(outputDir, { recursive: true });
  }

  // Write the TypeScript file
  fs.writeFileSync(outputPath, tsContent);

  console.log("✅ Contract data extracted successfully to", outputPath);
} catch (error) {
  console.error("❌ Failed to extract contract data:", error.message);
  process.exit(1);
}
