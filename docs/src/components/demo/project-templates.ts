import type { Project } from './types';

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

export const projects: Project[] = [
  {
    id: 'proj-sec-work',
    name: '1-Security-Demo-Working-Policy',
    files: [
      {
        id: 'proj-sec-work-app',
        name: 'app.js',
        language: 'javascript',
        content: simpleAppJs,
      },
      {
        id: 'proj-sec-work-policy',
        name: 'policy.json',
        language: 'json',
        content: JSON.stringify(
          {
            description: 'This policy allows basic execution and should pass.',
            permissions: {
              filesystem: { access: 'none' },
              network: { allow: [] },
              compute: {
                cpuCores: { max: 1 },
                memory: { max: '256MB' },
              },
            },
          },
          null,
          2
        ),
      },
      {
        id: 'proj-sec-work-manifest',
        name: 'manifest.json',
        language: 'json',
        content: JSON.stringify(
          {
            name: 'security-demo-working',
            entrypoint: 'app.js',
            version: '1.0.0',
          },
          null,
          2
        ),
      },
    ],
  },
  {
    id: 'proj-sec-fail',
    name: '2-Security-Demo-Failing-Policy',
    files: [
      {
        id: 'proj-sec-fail-app',
        name: 'app.js',
        language: 'javascript',
        content: simpleAppJs,
      },
      {
        id: 'proj-sec-fail-policy',
        name: 'policy.json',
        language: 'json',
        content: JSON.stringify(
          {
            description:
              'This policy requires network access which the app does not have, causing a mismatch. The job will not run.',
            permissions: {
              filesystem: { access: 'none' },
              network: { allow: ['api.some-service.com'] }, // This requirement will cause the policy check to fail.
              compute: {
                cpuCores: { max: 1 },
                memory: { max: '256MB' },
              },
            },
          },
          null,
          2
        ),
      },
      {
        id: 'proj-sec-fail-manifest',
        name: 'manifest.json',
        language: 'json',
        content: JSON.stringify(
          {
            name: 'security-demo-failing',
            entrypoint: 'app.js',
            version: '1.0.0',
          },
          null,
          2
        ),
      },
    ],
  },
  {
    id: 'proj-token-bridge',
    name: '3-App-Simple-Token-Bridge',
    files: [
      {
        id: 'proj-token-bridge-app',
        name: 'app.js',
        language: 'javascript',
        content: simpleTokenBridgeJs,
      },
      {
        id: 'proj-token-bridge-policy',
        name: 'policy.json',
        language: 'json',
        content: JSON.stringify(
          {
            permissions: {
              compute: { cpuCores: { max: 1 }, memory: { max: '512MB' } },
            },
          },
          null,
          2
        ),
      },
      {
        id: 'proj-token-bridge-manifest',
        name: 'manifest.json',
        language: 'json',
        content: JSON.stringify(
          {
            name: 'simple-token-bridge',
            entrypoint: 'app.js',
            version: '1.0.0',
            events: [{ name: 'bridgeToken', sourceChain: 'ethereum' }],
          },
          null,
          2
        ),
      },
    ],
  },
  {
    id: 'proj-ai-bridge',
    name: '4-App-AI-Asset-Bridge',
    files: [
      {
        id: 'proj-ai-bridge-app',
        name: 'app.js',
        language: 'javascript',
        content: aiAssetBridgeJs,
      },
      {
        id: 'proj-ai-bridge-policy',
        name: 'policy.json',
        language: 'json',
        content: JSON.stringify(
          {
            permissions: {
              network: { allow: ['api.ai-service.com'] },
              compute: { cpuCores: { max: 2 }, memory: { max: '1GB' } },
            },
          },
          null,
          2
        ),
      },
      {
        id: 'proj-ai-bridge-manifest',
        name: 'manifest.json',
        language: 'json',
        content: JSON.stringify(
          {
            name: 'ai-asset-bridge',
            entrypoint: 'app.js',
            version: '1.0.0',
            events: [{ name: 'bridgeAssetWithAI', sourceChain: 'polygon' }],
          },
          null,
          2
        ),
      },
    ],
  },
];
