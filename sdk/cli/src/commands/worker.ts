import * as fs from "fs/promises";
import * as path from "path";
import { randomBytes } from "crypto";
import { Command } from "commander";
import { Hex } from "viem";
import { LaunchWorkerEvent, WorkOrderPayload, WorkerManifest } from "../utils/types";
import { createUnsignedDsse, signDsse } from "../utils/crypto";

const DSSE_WORK_ORDER_PAYLOAD_TYPE = "application/vnd.nxcc.workorderpayload.v1+json";
const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE = "application/vnd.nxcc.workerbundlepayload.v1+json";

interface WorkerBundlePayload {
  vm: string;
  executable: string;
  metadata: Record<string, string>;
}

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
    const manifestJson = JSON.parse(manifestContent);
    const workerManifest: WorkerManifest = manifestJson;

    if (options.bundle) {
      const bundlePath = workerManifest.bundle.source;
      if (!bundlePath) {
        throw new Error('Manifest for bundling must have a "bundle.source" file path.');
      }
      const bundleAbsPath = path.resolve(manifestDir, bundlePath);
      const bundleContent = await fs.readFile(bundleAbsPath);

      const workerBundlePayload: WorkerBundlePayload = {
        vm: "nxcc/workerd",
        executable: bundleContent.toString("base64"),
        metadata: {},
      };
      const workerBundlePayloadJson = JSON.stringify(workerBundlePayload);

      const bundleDsseEnvelope = createUnsignedDsse(
        workerBundlePayloadJson,
        DSSE_WORKER_BUNDLE_PAYLOAD_TYPE,
      );
      const bundleDsseEnvelopeJson = JSON.stringify(bundleDsseEnvelope);
      const bundleB64 = Buffer.from(bundleDsseEnvelopeJson).toString("base64");
      workerManifest.bundle.source = `data:application/json;base64,${bundleB64}`;
    }

    const workOrderPayload: WorkOrderPayload = {
      id: `cli-wo-${randomBytes(8).toString("hex")}`,
      worker: workerManifest,
      events: manifestJson?.events ?? [],
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

    const responseMessage = await response.text();

    if (!response.ok) {
      throw new Error(
        `Failed to deploy worker: ${response.status} ${response.statusText}\n${responseMessage}`,
      );
    }

    console.log("Worker deployed successfully:");
    console.log(responseMessage);
  } catch (error) {
    console.error("Failed to deploy worker:", error);
    process.exit(1);
  }
}

async function logs(
  workerId: string,
  options: {
    rpcUrl: string;
    follow: boolean;
    tail: string;
  },
) {
  try {
    const tailLines = parseInt(options.tail, 10);
    if (isNaN(tailLines) || tailLines < 0) {
      console.error("Error: --tail must be a positive number");
      process.exit(1);
    }

    // Build the API URL
    const baseUrl = options.rpcUrl.replace(/\/+$/, ""); // Remove trailing slashes
    const apiUrl = new URL(`/api/workers/${encodeURIComponent(workerId)}/logs`, baseUrl);

    // Add query parameters
    if (tailLines > 0) {
      apiUrl.searchParams.set("tail", tailLines.toString());
    }
    if (options.follow) {
      apiUrl.searchParams.set("follow", "true");
    }

    console.log(`Fetching logs for worker: ${workerId}`);

    if (options.follow) {
      console.log("Streaming logs (press Ctrl+C to stop)...");
      await streamLogs(apiUrl.toString());
    } else {
      // For non-streaming, we'd make a regular HTTP request
      // But for now, let's redirect users to use --follow
      console.log("Note: Use --follow to stream logs in real-time");
      apiUrl.searchParams.set("follow", "true");
      await streamLogs(apiUrl.toString());
    }
  } catch (error) {
    console.error("Error fetching logs:", error);
    process.exit(1);
  }
}

async function streamLogs(url: string) {
  try {
    const response = await fetch(url, {
      headers: {
        Accept: "text/event-stream",
        "Cache-Control": "no-cache",
      },
    });

    if (!response.ok) {
      const errorText = await response.text();
      throw new Error(`HTTP ${response.status}: ${errorText}`);
    }

    if (!response.body) {
      throw new Error("No response body");
    }

    // Read the Server-Sent Events stream
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    // Handle Ctrl+C gracefully
    process.on("SIGINT", () => {
      console.log("\nStopping log stream...");
      reader.cancel();
      process.exit(0);
    });

    while (true) {
      const { done, value } = await reader.read();

      if (done) {
        break;
      }

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");

      // Process all complete lines, keep the last incomplete line in buffer
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (line.startsWith("data: ")) {
          const logData = line.substring(6); // Remove "data: " prefix
          if (logData.trim() && logData !== "keep-alive") {
            console.log(logData);
          }
        }
      }
    }
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") {
      console.log("Log stream stopped.");
    } else {
      throw error;
    }
  }
}

export function workerSubcommand(program: Command) {
  const worker = program.command("worker").description("Interact with a worker");

  worker
    .command("deploy <manifest-path>")
    .description("Deploy a worker to an nXCC node")
    .requiredOption("--rpc-url <url>", "nXCC node HTTP RPC URL", "http://localhost:6922")
    .option("--bundle", "Bundle the worker code into a data URL")
    .option("--signer <private-key>", "Private key to sign the work order")
    .action(deploy);

  worker
    .command("logs <worker-id>")
    .description("Stream logs from a worker")
    .requiredOption("--rpc-url <url>", "nXCC node HTTP RPC URL", "http://localhost:6922")
    .option("-f, --follow", "Follow log output (stream new logs)", false)
    .option("-t, --tail <lines>", "Number of lines to tail", "10")
    .action(logs);
}
