import { Command } from "commander";
import { Address, Hex } from "viem";
import { createIdentity, getPolicy, setPolicy, deployIdentity } from "../utils/web3";
import { bundleManifestFileToDataUrl } from "../utils/bundle";

function shouldTreatAsRemote(url: string) {
  return url.startsWith("http://") || url.startsWith("https://");
}

function hasProtocol(url: string) {
  return /^[a-zA-Z]+:\/\//.test(url);
}

async function resolvePolicyUrl(urlOrPath: string): Promise<string> {
  if (!urlOrPath) {
    return urlOrPath;
  }

  if (urlOrPath.startsWith("data:")) {
    return urlOrPath;
  }

  if (
    shouldTreatAsRemote(urlOrPath) ||
    (hasProtocol(urlOrPath) && !urlOrPath.startsWith("file://"))
  ) {
    return urlOrPath;
  }

  const { dataUrl } = await bundleManifestFileToDataUrl(urlOrPath);
  console.log(`Bundled policy manifest into data URL: ${dataUrl.substring(0, 60)}...`);
  return dataUrl;
}

async function create(
  address: Address,
  options: { gatewayUrl: string; signer: Hex; policy?: string },
) {
  try {
    let policyUrl = "";

    if (options.policy) {
      policyUrl = await resolvePolicyUrl(options.policy);
    }

    const result = await createIdentity(options.gatewayUrl, address, options.signer, policyUrl);
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
    const policyUrl = await resolvePolicyUrl(urlOrPath);

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

async function deployCmd(options: {
  gatewayUrl: string;
  signer: Hex;
  allowNondeterministicAddress?: boolean;
  salt?: Hex;
}) {
  try {
    const result = await deployIdentity(options.gatewayUrl, options.signer, {
      allowNondeterministicAddress: options.allowNondeterministicAddress,
      salt: options.salt,
    });

    console.log(`Address: ${result.address}`);
    if (result.txHash !== "0x0000000000000000000000000000000000000000000000000000000000000000") {
      console.log(`Transaction Hash: ${result.txHash}`);
    }
    console.log(`Deterministic: ${result.isDeterministic}`);

    if (!result.isDeterministic) {
      console.warn("⚠️  Address is not deterministic across chains");
    }
  } catch (error) {
    console.error("Failed to deploy identity:", error);
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

  identity
    .command("deploy")
    .description("Deploy a new Identity contract")
    .requiredOption("--signer <private-key>", "Private key to sign the transaction")
    .option("--allow-nondeterministic-address", "Allow deployment without deterministic address")
    .option("--salt <hex>", "Custom salt for deterministic deployment (default: 0x0...0)")
    .action(deployCmd);
}
