import * as fs from "fs/promises";
import * as path from "path";
import { Hex } from "viem";
import { WorkerBundlePayload, WorkerManifest } from "../utils/types";
import { resolveBundleVm } from "../utils/bundle";
import { createUnsignedDsse, signDsse } from "../utils/crypto";

const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE = "application/vnd.nxcc.workerbundlepayload.v1+json";

export async function bundleCommand(
  manifestTemplatePath: string,
  options: { out?: string; signer?: Hex },
) {
  try {
    const manifestTemplateAbsPath = path.resolve(process.cwd(), manifestTemplatePath);
    const manifestDir = path.dirname(manifestTemplateAbsPath);

    const templateContent = await fs.readFile(manifestTemplateAbsPath, "utf-8");
    const manifestTemplate: Partial<WorkerManifest> = JSON.parse(templateContent);

    const codePath = manifestTemplate.bundle?.source;
    if (!codePath || typeof codePath !== "string") {
      throw new Error(
        'Manifest template must have a "bundle.source" property with a path to the built code.',
      );
    }
    const codeAbsPath = path.resolve(manifestDir, codePath);
    const code = await fs.readFile(codeAbsPath);

    const bundlePayload: WorkerBundlePayload = {
      vm: resolveBundleVm(manifestTemplate),
      executable: code.toString("base64"),
      metadata: manifestTemplate.userdata?.name ? { name: manifestTemplate.userdata.name } : {},
    };

    const payloadJson = JSON.stringify(bundlePayload);

    const dsseEnvelope = options.signer
      ? await signDsse(payloadJson, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, options.signer)
      : createUnsignedDsse(payloadJson, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE);

    const outPath = options.out
      ? path.resolve(process.cwd(), options.out)
      : path.join(manifestDir, "bundle.json");

    await fs.writeFile(outPath, JSON.stringify(dsseEnvelope, null, 2));

    console.log(`Worker bundle created successfully at: ${outPath}`);
  } catch (error) {
    console.error("Failed to create bundle:", error);
    process.exit(1);
  }
}
