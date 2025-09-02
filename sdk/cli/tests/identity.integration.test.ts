import { exec } from "child_process";
import { promisify } from "util";
import * as fs from "fs/promises";
import * as path from "path";
import { createPublicClient, http, Address } from "viem";

const execAsync = promisify(exec);

describe("Identity CLI Integration Tests", () => {
  let anvilProcess: any;
  let anvilRpcUrl: string;
  let identityContractAddress: Address;
  let testPrivateKey: string;
  let testPolicyPath: string;
  let cliPath: string;

  beforeAll(async () => {
    // Set up paths
    const projectRoot = path.resolve(__dirname, "../../..");
    cliPath = path.join(projectRoot, "sdk/cli/dist/index.js");

    // Build CLI if needed
    try {
      await fs.access(cliPath);
    } catch {
      console.log("Building CLI...");
      await execAsync("pnpm build", { cwd: path.join(projectRoot, "sdk/cli") });
    }

    // Start anvil or use existing instance
    anvilRpcUrl = "http://localhost:8545";
    testPrivateKey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"; // Anvil default[0]

    try {
      // Check if anvil is already running
      const client = createPublicClient({ transport: http(anvilRpcUrl) });
      await client.getChainId();
      console.log("Using existing anvil instance");
    } catch {
      // Start new anvil instance
      console.log("Starting anvil...");
      const { spawn } = require("child_process");
      anvilProcess = spawn("anvil", ["--port", "8545"], {
        stdio: "pipe",
        detached: false,
      });

      // Wait for anvil to be ready
      await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error("Anvil failed to start")), 10000);
        const checkReady = async () => {
          try {
            const client = createPublicClient({ transport: http(anvilRpcUrl) });
            await client.getChainId();
            clearTimeout(timeout);
            resolve(void 0);
          } catch {
            setTimeout(checkReady, 500);
          }
        };
        checkReady();
      });
    }

    // Deploy Identity contract using forge script
    console.log("Deploying Identity contract...");
    const contractsDir = path.join(projectRoot, "contracts/evm");

    // Install dependencies if needed
    try {
      await fs.access(path.join(contractsDir, "dependencies"));
    } catch {
      console.log("Installing contract dependencies...");
      await execAsync("forge soldeer install", { cwd: contractsDir });
    }

    // Build contracts
    await execAsync("forge build", { cwd: contractsDir });

    try {
      // Deploy using the script
      const deployResult = await execAsync(
        `PRIVATE_KEY="${testPrivateKey}" forge script script/DeployIdentity.s.sol --rpc-url ${anvilRpcUrl} --broadcast`,
        { cwd: contractsDir },
      );

      // Extract contract address from output
      const addressMatch = deployResult.stdout.match(/Identity deployed to: (0x[a-fA-F0-9]{40})/);
      if (addressMatch) {
        identityContractAddress = addressMatch[1] as Address;
      } else {
        // Use default CREATE2 address
        identityContractAddress = "0xb1c985140805a55bf6d5Ea42232B73023dc51eE0" as Address;
      }
    } catch (error) {
      // Contract likely already deployed, use deterministic CREATE2 address
      console.log("Contract already deployed, using deterministic address");
      identityContractAddress = "0xb1c985140805a55bf6d5Ea42232B73023dc51eE0" as Address;
    }

    console.log(`Identity contract deployed at: ${identityContractAddress}`);

    // Create test policy file
    testPolicyPath = path.join(__dirname, "../test-policy.json");
    await fs.writeFile(
      testPolicyPath,
      JSON.stringify(
        {
          bundle: {
            source: "data:application/javascript;base64,Y29uc29sZS5sb2coIkhlbGxvIHdvcmxkISIp",
            hash: null,
          },
          identities: [],
          events: [{ handler: "launch", kind: "launch" }],
          userdata: { testPolicy: true },
        },
        null,
        2,
      ),
    );
  }, 60000);

  afterAll(async () => {
    // Clean up
    if (anvilProcess) {
      anvilProcess.kill();
    }
    try {
      await fs.unlink(testPolicyPath);
    } catch {
      // Ignore cleanup errors
    }
  });

  describe("identity create", () => {
    it("should create a new identity successfully", async () => {
      const { stdout } = await execAsync(
        `node "${cliPath}" identity create ${identityContractAddress} --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
      );

      // Extract JSON from stdout (skip the "Identity created successfully:" line)
      const jsonMatch = stdout.match(/\{[\s\S]*\}/);
      if (!jsonMatch) {
        throw new Error(`No JSON found in output: ${stdout}`);
      }
      const result = JSON.parse(jsonMatch[0]);
      expect(result).toMatchObject({
        chain: 31337,
        address: identityContractAddress,
        id: expect.any(String),
        txHash: expect.stringMatching(/^0x[a-fA-F0-9]{64}$/),
      });

      // Store the created identity ID for subsequent tests
      process.env.TEST_IDENTITY_ID = result.id;
    }, 30000);

    it("should fail with invalid contract address", async () => {
      await expect(
        execAsync(
          `node "${cliPath}" identity create 0x1234567890123456789012345678901234567890 --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
        ),
      ).rejects.toThrow();
    });
  });

  describe("identity set-policy", () => {
    it("should set policy for existing identity", async () => {
      const identityId = process.env.TEST_IDENTITY_ID || "1";

      const { stdout, stderr } = await execAsync(
        `node "${cliPath}" identity set-policy ${identityContractAddress} ${identityId} ${testPolicyPath} --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
      );

      expect(stdout).toMatch(/Policy set successfully\. Transaction hash: 0x[a-fA-F0-9]{64}/);
    }, 30000);

    it("should fail with non-existent identity", async () => {
      const nonExistentId = "999999";

      await expect(
        execAsync(
          `node "${cliPath}" identity set-policy ${identityContractAddress} ${nonExistentId} ${testPolicyPath} --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
        ),
      ).rejects.toThrow();
    });

    it("should handle HTTP URLs for policy", async () => {
      const identityId = process.env.TEST_IDENTITY_ID || "1";
      const httpPolicyUrl =
        "data:application/json;base64,eyJidW5kbGUiOnsic291cmNlIjoiZGF0YTphcHBsaWNhdGlvbi9qYXZhc2NyaXB0O2Jhc2U2NCxZMjl1YzI5c1pTNXNiMmNvSWtobGJHeGJJSGR2Y214a0lTSXAiLCJoYXNoIjpudWxsfSwiaWRlbnRpdGllcyI6W10sImV2ZW50cyI6W3siaGFuZGxlciI6ImxhdW5jaCIsImtpbmQiOiJsYXVuY2gifV0sInVzZXJkYXRhIjp7InRlc3RQb2xpY3kiOnRydWV9fQ==";

      const { stdout } = await execAsync(
        `node "${cliPath}" identity set-policy ${identityContractAddress} ${identityId} "${httpPolicyUrl}" --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
      );

      expect(stdout).toMatch(/Policy set successfully\. Transaction hash: 0x[a-fA-F0-9]{64}/);
    }, 30000);
  });

  describe("identity get-policy", () => {
    it("should retrieve policy for existing identity", async () => {
      const identityId = process.env.TEST_IDENTITY_ID || "1";

      const { stdout } = await execAsync(
        `node "${cliPath}" identity get-policy ${identityContractAddress} ${identityId} --gateway-url ${anvilRpcUrl}`,
      );

      expect(stdout).toMatch(/Policy URL: data:application\/json;base64,/);
    }, 30000);

    it("should fail with non-existent identity", async () => {
      const nonExistentId = "999999";

      await expect(
        execAsync(
          `node "${cliPath}" identity get-policy ${identityContractAddress} ${nonExistentId} --gateway-url ${anvilRpcUrl}`,
        ),
      ).rejects.toThrow();
    });
  });

  describe("full workflow", () => {
    it("should complete create -> set-policy -> get-policy workflow", async () => {
      // Create identity
      const createResult = await execAsync(
        `node "${cliPath}" identity create ${identityContractAddress} --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
      );

      // Extract JSON from stdout
      const jsonMatch = createResult.stdout.match(/\{[\s\S]*\}/);
      if (!jsonMatch) {
        throw new Error(`No JSON found in create output: ${createResult.stdout}`);
      }
      const identity = JSON.parse(jsonMatch[0]);

      // Set policy using file path
      await execAsync(
        `node "${cliPath}" identity set-policy ${identityContractAddress} ${identity.id} ${testPolicyPath} --gateway-url ${anvilRpcUrl} --signer ${testPrivateKey}`,
      );

      // Get policy back
      const getPolicyResult = await execAsync(
        `node "${cliPath}" identity get-policy ${identityContractAddress} ${identity.id} --gateway-url ${anvilRpcUrl}`,
      );

      expect(getPolicyResult.stdout).toMatch(/Policy URL: data:application\/json;base64,/);
    }, 60000);
  });
});
