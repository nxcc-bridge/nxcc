import * as fs from "fs/promises";
import * as path from "path";
import { execSync } from "child_process";
import { initCommand } from "./init";

const testDir = path.join(__dirname, "../../test-projects");

beforeAll(async () => {
  // Clean up any existing test directory
  try {
    await fs.rm(testDir, { recursive: true, force: true });
  } catch (error) {
    // Ignore if directory doesn't exist
  }
});

afterAll(async () => {
  // Clean up test directory
  try {
    await fs.rm(testDir, { recursive: true, force: true });
  } catch (error) {
    // Ignore cleanup errors
  }
});

describe("init command", () => {
  it("should create a project that builds without TypeScript errors", async () => {
    const projectName = "test-init-build";
    const projectPath = path.join(testDir, projectName);

    // Initialize project
    await initCommand(projectPath);

    // Verify files were created
    const expectedFiles = [
      "package.json",
      "tsconfig.json",
      "workers/my-worker.ts",
      "workers/manifest.template.json",
      "policies/default-policy.ts",
      "policies/manifest.template.json",
    ];

    for (const file of expectedFiles) {
      const filePath = path.join(projectPath, file);
      const exists = await fs.access(filePath).then(() => true).catch(() => false);
      expect(exists).toBe(true);
    }

    // Install dependencies
    execSync("npm install", { cwd: projectPath, stdio: "inherit" });

    // Install local SDK for testing
    const sdkPath = path.resolve(__dirname, "../../../lib");
    execSync(`npm install "${sdkPath}"`, { cwd: projectPath, stdio: "inherit" });

    // Build project - this should not throw
    expect(() => {
      execSync("npm run build", { cwd: projectPath, stdio: "inherit" });
    }).not.toThrow();

    // Verify build outputs exist
    const buildOutputs = [
      "dist/my-worker.js",
      "dist/default-policy.js",
    ];

    for (const output of buildOutputs) {
      const outputPath = path.join(projectPath, output);
      const exists = await fs.access(outputPath).then(() => true).catch(() => false);
      expect(exists).toBe(true);
    }
  });

  it("should create a project with valid TypeScript types", async () => {
    const projectName = "test-init-types";
    const projectPath = path.join(testDir, projectName);

    // Initialize project
    await initCommand(projectPath);

    // Install dependencies
    execSync("npm install", { cwd: projectPath, stdio: "inherit" });

    // Install local SDK for testing
    const sdkPath = path.resolve(__dirname, "../../../lib");
    execSync(`npm install "${sdkPath}"`, { cwd: projectPath, stdio: "inherit" });

    // TypeScript compilation should succeed
    expect(() => {
      execSync("npx tsc --noEmit", { cwd: projectPath, stdio: "inherit" });
    }).not.toThrow();
  });

  it("should create valid worker and policy manifest templates", async () => {
    const projectName = "test-init-manifests";
    const projectPath = path.join(testDir, projectName);

    // Initialize project
    await initCommand(projectPath);

    // Check worker manifest template
    const workerManifest = await fs.readFile(
      path.join(projectPath, "workers/manifest.template.json"),
      "utf-8"
    );
    const workerManifestObj = JSON.parse(workerManifest);
    expect(workerManifestObj.bundle).toBeDefined();
    expect(workerManifestObj.identities).toBeDefined();
    expect(workerManifestObj.userdata).toBeDefined();

    // Check policy manifest template
    const policyManifest = await fs.readFile(
      path.join(projectPath, "policies/manifest.template.json"),
      "utf-8"
    );
    const policyManifestObj = JSON.parse(policyManifest);
    expect(policyManifestObj.bundle).toBeDefined();
    expect(policyManifestObj.identities).toBeDefined();
  });
});