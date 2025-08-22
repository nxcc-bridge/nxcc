#!/usr/bin/env node

import { Command } from "commander";
import { initCommand } from "./commands/init";
import { bundleCommand } from "./commands/bundle";
import { workerSubcommand } from "./commands/worker";
import { identitySubcommand } from "./commands/identity";
import { nodeSubcommand } from "./commands/node";
import pkg from "../package.json";

const program = new Command();

program.name("nxcc").description("CLI to interact with nXCC nodes").version(pkg.version);

program
  .command("init [directory]")
  .description("Create a new nXCC TypeScript project")
  .action(initCommand);

program
  .command("bundle <manifest-template>")
  .description("Create a worker bundle from a manifest template")
  .option("--out <path>", "Output path for the bundle")
  .option("--signer <private-key>", "Private key to sign the bundle")
  .action(bundleCommand);

workerSubcommand(program);
identitySubcommand(program);
nodeSubcommand(program);

program.parse(process.argv);
