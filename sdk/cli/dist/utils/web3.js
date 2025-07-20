"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.createIdentity = createIdentity;
exports.setPolicy = setPolicy;
exports.getPolicy = getPolicy;
const viem_1 = require("viem");
const accounts_1 = require("viem/accounts");
const Identity_json_1 = __importDefault(require("../abi/Identity.json"));
const identityContractAbi = Identity_json_1.default.abi;
function getChain(chainId, gatewayUrl) {
    // TODO: Add more chains from viem/chains or allow custom chain definitions
    if (chainId === 31337) {
        return (0, viem_1.defineChain)({
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
function getClients(gatewayUrl, chainId, signerKey) {
    const chain = getChain(chainId, gatewayUrl);
    const transport = (0, viem_1.http)(gatewayUrl);
    const publicClient = (0, viem_1.createPublicClient)({ chain, transport });
    if (signerKey) {
        const account = (0, accounts_1.privateKeyToAccount)(signerKey);
        const walletClient = (0, viem_1.createWalletClient)({
            account,
            chain,
            transport,
        });
        return { publicClient, walletClient };
    }
    return { publicClient };
}
async function createIdentity(gatewayUrl, chainId, contractAddress, signerKey, policyUrl) {
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
    const transferEvent = receipt.logs.find((log) => log.topics[0] === "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
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
async function setPolicy(gatewayUrl, chainId, contractAddress, tokenId, policyUrl, signerKey) {
    const { publicClient, walletClient } = getClients(gatewayUrl, chainId, signerKey);
    if (!walletClient)
        throw new Error("Signer key is required to set a policy");
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
async function getPolicy(gatewayUrl, chainId, contractAddress, tokenId) {
    const { publicClient } = getClients(gatewayUrl, chainId);
    const policyUrl = await publicClient.readContract({
        address: contractAddress,
        abi: identityContractAbi,
        functionName: "tokenURI",
        args: [BigInt(tokenId)],
    });
    return policyUrl;
}
