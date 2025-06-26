import type { Project, CodeFile } from "./types";

// Original content snippets and helper functions
const simpleAppJs = `/**
 * A simple app that logs a message on startup.
 */
function main() {
  console.log("Hello from the secure app!");
}

main();
`;

const simpleTokenBridgeJs = `/**
 * Mocks moving a token between two chains.
 * In a real app, this would involve listening to events and signing transactions.
 */
function onEvent(event) {
  if (event.name === 'bridgeToken') {
    console.log(
      \`Bridging \${event.data.amount} \${event.data.token} from \${event.data.fromChain} to \${event.data.toChain}\`
    );
    // TODO: Add actual bridge logic here
  }
}

console.log("Simple Token Bridge app loaded. Waiting for events.");
`;

const aiAssetBridgeJs = `/**
 * Mocks an AI-powered asset bridge.
 * The app calls a (mocked) AI service to determine the optimal bridging strategy.
 */
async function callAIService(data) {
  console.log("Calling AI service with data:", data);
  // Mocked AI response
  return new Promise(resolve => {
    setTimeout(() => {
      resolve({
        optimalRoute: "direct-swap",
        estimatedCost: "0.01 ETH",
        confidence: 0.95,
      });
    }, 1000);
  });
}

async function onEvent(event) {
  if (event.name === 'bridgeAssetWithAI') {
    console.log("Received asset bridge request:", event.data);
    const aiRecommendation = await callAIService(event.data);
    console.log("AI Recommendation:", aiRecommendation);
    console.log("Executing bridge based on AI recommendation...");
  }
}

console.log("AI Asset Bridge app loaded. Waiting for events.");
`;

function createPolicyJsContent(policyObject: object): string {
  return `export default ${JSON.stringify(policyObject, null, 2)};`;
}

