import * as fs from "fs/promises";
import * as os from "os";
import * as path from "path";
import { pathToFileURL } from "url";
import {
  bundleManifestFileToDataUrl,
  embedBundleSource,
  DSSE_WORKER_BUNDLE_PAYLOAD_TYPE,
  DEFAULT_BUNDLE_VM,
  resolveBundleVm,
} from "./bundle";
import { WorkerBundlePayload, WorkerManifest } from "./types";

describe("bundle helpers", () => {
  let tempDir: string;

  beforeAll(async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "nxcc-bundle-test-"));
    const policyPath = path.join(tempDir, "policy.js");
    await fs.writeFile(policyPath, "export default () => 'ok';\n");
  });

  afterAll(async () => {
    await fs.rm(tempDir, { recursive: true, force: true });
  });

  function decodeBundlePayload(source: string): WorkerBundlePayload {
    const encodedEnvelope = source.split(",")[1];
    const envelopeJson = Buffer.from(encodedEnvelope, "base64").toString("utf-8");
    const envelope = JSON.parse(envelopeJson);
    const payloadJson = Buffer.from(envelope.payload, "base64").toString("utf-8");
    return JSON.parse(payloadJson) as WorkerBundlePayload;
  }

  async function writeManifest(
    filename = "manifest.json",
    overrides: Partial<WorkerManifest> = {},
  ) {
    const manifestPath = path.join(tempDir, filename);
    const manifest: WorkerManifest = {
      bundle: { source: "./policy.js" },
      identities: [],
      userdata: {},
      ...overrides,
      bundle: {
        source: "./policy.js",
        ...overrides.bundle,
      },
    };
    await fs.writeFile(manifestPath, JSON.stringify(manifest));
    return manifestPath;
  }

  it("resolveBundleVm prefers manifest vm over bundle vm", () => {
    const resolved = resolveBundleVm({
      vm: "nxcc/zenroom",
      bundle: { source: "./policy.js", vm: "nxcc/workerd" },
    });

    expect(resolved).toBe("nxcc/zenroom");
  });

  it("resolveBundleVm uses bundle vm when manifest vm is missing", () => {
    const resolved = resolveBundleVm({
      bundle: { source: "./policy.js", vm: "nxcc/zenroom" },
    });

    expect(resolved).toBe("nxcc/zenroom");
  });

  it("resolveBundleVm falls back to default when no vm is set", () => {
    const resolved = resolveBundleVm({
      bundle: { source: "./policy.js" },
    });

    expect(resolved).toBe(DEFAULT_BUNDLE_VM);
  });

  it("embedBundleSource converts local bundle source to data URL", async () => {
    const manifest: WorkerManifest = {
      bundle: { source: "./policy.js" },
      identities: [],
      userdata: {},
    };

    await embedBundleSource(manifest, tempDir);

    expect(manifest.bundle.source).toMatch(/^data:application\/json;base64,/);

    const encodedEnvelope = manifest.bundle.source.split(",")[1];
    const envelopeJson = Buffer.from(encodedEnvelope, "base64").toString("utf-8");
    const envelope = JSON.parse(envelopeJson);
    expect(envelope.payloadType).toBe(DSSE_WORKER_BUNDLE_PAYLOAD_TYPE);
  });

  it("embedBundleSource writes bundle payload vm from manifest vm", async () => {
    const manifest: WorkerManifest = {
      vm: "nxcc/zenroom",
      bundle: { source: "./policy.js" },
      identities: [],
      userdata: {},
    };

    await embedBundleSource(manifest, tempDir);

    const payload = decodeBundlePayload(manifest.bundle.source);
    expect(payload.vm).toBe("nxcc/zenroom");
  });

  it("bundleManifestFileToDataUrl uses bundle vm when manifest vm is missing", async () => {
    const manifestPath = await writeManifest("bundle-vm-manifest.json", {
      bundle: { vm: "nxcc/zenroom" },
    });

    const { dataUrl } = await bundleManifestFileToDataUrl(manifestPath);

    const manifestJson = Buffer.from(dataUrl.split(",")[1], "base64").toString("utf-8");
    const manifest = JSON.parse(manifestJson) as WorkerManifest;
    const payload = decodeBundlePayload(manifest.bundle.source);
    expect(payload.vm).toBe("nxcc/zenroom");
  });

  it("bundleManifestFileToDataUrl returns bundled manifest data URL", async () => {
    const manifestPath = await writeManifest();

    const { dataUrl } = await bundleManifestFileToDataUrl(manifestPath);

    expect(dataUrl).toMatch(/^data:application\/json;base64,/);
    const manifestJson = Buffer.from(dataUrl.split(",")[1], "base64").toString("utf-8");
    const manifest = JSON.parse(manifestJson) as WorkerManifest;
    expect(manifest.bundle.source).toMatch(/^data:application\/json;base64,/);
  });

  it("supports manifest paths provided as file URLs", async () => {
    const manifestPath = await writeManifest("file-url-manifest.json");
    const manifestUrl = pathToFileURL(manifestPath).toString();

    const { dataUrl } = await bundleManifestFileToDataUrl(manifestUrl);

    expect(dataUrl).toMatch(/^data:application\/json;base64,/);
  });
});
