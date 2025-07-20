import {
  createPublicClient,
  createWalletClient,
  http,
  Hex,
  Address,
  Chain,
  defineChain,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import IdentityAbi from "../abi/Identity.json";

const identityContractAbi = IdentityAbi.abi;

function getChain(chainId: number, gatewayUrl: string): Chain {
  // TODO: Add more chains from viem/chains or allow custom chain definitions
  if (chainId === 31337) {
    return defineChain({
      id: 31337,
      name: "Anvil",
      nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
      rpcUrls: {
        default: { http: [gatewayUrl] },
      },
    });
  }
  throw new Error(`Chain with id ${chainId} not supported yet.`);
}

function getClients(gatewayUrl: string, chainId: number, signerKey?: Hex) {
  const chain = getChain(chainId, gatewayUrl);
  const transport = http(gatewayUrl);

  const publicClient = createPublicClient({ chain, transport });

  if (signerKey) {
    const account = privateKeyToAccount(signerKey);
    const walletClient = createWalletClient({
      account,
      chain,
      transport,
    });
    return { publicClient, walletClient };
  }

  return { publicClient };
}

export async function createIdentity(
  gatewayUrl: string,
  chainId: number,
  contractAddress: Address,
  signerKey: Hex,
  policyUrl: string,
) {
  const { publicClient, walletClient } = getClients(gatewayUrl, chainId, signerKey);
  if (!walletClient) {
    throw new Error("Signer key is required to create an identity");
  }

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
  chainId: number,
  contractAddress: Address,
  tokenId: string,
  policyUrl: string,
  signerKey: Hex,
) {
  const { publicClient, walletClient } = getClients(gatewayUrl, chainId, signerKey);
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

export async function getPolicy(
  gatewayUrl: string,
  chainId: number,
  contractAddress: Address,
  tokenId: string,
) {
  const { publicClient } = getClients(gatewayUrl, chainId);

  const policyUrl = await publicClient.readContract({
    address: contractAddress,
    abi: identityContractAbi,
    functionName: "tokenURI",
    args: [BigInt(tokenId)],
  });

  return policyUrl;
}
