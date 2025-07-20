import * as fs from "fs/promises";
import * as path from "path";
import { Command } from "commander";
import { Address, Hex } from "viem";
import { createIdentity, getPolicy, setPolicy } from "../utils/web3";

async function create(
  chain: string,
  address: Address,
  options: { gatewayUrl: string; signer: Hex },
) {
  try {
    const chainId = parseInt(chain, 10);
    const result = await createIdentity(options.gatewayUrl, chainId, address, options.signer, "");
    console.log("Identity created successfully:");
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    console.error("Failed to create identity:", error);
    process.exit(1);
  }
}

async function setPolicyCmd(
  chain: string,
  address: Address,
  id: string,
  urlOrPath: string,
  options: { gatewayUrl: string; signer: Hex },
) {
  try {
    let policyUrl = urlOrPath;
    if (!urlOrPath.startsWith("http") && !urlOrPath.startsWith("data:")) {
      const bundlePath = path.resolve(process.cwd(), urlOrPath);
      const bundleContent = await fs.readFile(bundlePath);
      const bundleB64 = bundleContent.toString("base64");
      policyUrl = `data:application/json;base64,${bundleB64}`;
      console.log(`Using data URL for policy: ${policyUrl.substring(0, 50)}...`);
    }

    const chainId = parseInt(chain, 10);
    const txHash = await setPolicy(
      options.gatewayUrl,
      chainId,
      address,
      id,
      policyUrl,
      options.signer,
    );
    console.log(`Policy set successfully. Transaction hash: ${txHash}`);
  } catch (error) {
    console.error("Failed to set policy:", error);
    process.exit(1);
  }
}

async function getPolicyCmd(
  chain: string,
  address: Address,
  id: string,
  options: { gatewayUrl: string },
) {
  try {
    const chainId = parseInt(chain, 10);
    const policyUrl = await getPolicy(options.gatewayUrl, chainId, address, id);
    console.log("Policy URL:", policyUrl);
  } catch (error) {
    console.error("Failed to get policy:", error);
    process.exit(1);
  }
}

export function identitySubcommand(program: Command) {
  const identity = program
    .command("identity")
    .description("Interact with an identity")
    .requiredOption("--gateway-url <url>", "Web3 gateway URL", "http://localhost:8545");

  identity
    .command("create <chain> <address>")
    .description("Create a new identity")
    .requiredOption("--signer <private-key>", "Private key to sign the transaction")
    .action(create);

  identity
    .command("set-policy <chain> <address> <id> <url-or-path-to-bundle>")
    .description("Set the policy worker for an identity")
    .requiredOption("--signer <private-key>", "Private key to sign the transaction")
    .action(setPolicyCmd);

  identity
    .command("get-policy <chain> <address> <id>")
    .description("Get the policy worker URL for an identity")
    .action(getPolicyCmd);
}
