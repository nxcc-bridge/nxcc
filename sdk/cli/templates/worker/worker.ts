import { worker, type WorkerContext } from "@nxcc/sdk";
import { Hex, decodeEventLog, formatUnits, parseAbiItem } from "viem";

const transferEvent = parseAbiItem(
  "event Transfer(address indexed from, address indexed to, uint256 value)",
);

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
    try {
      const {
        args: { from, to, value },
      } = decodeEventLog({
        abi: [transferEvent],
        topics: eventPayload.topics as [signature: Hex, ...args: Hex[]],
        data: eventPayload.data as Hex,
      });

      const transactionHash = eventPayload.transaction_hash as Hex;
      const blockNumber = eventPayload.block_number as number;

      console.log(`➡️ Transfer detected in block ${blockNumber}:`);
      console.log(`  From: ${from}`);
      console.log(`  To: ${to}`);
      console.log(`  Amount: ${formatUnits(value, 6)} USDC`);
      console.log(`  Tx: ${transactionHash}`);
    } catch (error) {
      console.error("Failed to decode transfer event", error, eventPayload);
    }
  },

  async tick(eventPayload: Record<string, unknown>, { userdata }: WorkerContext) {
    const timestamp = new Date().toISOString();
    console.log(`Scheduled tick executed at ${timestamp}`);

    // Example: Perform periodic tasks like data aggregation, monitoring, etc.
    const status = {
      timestamp,
      message: "Scheduled event fired successfully",
      eventPayload,
      userdata,
    };

    console.log("Tick event processed:", status);
    return status;
  },
});
