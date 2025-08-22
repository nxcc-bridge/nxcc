import * as fs from "fs/promises";
import { Command } from "commander";

interface GetReportOptions {
  rpcUrl: string;
  output?: string;
}

async function getReport(options: GetReportOptions) {
  try {
    // Build the API URL
    const baseUrl = options.rpcUrl.replace(/\/+$/, ""); // Remove trailing slashes
    const apiUrl = new URL("/api/env-report", baseUrl);

    console.log(`Fetching env report from: ${apiUrl.toString()}`);

    const response = await fetch(apiUrl.toString(), {
      method: "GET",
      headers: {
        Accept: "application/json",
      },
    });

    if (!response.ok) {
      const errorText = await response.text();
      throw new Error(
        `Failed to fetch env report: ${response.status} ${response.statusText}\n${errorText}`,
      );
    }

    const envReportData = await response.json();

    // If output is specified, write to file, otherwise print to console
    if (options.output) {
      await fs.writeFile(options.output, JSON.stringify(envReportData, null, 2));
      console.log(`Environment report saved to: ${options.output}`);
    } else {
      console.log(JSON.stringify(envReportData, null, 2));
    }
  } catch (error) {
    console.error("Failed to get env report:", error);
    process.exit(1);
  }
}

export function nodeSubcommand(program: Command) {
  const node = program.command("node").description("Interact with an nXCC node");

  node
    .command("get-report")
    .description("Get the node's environment report (attestation + operator signature)")
    .requiredOption("--rpc-url <url>", "nXCC node HTTP RPC URL", "http://localhost:6922")
    .option("-o, --output <path>", "Output file to save the env report JSON")
    .action(getReport);
}
