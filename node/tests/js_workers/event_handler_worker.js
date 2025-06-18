import { ethers } from "ethers";

const USER_CONFIG = {}; // Populated in main fetch

async function handleLaunch(eventPayload, env) {
  console.log("Worker received Launch event. No action taken for this test.");
  return new Response(JSON.stringify({ message: "Launch event processed" }), {
    headers: { "Content-Type": "application/json" },
  });
}

async function sendUpdateStateTx(targetChainConfig, newValue, data) {
  // --- 1. Configuration and Pre-flight Checks ---
  // These checks prevent errors from undefined inputs before we start.
  if (!targetChainConfig || !USER_CONFIG) {
    const errorMsg =
      "Invalid configuration: targetChainConfig or USER_CONFIG is missing.";
    console.error(errorMsg);
    return new Response(JSON.stringify({ success: false, error: errorMsg }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }
  const { rpcUrl, contractAddress } = targetChainConfig;
  const { contractAbi, ethereumPrivateKey } = USER_CONFIG;

  console.log(
    `Worker sending updateState(${newValue.toString()}, "${data}") to ${contractAddress} on ${rpcUrl}`,
  );

  // --- 2. ABI Parsing ---
  let parsedAbi;
  try {
    parsedAbi = JSON.parse(contractAbi);
  } catch (e) {
    const errorMsg = `Failed to parse contract ABI. Ensure USER_CONFIG.contractAbi is valid JSON. Details: ${e.message}`;
    console.error(errorMsg, e.stack);
    return new Response(JSON.stringify({ success: false, error: errorMsg }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }

  // --- 3. Ethers.js Provider, Wallet, and Contract Initialization ---
  let contract;
  try {
    const provider = new ethers.JsonRpcProvider(rpcUrl);
    const wallet = new ethers.Wallet(ethereumPrivateKey, provider);
    contract = new ethers.Contract(contractAddress, parsedAbi, wallet);
  } catch (e) {
    const errorMsg = `Failed to initialize ethers objects. Check RPC URL, private key, or contract address. Details: ${e.message}`;
    console.error(errorMsg, e.stack);
    return new Response(JSON.stringify({ success: false, error: errorMsg }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }

  // --- 4. Transaction Submission ---
  let tx;
  try {
    // Note: The original function expected this to sometimes fail due to race conditions.
    tx = await contract.updateState(newValue, data);
    console.log(`Transaction sent: ${tx.hash}. Waiting for confirmation...`);
  } catch (e) {
    const errorMsg = `Failed to submit transaction to the node. This can happen in a race condition or due to node/gas issues. Details: ${e.message}`;
    console.error(errorMsg, e.stack);
    // As per the original logic, we treat this as a recoverable error, not a hard failure.
    return new Response(
      JSON.stringify({ success: false, error: errorMsg, txHash: null }),
      {
        headers: { "Content-Type": "application/json" },
      },
    );
  }

  // --- 5. Transaction Confirmation ---
  try {
    const receipt = await tx.wait(1); // Wait for 1 confirmation
    console.log(`Transaction confirmed: ${receipt.hash}`);
    return new Response(
      JSON.stringify({ success: true, txHash: receipt.hash }),
      {
        headers: { "Content-Type": "application/json" },
      },
    );
  } catch (e) {
    const errorMsg = `Transaction with hash ${tx.hash} failed to confirm (it may have been mined and reverted). Details: ${e.message}`;
    console.error(errorMsg, e.stack);
    return new Response(
      JSON.stringify({
        success: false,
        error: errorMsg,
        txHash: tx.hash, // Return the hash for debugging purposes
      }),
      {
        status: 500,
        headers: { "Content-Type": "application/json" },
      },
    );
  }
}

async function handleValueChanged(eventPayload, env) {
  const web3Log = eventPayload;
  const newValueFromEvent = BigInt(web3Log.topics[2]);
  const dataFromEvent = ethers.AbiCoder.defaultAbiCoder().decode(
    ["bytes"],
    web3Log.data,
  )[0];
  console.log(
    `Worker (handleValueChanged) processing event from chain 1. NewValue: ${newValueFromEvent.toString()}, Data: ${dataFromEvent}`,
  );
  return sendUpdateStateTx(
    USER_CONFIG.chain2,
    newValueFromEvent,
    dataFromEvent,
  );
}

async function handleOtherEvent(eventPayload, env) {
  const web3Log = eventPayload;
  const newValueFromEvent = BigInt(web3Log.topics[1]);
  const dataFromEvent = "0x";
  console.log(
    `Worker (handleOtherEvent) processing event from chain 2. NewValue: ${newValueFromEvent.toString()}`,
  );
  return sendUpdateStateTx(
    USER_CONFIG.chain1,
    newValueFromEvent,
    dataFromEvent,
  );
}

const handlers = {
  launch: handleLaunch,
  valueChanged: handleValueChanged,
  otherEvent: handleOtherEvent,
};

export default {
  async fetch(request, env, ctx) {
    // Populate global USER_CONFIG from env for handlers to use
    // This is a simple way; a more robust solution might pass env or specific config to handlers
    Object.assign(USER_CONFIG, env.USER_CONFIG || {});

    const { chain1, chain2, contractAbi, ethereumPrivateKey } = USER_CONFIG;

    if (
      !chain1 ||
      !chain2 ||
      !contractAbi ||
      !ethereumPrivateKey ||
      !chain1.rpcUrl ||
      !chain1.contractAddress ||
      !chain2.rpcUrl ||
      !chain2.contractAddress
    ) {
      console.error(
        `Missing or incomplete userdata fields. Received: ${JSON.stringify(USER_CONFIG)}`,
      );
      return new Response("Missing or incomplete userdata", { status: 500 });
    }

    const url = new URL(request.url);
    let handlerName = url.pathname.startsWith("/")
      ? url.pathname.substring(1)
      : url.pathname;

    // Default handler if path is "/" or empty
    if (!handlerName || handlerName === "") {
      // This case should ideally not happen if handlers are always specified.
      // Or, define a default handler. For now, let's assume a handler is always part of the path.
      console.warn(
        "Request path is empty or '/', no specific handler. This might be an issue.",
      );
      // For now, let's try to infer if it's a policy or default to a generic error.
      // The policy worker uses "_policy"
      if (handlerName === "_policy") {
        // This won't be hit by this worker, but for completeness
        console.error(
          "Policy handler invoked on event worker. This is unexpected.",
        );
        return new Response("Policy handler invoked on event worker", {
          status: 400,
        });
      }
      // Fallback for now, or could be an error.
      // handlerName = "default"; // Or some other logic
    }

    const vmInvocationPayload = await request.json(); // This is VmEventInvocation { handler, event_payload }
    console.log(
      `Worker received VmInvocationPayload for path ${url.pathname}: ${JSON.stringify(vmInvocationPayload)}`,
    );

    // The handler name from the VmInvocationPayload should match the one from the path.
    // For robustness, could verify: if (vmInvocationPayload.handler !== handlerName) { error }
    // However, the path is the primary dispatch mechanism from the VMM.

    const actualHandler =
      handlers[handlerName] || handlers[vmInvocationPayload.handler];
    if (actualHandler) {
      return actualHandler(vmInvocationPayload.event_payload, env);
    } else {
      console.error(
        `No handler found for '${handlerName}' or '${vmInvocationPayload.handler}'`,
      );
      return new Response(
        `No handler for ${handlerName || vmInvocationPayload.handler}`,
        { status: 404 },
      );
    }
  },
};
