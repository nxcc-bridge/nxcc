import * as fs from "fs/promises";
import * as path from "path";
import { Command } from "commander";
import { Address, Hex } from "viem";
import { createIdentity, getPolicy, setPolicy } from "../utils/web3";

async function create(
  address: Address,
  options: { gatewayUrl: string; signer: Hex; policy?: string },
) {
  try {
    let policyUrl = "";

    if (options.policy) {
      if (!options.policy.startsWith("http") && !options.policy.startsWith("data:")) {
        const bundlePath = path.resolve(process.cwd(), options.policy);
        const bundleContent = await fs.readFile(bundlePath);
        const bundleB64 = bundleContent.toString("base64");
        policyUrl = `data:application/json;base64,${bundleB64}`;
        console.log(`Using data URL for policy: ${policyUrl.substring(0, 50)}...`);
      } else {
        policyUrl = options.policy;
      }
    }

    const result = await createIdentity(options.gatewayUrl, address, options.signer, policyUrl);
    console.log("Identity created successfully:");
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    console.error("Failed to create identity:", error);
    process.exit(1);
  }
}

async function setPolicyCmd(
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

    const txHash = await setPolicy(options.gatewayUrl, address, id, policyUrl, options.signer);
    console.log(`Policy set successfully. Transaction hash: ${txHash}`);
  } catch (error) {
    console.error("Failed to set policy:", error);
    process.exit(1);
  }
}

async function getPolicyCmd(address: Address, id: string, options: { gatewayUrl: string }) {
  try {
    const policyUrl = await getPolicy(options.gatewayUrl, address, id);
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
    .command("create <address>")
    .description("Create a new identity")
    .requiredOption("--signer <private-key>", "Private key to sign the transaction")
    .option("--policy <url-or-path-to-bundle>", "Policy URL or path to policy bundle file")
    .action(create);

  identity
    .command("set-policy <address> <id> <url-or-path-to-bundle>")
    .description("Set the policy worker for an identity")
    .requiredOption("--signer <private-key>", "Private key to sign the transaction")
    .action(setPolicyCmd);

  identity
    .command("get-policy <address> <id>")
    .description("Get the policy worker URL for an identity")
    .action(getPolicyCmd);
}
