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

    const policiesDir = path.join(projectDir, "policies");
    await fs.mkdir(policiesDir, { recursive: true });
    const workersDir = path.join(projectDir, "workers");
    await fs.mkdir(workersDir, { recursive: true });

    await copyTemplate("worker/worker.ts", workersDir, "my-worker.ts");
    await copyTemplate("worker/manifest.template.json", workersDir, "manifest.template.json");
    await copyTemplate("policy/default-policy.ts", policiesDir, "default-policy.ts");
    await copyTemplate("policy/manifest.template.json", policiesDir, "manifest.template.json");
    await copyTemplate("project_tsconfig.json", projectDir, "tsconfig.json");
    await copyTemplate("project_package.json", projectDir, "package.json");
  } catch (error) {
    console.error("Failed to initialize project:", error);
    process.exit(1);
  }
}
