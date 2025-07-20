#!/usr/bin/env node
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const commander_1 = require("commander");
const init_1 = require("./commands/init");
const bundle_1 = require("./commands/bundle");
const worker_1 = require("./commands/worker");
const identity_1 = require("./commands/identity");
const package_json_1 = __importDefault(require("../package.json"));
const program = new commander_1.Command();
program.name("nxcc").description("CLI to interact with nXCC nodes").version(package_json_1.default.version);
program
    .command("init [directory]")
    .description("Create a new nXCC TypeScript project")
    .action(init_1.initCommand);
program
    .command("bundle <manifest-template>")
    .description("Create a worker bundle from a manifest template")
    .option("--out <path>", "Output path for the bundle")
    .option("--signer <private-key>", "Private key to sign the bundle")
    .action(bundle_1.bundleCommand);
(0, worker_1.workerSubcommand)(program);
(0, identity_1.identitySubcommand)(program);
program.parse(process.argv);
