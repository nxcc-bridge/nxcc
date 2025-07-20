import * as fs from "fs/promises";
import * as path from "path";
import { randomBytes } from "crypto";
import { Command } from "commander";
import { Hex } from "viem";
import { LaunchWorkerEvent, WorkOrderPayload, WorkerManifest } from "../utils/types";
import { createUnsignedDsse, signDsse } from "../utils/crypto";

const DSSE_WORK_ORDER_PAYLOAD_TYPE = "application/vnd.nxcc.workorderpayload.v1+json";

async function deploy(
  manifestPath: string,
  options: {
    rpcUrl: string;
    bundle?: boolean;
    signer?: Hex;
  },
) {
  try {
    const manifestAbsPath = path.resolve(process.cwd(), manifestPath);
    const manifestDir = path.dirname(manifestAbsPath);

    const manifestContent = await fs.readFile(manifestAbsPath, "utf-8");
    const workerManifest: WorkerManifest = JSON.parse(manifestContent);

    if (options.bundle) {
      const bundlePath = workerManifest.bundle.source;
      if (!bundlePath) {
        throw new Error('Manifest for bundling must have a "bundle.source" file path.');
      }
      const bundleAbsPath = path.resolve(manifestDir, bundlePath);
      const bundleContent = await fs.readFile(bundleAbsPath);
      const bundleB64 = bundleContent.toString("base64");
      workerManifest.bundle.source = `data:application/json;base64,${bundleB64}`;
    }

    const launchEvent: LaunchWorkerEvent = {
      handler: "launch",
      kind: "launch",
    };

    const workOrderPayload: WorkOrderPayload = {
      id: `cli-wo-${randomBytes(8).toString("hex")}`,
      worker: workerManifest,
      events: [launchEvent],
    };

    const payloadJson = JSON.stringify(workOrderPayload);

    const dsseEnvelope = options.signer
      ? await signDsse(payloadJson, DSSE_WORK_ORDER_PAYLOAD_TYPE, options.signer)
      : createUnsignedDsse(payloadJson, DSSE_WORK_ORDER_PAYLOAD_TYPE);

    const response = await fetch(`${options.rpcUrl}/api/work-orders`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(dsseEnvelope),
    });

    const responseData = await response.json();

    if (!response.ok) {
      throw new Error(
        `Failed to deploy worker: ${response.status} ${
          response.statusText
        }\n${JSON.stringify(responseData)}`,
      );
    }

    console.log("Worker deployed successfully:");
    console.log(JSON.stringify(responseData, null, 2));
  } catch (error) {
    console.error("Failed to deploy worker:", error);
    process.exit(1);
  }
}

export function workerSubcommand(program: Command) {
  const worker = program
    .command("worker")
    .description("Interact with a worker")
    .requiredOption("--rpc-url <url>", "nXCC node HTTP RPC URL", "http://localhost:6922");

  worker
    .command("deploy <manifest-path>")
    .description("Deploy a worker to an nXCC node")
    .option("--bundle", "Bundle the worker code into a data URL")
    .option("--signer <private-key>", "Private key to sign the work order")
    .action(deploy);
}
