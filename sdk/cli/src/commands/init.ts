import * as fs from "fs/promises";
import * as path from "path";

const templatesDir = path.resolve(__dirname, "../../templates");

async function copyTemplate(templateName: string, destPath: string, fileName?: string) {
  const source = path.join(templatesDir, templateName);
  const dest = path.join(destPath, fileName || templateName);
  await fs.mkdir(path.dirname(dest), { recursive: true });
  await fs.copyFile(source, dest);
}

export async function initCommand(dir?: string) {
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
  } catch (error) {
    console.error("Failed to initialize project:", error);
    process.exit(1);
  }
}
