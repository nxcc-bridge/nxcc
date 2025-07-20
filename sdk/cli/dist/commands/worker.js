"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.workerSubcommand = workerSubcommand;
const fs = __importStar(require("fs/promises"));
const path = __importStar(require("path"));
const crypto_1 = require("crypto");
const crypto_2 = require("../utils/crypto");
const DSSE_WORK_ORDER_PAYLOAD_TYPE = "application/vnd.nxcc.workorderpayload.v1+json";
async function deploy(manifestPath, options) {
    try {
        const manifestAbsPath = path.resolve(process.cwd(), manifestPath);
        const manifestDir = path.dirname(manifestAbsPath);
        const manifestContent = await fs.readFile(manifestAbsPath, "utf-8");
        const workerManifest = JSON.parse(manifestContent);
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
        const launchEvent = {
            handler: "launch",
            kind: "launch",
        };
        const workOrderPayload = {
            id: `cli-wo-${(0, crypto_1.randomBytes)(8).toString("hex")}`,
            worker: workerManifest,
            events: [launchEvent],
        };
        const payloadJson = JSON.stringify(workOrderPayload);
        const dsseEnvelope = options.signer
            ? await (0, crypto_2.signDsse)(payloadJson, DSSE_WORK_ORDER_PAYLOAD_TYPE, options.signer)
            : (0, crypto_2.createUnsignedDsse)(payloadJson, DSSE_WORK_ORDER_PAYLOAD_TYPE);
        const response = await fetch(`${options.rpcUrl}/api/work-orders`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
            },
            body: JSON.stringify(dsseEnvelope),
        });
        const responseData = await response.json();
        if (!response.ok) {
            throw new Error(`Failed to deploy worker: ${response.status} ${response.statusText}\n${JSON.stringify(responseData)}`);
        }
        console.log("Worker deployed successfully:");
        console.log(JSON.stringify(responseData, null, 2));
    }
    catch (error) {
        console.error("Failed to deploy worker:", error);
        process.exit(1);
    }
}
function workerSubcommand(program) {
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
