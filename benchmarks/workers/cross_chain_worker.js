import {
  createPublicClient,
  http,
  createWalletClient,
  decodeEventLog,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";

const USER_CONFIG = {};

async function sendUpdateStateTx(targetChainConfig, newValue, data) {
  const { rpcUrl, contractAddress } = targetChainConfig;
  const { contractAbi, ethereumPrivateKey } = USER_CONFIG;

  const account = privateKeyToAccount(ethereumPrivateKey);
  const publicClient = createPublicClient({ transport: http(rpcUrl) });
  const walletClient = createWalletClient({
    account,
    transport: http(rpcUrl),
  });

  try {
    const { request } = await publicClient.simulateContract({
      address: contractAddress,
      abi: contractAbi,
      functionName: "updateState",
      args: [newValue, data],
      account,
    });
    const hash = await walletClient.writeContract(request);
    const receipt = await publicClient.waitForTransactionReceipt({ hash });
    return new Response(
      JSON.stringify({ success: true, txHash: receipt.transactionHash }),
      {
        headers: { "Content-Type": "application/json" },
      },
    );
  } catch (e) {
    console.error(`Transaction failed: ${e.message}`);
    return new Response(JSON.stringify({ success: false, error: e.message }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }
}

async function handleValueChanged(eventPayload, env) {
  const { contractAbi } = USER_CONFIG;
  const decodedLog = decodeEventLog({
    abi: contractAbi,
    eventName: "ValueChanged",
    data: eventPayload.data,
    topics: eventPayload.topics,
  });

  const { newValue, data } = decodedLog.args;
  return sendUpdateStateTx(USER_CONFIG.chain2, newValue, data);
}

const handlers = {
  valueChanged: handleValueChanged,
};

export default {
  async fetch(request, env, ctx) {
    Object.assign(USER_CONFIG, env.USER_CONFIG || {});
    if (
      !USER_CONFIG.chain1 ||
      !USER_CONFIG.chain2 ||
      !USER_CONFIG.contractAbi ||
      !USER_CONFIG.ethereumPrivateKey
    ) {
      return new Response("Missing or incomplete userdata", { status: 500 });
    }
    USER_CONFIG.contractAbi = JSON.parse(USER_CONFIG.contractAbi);

    const vmInvocationPayload = await request.json();
    const handler = handlers[vmInvocationPayload.handler];

    if (handler) {
      return handler(vmInvocationPayload.event_payload, env);
    } else {
      return new Response(`No handler for ${vmInvocationPayload.handler}`, {
        status: 404,
      });
    }
  },
};
