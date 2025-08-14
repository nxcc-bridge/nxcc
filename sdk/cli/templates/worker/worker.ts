import { worker } from "@nxcc/sdk";

export default worker({
  async launch(eventPayload, { userdata }) {
    console.log("Worker launched!", eventPayload, userdata);
  },

  async fetch(request, { userdata }) {
    return {
      message: "Hello from nXCC worker!",
      path: new URL(request.url).pathname,
    };
  },

  async handleTransfer(eventPayload, { userdata }) {
    const { from, to, value } = eventPayload.args;
    const { transactionHash, blockNumber } = eventPayload;

    console.log(`USDC Transfer detected:`);
    console.log(`  From: ${from}`);
    console.log(`  To: ${to}`);
    console.log(`  Amount: ${(Number(value) / 1e6).toFixed(2)} USDC`);
    console.log(`  Tx: ${transactionHash}`);
    console.log(`  Block: ${blockNumber}`);
  },
});
