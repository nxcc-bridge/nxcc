import {
  createPublicClient,
  createWalletClient,
  http,
  Hex,
  Address,
  keccak256,
  getCreate2Address,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { anvil } from "viem/chains";
import IdentityAbi from "../abi/Identity.json";
import { IDENTITY_ABI, IDENTITY_BYTECODE, DDP_DEPLOYER, DEFAULT_SALT } from "../contracts/identity";

const identityContractAbi = IdentityAbi.abi;

function getClients(gatewayUrl: string, signerKey?: Hex) {
  const transport = http(gatewayUrl);

  const publicClient = createPublicClient({
    chain: anvil,
    transport,
  });

  if (signerKey) {
    const account = privateKeyToAccount(signerKey);
    const walletClient = createWalletClient({
      account,
      chain: anvil,
      transport,
    });
    return { publicClient, walletClient };
  }

  return { publicClient };
}

export async function createIdentity(
  gatewayUrl: string,
  contractAddress: Address,
  signerKey: Hex,
  policyUrl: string,
) {
  const { publicClient, walletClient } = getClients(gatewayUrl, signerKey);
  if (!walletClient) {
    throw new Error("Signer key is required to create an identity");
  }

  const chainId = await publicClient.getChainId();

  const { request } = await publicClient.simulateContract({
    address: contractAddress,
    abi: identityContractAbi,
    functionName: "mint",
    args: [policyUrl],
    account: walletClient.account,
  });

  const hash = await walletClient.writeContract(request);
  const receipt = await publicClient.waitForTransactionReceipt({ hash });

  const transferEvent = receipt.logs.find(
    (log) => log.topics[0] === "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
  );

  if (!transferEvent || !transferEvent.topics[3]) {
    throw new Error("Could not find tokenId in mint transaction receipt");
  }

  const tokenId = BigInt(transferEvent.topics[3]).toString();

  return {
    chain: chainId,
    address: contractAddress,
    id: tokenId,
    txHash: hash,
  };
}

export async function setPolicy(
  gatewayUrl: string,
  contractAddress: Address,
  tokenId: string,
  policyUrl: string,
  signerKey: Hex,
) {
  const { publicClient, walletClient } = getClients(gatewayUrl, signerKey);
  if (!walletClient) throw new Error("Signer key is required to set a policy");

  const { request } = await publicClient.simulateContract({
    address: contractAddress,
    abi: identityContractAbi,
    functionName: "setPolicyURL",
    args: [BigInt(tokenId), policyUrl],
    account: walletClient.account,
  });

  const hash = await walletClient.writeContract(request);
  await publicClient.waitForTransactionReceipt({ hash });
  return hash;
}

export async function getPolicy(gatewayUrl: string, contractAddress: Address, tokenId: string) {
  const { publicClient } = getClients(gatewayUrl);

  const policyUrl = await publicClient.readContract({
    address: contractAddress,
    abi: identityContractAbi,
    functionName: "tokenURI",
    args: [BigInt(tokenId)],
  });

  return policyUrl;
}

export async function deployIdentity(
  gatewayUrl: string,
  signerKey: Hex,
  options: {
    allowNondeterministicAddress?: boolean;
    salt?: Hex;
  } = {},
): Promise<{ address: Address; txHash: Hex; isDeterministic: boolean }> {
  const { publicClient, walletClient } = getClients(gatewayUrl, signerKey);
  if (!walletClient) {
    throw new Error("Signer key is required to deploy an identity");
  }

  const salt = options.salt || DEFAULT_SALT;

  // Check if DDP is available on this chain
  const ddpCode = await publicClient.getCode({ address: DDP_DEPLOYER });
  const hasDDP = ddpCode && ddpCode !== "0x" && ddpCode.length > 2;

  if (!hasDDP && !options.allowNondeterministicAddress) {
    throw new Error(
      `Deterministic Deployment Proxy not found at ${DDP_DEPLOYER}. ` +
        "Use --allow-nondeterministic-address to deploy with non-deterministic address.",
    );
  }

  let deployTxHash: Hex;
  let deployedAddress: Address;

  if (hasDDP) {
    // Use deterministic deployment via DDP
    const initcode = IDENTITY_BYTECODE;

    // Compute the deterministic address first
    deployedAddress = getCreate2Address({
      from: DDP_DEPLOYER,
      salt,
      bytecodeHash: keccak256(initcode),
    });

    // Check if contract already exists at this address
    const existingCode = await publicClient.getCode({ address: deployedAddress });
    if (existingCode && existingCode !== "0x" && existingCode.length > 2) {
      // Contract already exists, return early
      return {
        address: deployedAddress,
        txHash: "0x0000000000000000000000000000000000000000000000000000000000000000" as Hex,
        isDeterministic: true,
      };
    }

    // DDP expects: salt (32 bytes) + initcode
    const deployData = `${salt}${initcode.slice(2)}` as Hex;

    deployTxHash = await walletClient.sendTransaction({
      to: DDP_DEPLOYER,
      data: deployData,
    });
  } else {
    // Use regular deployment (non-deterministic)
    deployTxHash = await walletClient.deployContract({
      abi: IDENTITY_ABI,
      bytecode: IDENTITY_BYTECODE,
    });

    const receipt = await publicClient.waitForTransactionReceipt({
      hash: deployTxHash,
    });

    deployedAddress = receipt.contractAddress!;
  }

  // Wait for deployment transaction
  await publicClient.waitForTransactionReceipt({ hash: deployTxHash });

  return {
    address: deployedAddress,
    txHash: deployTxHash,
    isDeterministic: !!hasDDP,
  };
}
