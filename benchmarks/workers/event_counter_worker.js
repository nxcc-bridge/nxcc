import {
  createPublicClient,
  http,
  createWalletClient,
  decodeEventLog,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";

const USER_CONFIG = {};

async function sendUpdateStateTx(newValue, data) {
  const { rpcUrl, contractAddress } = USER_CONFIG.chain1;
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

    // Actually wait for the transaction to complete
    const hash = await walletClient.writeContract(request);
    const receipt = await publicClient.waitForTransactionReceipt({
      hash,
      pollingInterval: 10,
      confirmations: 0,
      retryDelay: 10,
    });

    return { success: true, txHash: receipt.transactionHash };
  } catch (e) {
    console.error(`Transaction failed: ${e.message}`);
    throw e; // Re-throw to handle in the main handler
  }
}

async function handleValueChanged(eventPayload) {
  const { contractAbi } = USER_CONFIG;
  const decodedLog = decodeEventLog({
    abi: contractAbi,
    eventName: "ValueChanged",
    data: eventPayload.data,
    topics: eventPayload.topics,
  });

  const { newValue, data } = decodedLog.args;
  return await sendUpdateStateTx(newValue, data);
}

const handlers = {
  valueChanged: handleValueChanged,
};

export default {
  async fetch(request, env, ctx) {
    Object.assign(USER_CONFIG, env.USER_CONFIG || {});
    if (
      !USER_CONFIG.chain1 ||
      !USER_CONFIG.contractAbi ||
      !USER_CONFIG.ethereumPrivateKey
    ) {
      return new Response("Missing or incomplete userdata", { status: 500 });
    }
    USER_CONFIG.contractAbi = JSON.parse(USER_CONFIG.contractAbi);

    const vmInvocationPayload = await request.json();
    const handler = handlers[vmInvocationPayload.handler];

    if (handler) {
      try {
        const result = await handler(vmInvocationPayload.event_payload);
        return new Response(JSON.stringify(result), {
          headers: { "Content-Type": "application/json" },
        });
      } catch (e) {
        return new Response(
          JSON.stringify({ success: false, error: e.message }),
          {
            status: 500,
            headers: { "Content-Type": "application/json" },
          },
        );
      }
    } else {
      return new Response(`No handler for ${vmInvocationPayload.handler}`, {
        status: 404,
      });
    }
  },
};
