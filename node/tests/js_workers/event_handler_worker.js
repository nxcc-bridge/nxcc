import { ethers } from "ethers";

export default {
  async fetch(request, env, ctx) {
    // Userdata should contain: rpcUrl, contractAddress, contractAbi, ethereumPrivateKey
    const userdata = env.USER_CONFIG || {};
    const { rpcUrl, contractAddress, contractAbi, ethereumPrivateKey } =
      userdata;

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

    const eventPayload = await request.json();
    console.log(`Worker received payload: ${JSON.stringify(eventPayload)}`);

    if (eventPayload === null) {
      // TODO: map events to functions instead of using kind to discriminate. allow specifying handler function names in work order
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

    // Otherwise, it's a web3 event (will need to fix the TODO before other event types are added)

    const web3Log = eventPayload;
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
      `Worker processing Web3Log. NewValue: ${newValueFromEvent.toString()}, Data: ${dataFromEvent}`,
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
  },
};
