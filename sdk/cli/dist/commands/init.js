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
exports.initCommand = initCommand;
const fs = __importStar(require("fs/promises"));
const path = __importStar(require("path"));
const templatesDir = path.resolve(__dirname, "../../templates");
async function copyTemplate(templateName, destPath, fileName) {
    const source = path.join(templatesDir, templateName);
    const dest = path.join(destPath, fileName || templateName);
    await fs.mkdir(path.dirname(dest), { recursive: true });
    await fs.copyFile(source, dest);
}
async function initCommand(dir) {
    const projectDir = path.resolve(process.cwd(), dir || ".");
    console.log(`Initializing new nXCC project in ${projectDir}...`);
    try {
        await fs.mkdir(projectDir, { recursive: true });
        await fs.mkdir(path.join(projectDir, "policies"), { recursive: true });
        const workersDir = path.join(projectDir, "workers");
        await fs.mkdir(workersDir, { recursive: true });
        await copyTemplate("worker/worker.ts", workersDir, "my-worker.ts");
        await copyTemplate("worker/manifest.template.json", workersDir, "manifest.template.json");
        await copyTemplate("project_tsconfig.json", projectDir, "tsconfig.json");
        await copyTemplate("project_package.json", projectDir, "package.json");
        console.log("Project initialized successfully.");
        console.log("\nNext steps:");
        console.log(`1. cd into your new project directory${dir ? ` (${dir})` : ""}.`);
        console.log("2. Run `npm install` to install dependencies.");
        console.log("3. Edit `workers/my-worker.ts` and `workers/manifest.template.json`.");
        console.log("4. Run `npm run build` to compile your worker.");
        console.log("5. Use `nxcc bundle` and `nxcc worker deploy` to deploy your worker.");
    }
    catch (error) {
        console.error("Failed to initialize project:", error);
        process.exit(1);
    }
}
