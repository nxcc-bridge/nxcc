/**
 * Test that ensures newly initialized projects compile without type errors
 */
import { worker, type WorkerContext } from "./index";

describe("Project initialization type safety", () => {
  it("should compile a basic worker without type errors", () => {
    // This test ensures that the worker function and context types work correctly
    // when used in a typical project initialization scenario
    const testWorker = worker({
      async launch(eventPayload, { userdata }) {
        console.log("Worker launched!", eventPayload, userdata);
        // Should be able to access userdata properties without errors
        const name = userdata.name as string;
        expect(name).toBeDefined();
      },

      async fetch(request, { userdata }) {
        return {
          message: "Hello from nXCC worker!",
          path: new URL(request.url).pathname,
          config: userdata,
        };
      },

      async handleTransfer(eventPayload, { userdata }) {
        // Should be able to destructure from eventPayload
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
        
        return { processed: true, userConfig: userdata };
      },
    });

    expect(testWorker).toBeDefined();
    expect(typeof testWorker.fetch).toBe("function");
  });

  it("should work with proper context typing", () => {
    const mockContext: WorkerContext = {
      userdata: {
        name: "test-worker",
        version: "1.0.0",
        settings: {
          enabled: true,
          threshold: 100,
        },
      },
      env: {},
    };

    expect(mockContext.userdata.name).toBe("test-worker");
    expect(mockContext.userdata.settings).toEqual({
      enabled: true,
      threshold: 100,
    });
  });

  it("should handle event payload types correctly", () => {
    const mockEventPayload: Record<string, unknown> = {
      args: {
        from: "0x123...",
        to: "0x456...",
        value: "1000000",
      },
      transactionHash: "0xabc...",
      blockNumber: 12345,
      nested: {
        data: {
          timestamp: Date.now(),
        },
      },
    };

    // Should be able to access nested properties with type assertions
    const args = mockEventPayload.args as Record<string, unknown>;
    const nested = mockEventPayload.nested as Record<string, unknown>;
    const data = nested.data as Record<string, unknown>;
    
    expect(args.from).toBe("0x123...");
    expect(data.timestamp).toBeDefined();
  });
});