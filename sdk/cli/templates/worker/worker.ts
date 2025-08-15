import { worker, type WorkerContext } from "@nxcc/sdk";

export default worker({
  async launch(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    console.log("Worker launched!", eventPayload, userdata);
  },

  async fetch(request: Request, { userdata }: WorkerContext) {
    return {
      message: "Hello from nXCC worker!",
      path: new URL(request.url).pathname,
    };
  },

  async handleTransfer(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    const args = eventPayload.args as Record<string, unknown>;
    const from = args?.from as string;
    const to = args?.to as string;
    const value = args?.value as string;
    const transactionHash = eventPayload.transactionHash as string;
    const blockNumber = eventPayload.blockNumber as number;

    console.log(`USDC Transfer detected:`);
    console.log(`  From: ${from}`);
    console.log(`  To: ${to}`);
    console.log(`  Amount: ${(Number(value) / 1e6).toFixed(2)} USDC`);
    console.log(`  Tx: ${transactionHash}`);
    console.log(`  Block: ${blockNumber}`);
  },
});
