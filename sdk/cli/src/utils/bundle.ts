import * as fs from "fs/promises";
import * as path from "path";
import { fileURLToPath } from "url";
import { WorkerBundlePayload, WorkerManifest } from "./types";
import { createUnsignedDsse } from "./crypto";

export const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE = "application/vnd.nxcc.workerbundlepayload.v1+json";

function resolveLocalPath(target: string, baseDir: string): string {
  if (target.startsWith("file://")) {
    return fileURLToPath(target);
  }

  if (/^[a-zA-Z]+:\/\//.test(target)) {
    throw new Error(`Unsupported protocol in path: ${target}`);
  }

  if (path.isAbsolute(target)) {
    return target;
  }

  return path.resolve(baseDir, target);
}

export async function embedBundleSource(
  manifest: WorkerManifest,
  manifestDir: string,
): Promise<void> {
  if (!manifest.bundle || !manifest.bundle.source) {
    throw new Error('Manifest must include a "bundle.source" value to bundle.');
  }

  const source = manifest.bundle.source;
  if (source.startsWith("data:")) {
    return;
  }

  const bundlePath = resolveLocalPath(source, manifestDir);
  const bundleContent = await fs.readFile(bundlePath);

  const workerBundlePayload: WorkerBundlePayload = {
    vm: "nxcc/workerd",
    executable: bundleContent.toString("base64"),
    metadata: {},
  };

  const bundleEnvelope = createUnsignedDsse(
    JSON.stringify(workerBundlePayload),
    DSSE_WORKER_BUNDLE_PAYLOAD_TYPE,
  );

  const bundleEnvelopeJson = JSON.stringify(bundleEnvelope);
  const bundleB64 = Buffer.from(bundleEnvelopeJson).toString("base64");
  manifest.bundle = {
    ...manifest.bundle,
    source: `data:application/json;base64,${bundleB64}`,
  };
}

export async function bundleManifestFileToDataUrl(
  manifestPathOrUrl: string,
  baseDir: string = process.cwd(),
): Promise<{ dataUrl: string; manifest: WorkerManifest }> {
  const manifestPath = resolveLocalPath(manifestPathOrUrl, baseDir);
  const manifestDir = path.dirname(manifestPath);
  const manifestContent = await fs.readFile(manifestPath, "utf-8");
  const manifestJson: WorkerManifest = JSON.parse(manifestContent);

  await embedBundleSource(manifestJson, manifestDir);

  const manifestJsonString = JSON.stringify(manifestJson);
  const manifestBase64 = Buffer.from(manifestJsonString).toString("base64");

  return {
    dataUrl: `data:application/json;base64,${manifestBase64}`,
    manifest: manifestJson,
  };
}
