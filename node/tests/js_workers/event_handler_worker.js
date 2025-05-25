import { ethers } from "ethers";

const USER_CONFIG = {}; // Populated in main fetch

async function handleLaunch(eventPayload, env) {
  console.log(
    "Worker received Launch event. No action taken for this test.",
  );
  return new Response(
    JSON.stringify({ message: "Launch event processed" }),
    {
      headers: { "Content-Type": "application/json" },
    },
  );
}

async function handleValueChanged(eventPayload, env) {
  // Userdata is already in USER_CONFIG
  const { rpcUrl, contractAddress, contractAbi, ethereumPrivateKey } =
    USER_CONFIG;

  const web3Log = eventPayload; // The event_payload part of VmEventInvocation
  // Assuming ValueChanged(uint256 indexed oldValue, uint256 indexed newValue, bytes data)
  // topics[0] = event signature
  // topics[1] = oldValue (ignored by this worker)
  // topics[2] = newValue
  // data = data field (non-indexed)

  const newValueFromEvent = BigInt(web3Log.topics[2]); // topics are 0x-prefixed hex strings
  const dataFromEvent = ethers.AbiCoder.defaultAbiCoder().decode(
    ["bytes"],
    web3Log.data,
  )[0];

  console.log(
    `Worker (handleValueChanged) processing Web3Log. NewValue: ${newValueFromEvent.toString()}, Data: ${dataFromEvent}`,
  );

  try {
    const provider = new ethers.JsonRpcProvider(rpcUrl);
    const wallet = new ethers.Wallet(ethereumPrivateKey, provider);
    const contract = new ethers.Contract(
      contractAddress,
      JSON.parse(contractAbi),
      wallet,
    );

    console.log(
      `Attempting to call updateState(${newValueFromEvent.toString()}, "${dataFromEvent}") on ${contractAddress}`,
    );
    let tx;
    try {
      tx = await contract.updateState(newValueFromEvent, dataFromEvent);
    } catch (e) {
      // The nodes race to produce the result. One will fail because the tx was already submitted.
      console.error("Failed to submit transaction:", e);
      return new Response(JSON.stringify({ success: false, txHash: null }), {
        headers: { "Content-Type": "application/json" },
      });
    }
    console.log(`Transaction sent: ${tx.hash}. Waiting for confirmation...`);
    const receipt = await tx.wait(1); // Wait for 1 confirmation
    console.log(`Transaction confirmed: ${receipt.hash}`);

    return new Response(
      JSON.stringify({ success: true, txHash: receipt.hash }),
      {
        headers: { "Content-Type": "application/json" },
      },
    );
  } catch (e) {
    console.error(`Error sending transaction: ${e.message}`, e.stack);
    return new Response(
      JSON.stringify({ success: false, error: e.message, stack: e.stack }),
      {
        status: 500,
        headers: { "Content-Type": "application/json" },
      },
    );
  }
}

const handlers = {
  launch: handleLaunch,
  valueChanged: handleValueChanged,
  // Add other handlers here
};

export default {
  async fetch(request, env, ctx) {
    // Populate global USER_CONFIG from env for handlers to use
    // This is a simple way; a more robust solution might pass env or specific config to handlers
    Object.assign(USER_CONFIG, env.USER_CONFIG || {});

    const { rpcUrl, contractAddress, contractAbi, ethereumPrivateKey } =
      USER_CONFIG;

    if (!rpcUrl || !contractAddress || !contractAbi || !ethereumPrivateKey) {
      const missing = [
        rpcUrl ? null : "rpcUrl",
        contractAddress ? null : "contractAddress",
        contractAbi ? null : "contractAbi",
        ethereumPrivateKey ? null : "ethereumPrivateKey",
      ]
        .filter(Boolean)
        .join(", ");
      console.error(`Missing userdata fields: ${missing}`);
      return new Response(`Missing userdata: ${missing}`, { status: 500 });
    }

    const url = new URL(request.url);
    let handlerName = url.pathname.startsWith("/")
      ? url.pathname.substring(1)
      : url.pathname;
    
    // Default handler if path is "/" or empty
    if (!handlerName || handlerName === "") {
        // This case should ideally not happen if handlers are always specified.
        // Or, define a default handler. For now, let's assume a handler is always part of the path.
        console.warn("Request path is empty or '/', no specific handler. This might be an issue.");
        // For now, let's try to infer if it's a policy or default to a generic error.
        // The policy worker uses "_policy"
        if (handlerName === "_policy") { // This won't be hit by this worker, but for completeness
             console.error("Policy handler invoked on event worker. This is unexpected.");
             return new Response("Policy handler invoked on event worker", { status: 400 });
        }
        // Fallback for now, or could be an error.
        // handlerName = "default"; // Or some other logic
    }

    const vmInvocationPayload = await request.json(); // This is VmEventInvocation { handler, event_payload }
    console.log(`Worker received VmInvocationPayload for path ${url.pathname}: ${JSON.stringify(vmInvocationPayload)}`);

    // The handler name from the VmInvocationPayload should match the one from the path.
    // For robustness, could verify: if (vmInvocationPayload.handler !== handlerName) { error }
    // However, the path is the primary dispatch mechanism from the VMM.
    
    const actualHandler = handlers[handlerName] || handlers[vmInvocationPayload.handler];
    if (actualHandler) {
      return actualHandler(vmInvocationPayload.event_payload, env);
    } else {
      console.error(`No handler found for '${handlerName}' or '${vmInvocationPayload.handler}'`);
      return new Response(`No handler for ${handlerName || vmInvocationPayload.handler}`, { status: 404 });
    }
  },
};
