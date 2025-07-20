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
exports.bundleCommand = bundleCommand;
const fs = __importStar(require("fs/promises"));
const path = __importStar(require("path"));
const crypto_1 = require("../utils/crypto");
const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE = "application/vnd.nxcc.workerbundlepayload.v1+json";
async function bundleCommand(manifestTemplatePath, options) {
    try {
        const manifestTemplateAbsPath = path.resolve(process.cwd(), manifestTemplatePath);
        const manifestDir = path.dirname(manifestTemplateAbsPath);
        const templateContent = await fs.readFile(manifestTemplateAbsPath, "utf-8");
        const manifestTemplate = JSON.parse(templateContent);
        const codePath = manifestTemplate.bundle?.source;
        if (!codePath || typeof codePath !== "string") {
            throw new Error('Manifest template must have a "bundle.source" property with a path to the built code.');
        }
        const codeAbsPath = path.resolve(manifestDir, codePath);
        const code = await fs.readFile(codeAbsPath);
        const bundlePayload = {
            vm: "nxcc/workerd",
            executable: code.toString("base64"),
            metadata: manifestTemplate.userdata?.name ? { name: manifestTemplate.userdata.name } : {},
        };
        const payloadJson = JSON.stringify(bundlePayload);
        const dsseEnvelope = options.signer
            ? await (0, crypto_1.signDsse)(payloadJson, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, options.signer)
            : (0, crypto_1.createUnsignedDsse)(payloadJson, DSSE_WORKER_BUNDLE_PAYLOAD_TYPE);
        const outPath = options.out
            ? path.resolve(process.cwd(), options.out)
            : path.join(manifestDir, "bundle.json");
        await fs.writeFile(outPath, JSON.stringify(dsseEnvelope, null, 2));
        console.log(`Worker bundle created successfully at: ${outPath}`);
    }
    catch (error) {
        console.error("Failed to create bundle:", error);
        process.exit(1);
    }
}
