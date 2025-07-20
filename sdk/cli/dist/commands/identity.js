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
exports.identitySubcommand = identitySubcommand;
const fs = __importStar(require("fs/promises"));
const path = __importStar(require("path"));
const web3_1 = require("../utils/web3");
async function create(chain, address, options) {
    try {
        const chainId = parseInt(chain, 10);
        const result = await (0, web3_1.createIdentity)(options.gatewayUrl, chainId, address, options.signer, "");
        console.log("Identity created successfully:");
        console.log(JSON.stringify(result, null, 2));
    }
    catch (error) {
        console.error("Failed to create identity:", error);
        process.exit(1);
    }
}
async function setPolicyCmd(chain, address, id, urlOrPath, options) {
    try {
        let policyUrl = urlOrPath;
        if (!urlOrPath.startsWith("http") && !urlOrPath.startsWith("data:")) {
            const bundlePath = path.resolve(process.cwd(), urlOrPath);
            const bundleContent = await fs.readFile(bundlePath);
            const bundleB64 = bundleContent.toString("base64");
            policyUrl = `data:application/json;base64,${bundleB64}`;
            console.log(`Using data URL for policy: ${policyUrl.substring(0, 50)}...`);
        }
        const chainId = parseInt(chain, 10);
        const txHash = await (0, web3_1.setPolicy)(options.gatewayUrl, chainId, address, id, policyUrl, options.signer);
        console.log(`Policy set successfully. Transaction hash: ${txHash}`);
    }
    catch (error) {
        console.error("Failed to set policy:", error);
        process.exit(1);
    }
}
async function getPolicyCmd(chain, address, id, options) {
    try {
        const chainId = parseInt(chain, 10);
        const policyUrl = await (0, web3_1.getPolicy)(options.gatewayUrl, chainId, address, id);
        console.log("Policy URL:", policyUrl);
    }
    catch (error) {
        console.error("Failed to get policy:", error);
        process.exit(1);
    }
}
function identitySubcommand(program) {
    const identity = program
        .command("identity")
        .description("Interact with an identity")
        .requiredOption("--gateway-url <url>", "Web3 gateway URL", "http://localhost:8545");
    identity
        .command("create <chain> <address>")
        .description("Create a new identity")
        .requiredOption("--signer <private-key>", "Private key to sign the transaction")
        .action(create);
    identity
        .command("set-policy <chain> <address> <id> <url-or-path-to-bundle>")
        .description("Set the policy worker for an identity")
        .requiredOption("--signer <private-key>", "Private key to sign the transaction")
        .action(setPolicyCmd);
    identity
        .command("get-policy <chain> <address> <id>")
        .description("Get the policy worker URL for an identity")
        .action(getPolicyCmd);
}