// Data for the original default projects
const originalProjectTemplates: Project[] = [
  {
    id: "proj-sec-work",
    name: "1-Security-Demo-Working-Policy",
    files: [
      {
        id: "proj-sec-work-worker",
        name: "workers/security-demo-working/worker.js",
        language: "javascript",
        content: simpleAppJs,
      },
      {
        id: "proj-sec-work-worker-manifest",
        name: "workers/security-demo-working/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "security-demo-working",
            entrypoint: "worker.js",
            version: "1.0.0",
            type: "worker",
          },
          null,
          2,
        ),
      },
      {
        id: "proj-sec-work-policy",
        name: "policies/security-demo-working/policy.js",
        language: "javascript",
        content: createPolicyJsContent({
          description: "This policy allows basic execution and should pass.",
          permissions: {
            filesystem: { access: "none" },
            network: { allow: [] },
            compute: {
              cpuCores: { max: 1 },
              memory: { max: "256MB" },
            },
          },
        }),
      },
      {
        id: "proj-sec-work-policy-manifest",
        name: "policies/security-demo-working/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "security-demo-working-policy",
            entrypoint: "policy.js",
            type: "policy",
          },
          null,
          2,
        ),
      },
    ],
  },
  {
    id: "proj-sec-fail",
    name: "2-Security-Demo-Failing-Policy",
    files: [
      {
        id: "proj-sec-fail-worker",
        name: "workers/security-demo-failing/worker.js",
        language: "javascript",
        content: simpleAppJs,
      },
      {
        id: "proj-sec-fail-worker-manifest",
        name: "workers/security-demo-failing/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "security-demo-failing",
            entrypoint: "worker.js",
            version: "1.0.0",
            type: "worker",
          },
          null,
          2,
        ),
      },
      {
        id: "proj-sec-fail-policy",
        name: "policies/security-demo-failing/policy.js",
        language: "javascript",
        content: createPolicyJsContent({
          description:
            "This policy requires network access which the app does not have, causing a mismatch. The job will not run.",
          permissions: {
            filesystem: { access: "none" },
            network: { allow: ["api.some-service.com"] },
            compute: {
              cpuCores: { max: 1 },
              memory: { max: "256MB" },
            },
          },
        }),
      },
      {
        id: "proj-sec-fail-policy-manifest",
        name: "policies/security-demo-failing/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "security-demo-failing-policy",
            entrypoint: "policy.js",
            type: "policy",
          },
          null,
          2,
        ),
      },
    ],
  },
  {
    id: "proj-token-bridge",
    name: "3-App-Simple-Token-Bridge",
    files: [
      {
        id: "proj-token-bridge-worker",
        name: "workers/simple-token-bridge/worker.js",
        language: "javascript",
        content: simpleTokenBridgeJs,
      },
      {
        id: "proj-token-bridge-worker-manifest",
        name: "workers/simple-token-bridge/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "simple-token-bridge",
            entrypoint: "worker.js",
            version: "1.0.0",
            type: "worker",
            events: [{ name: "bridgeToken", sourceChain: "ethereum" }],
          },
          null,
          2,
        ),
      },
      {
        id: "proj-token-bridge-policy",
        name: "policies/simple-token-bridge/policy.js",
        language: "javascript",
        content: createPolicyJsContent({
          permissions: {
            compute: { cpuCores: { max: 1 }, memory: { max: "512MB" } },
          },
        }),
      },
      {
        id: "proj-token-bridge-policy-manifest",
        name: "policies/simple-token-bridge/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "simple-token-bridge-policy",
            entrypoint: "policy.js",
            type: "policy",
          },
          null,
          2,
        ),
      },
    ],
  },
  {
    id: "proj-ai-bridge",
    name: "4-App-AI-Asset-Bridge",
    files: [
      {
        id: "proj-ai-bridge-worker",
        name: "workers/ai-asset-bridge/worker.js",
        language: "javascript",
        content: aiAssetBridgeJs,
      },
      {
        id: "proj-ai-bridge-worker-manifest",
        name: "workers/ai-asset-bridge/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "ai-asset-bridge",
            entrypoint: "worker.js",
            version: "1.0.0",
            type: "worker",
            events: [{ name: "bridgeAssetWithAI", sourceChain: "polygon" }],
          },
          null,
          2,
        ),
      },
      {
        id: "proj-ai-bridge-policy",
        name: "policies/ai-asset-bridge/policy.js",
        language: "javascript",
        content: createPolicyJsContent({
          permissions: {
            network: { allow: ["api.ai-service.com"] },
            compute: { cpuCores: { max: 2 }, memory: { max: "1GB" } },
          },
        }),
      },
      {
        id: "proj-ai-bridge-policy-manifest",
        name: "policies/ai-asset-bridge/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "ai-asset-bridge-policy",
            entrypoint: "policy.js",
            type: "policy",
          },
          null,
          2,
        ),
      },
    ],
  },
];

export const DEMO_PROJECT_ID = "proj-demo";
const DEMO_PROJECT_NAME = "Demo";

const demoFileConfigurations = [
  {
    projectSourceId: "proj-sec-work",
    type: "policies",
    demoSubfolder: "secure",
  },
  {
    projectSourceId: "proj-sec-fail",
    type: "policies",
    demoSubfolder: "failing",
  },
  {
    projectSourceId: "proj-token-bridge",
    type: "workers",
    demoSubfolder: "bridge",
  },
  {
    projectSourceId: "proj-ai-bridge",
    type: "workers",
    demoSubfolder: "ai",
  },
];

const allDemoFiles: CodeFile[] = [];

for (const config of demoFileConfigurations) {
  const sourceProject = originalProjectTemplates.find(
    (p) => p.id === config.projectSourceId,
  );
  if (!sourceProject) {
    continue;
  }

  sourceProject.files.forEach((file) => {
    const originalPathParts = file.name.split("/");
    const originalFileTypeFolder = originalPathParts[0]; // "workers" or "policies"
    const fileName = originalPathParts.slice(2).join("/"); // e.g. "worker.js" or "manifest.json"

    if (originalFileTypeFolder === config.type) {
      allDemoFiles.push({
        ...file,
        id: `demo-${file.id}`,
        name: `${config.type}/${config.demoSubfolder}/${fileName}`,
      });
    }
  });
}

export const projects: Project[] = [
  {
    id: DEMO_PROJECT_ID,
    name: DEMO_PROJECT_NAME,
    files: allDemoFiles,
  },
];
