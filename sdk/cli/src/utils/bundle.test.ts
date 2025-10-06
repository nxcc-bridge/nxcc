import * as fs from "fs/promises";
import * as os from "os";
import * as path from "path";
import { pathToFileURL } from "url";
import {
  bundleManifestFileToDataUrl,
  embedBundleSource,
  DSSE_WORKER_BUNDLE_PAYLOAD_TYPE,
} from "./bundle";
import { WorkerManifest } from "./types";

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

  async function writeManifest(filename = "manifest.json") {
    const manifestPath = path.join(tempDir, filename);
    const manifest: WorkerManifest = {
      bundle: { source: "./policy.js" },
      identities: [],
      userdata: {},
    };
    await fs.writeFile(manifestPath, JSON.stringify(manifest));
    return manifestPath;
  }

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
