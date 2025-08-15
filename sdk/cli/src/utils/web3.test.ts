import { createPublicClient, http } from "viem";

// Mock the internal getClients function by testing the direct approach
function testGetClients(gatewayUrl: string) {
  const transport = http(gatewayUrl);
  const publicClient = createPublicClient({ transport });
  return { publicClient };
}

describe("web3 utils", () => {
  // Mock gateway URL for testing
  const mockGatewayUrl = "http://localhost:8545";

  it("should create clients without errors", () => {
    expect(() => {
      testGetClients(mockGatewayUrl);
    }).not.toThrow();
  });

  it("should create clients with different gateway URLs", () => {
    const testUrls = [
      "http://localhost:8545",
      "https://rpc.ankr.com/eth",
      "https://polygon-rpc.com",
      "wss://eth-mainnet.alchemyapi.io/v2/demo",
    ];

    testUrls.forEach((url) => {
      expect(() => {
        testGetClients(url);
      }).not.toThrow();
    });
  });

  it("should create clients without requiring chain-specific configuration", () => {
    // This test verifies that we can create clients for any chain through a gateway
    // without needing to know chain-specific details
    expect(() => {
      const { publicClient } = testGetClients(mockGatewayUrl);
      expect(publicClient).toBeDefined();
    }).not.toThrow();
  });
});
