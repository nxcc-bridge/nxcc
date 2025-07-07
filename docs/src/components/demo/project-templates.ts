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

// --- New content for the Simple Token Bridge example ---
const simpleTokenBridgeSol = `// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

// This contract is both the token and the bridge vault.
// On the source chain, a user calls \`initiateTransfer\` to burn their tokens and emit an event.
// On the destination chain, the worker calls \`completeTransfer\` to mint new tokens for the recipient.
// This is a simplified Burn/Mint bridge model.
contract SimpleBridgeToken is ERC20, Ownable {

    // Event that the off-chain worker will listen for.
    event BridgeTransfer(
        address indexed sender,
        uint256 amount,
        bytes destinationAddress // The recipient address on the destination chain.
    );

    constructor(
        string memory name,
        string memory symbol,
        address initialOwner
    ) ERC20(name, symbol) Ownable(initialOwner) {
        // Mint some initial tokens to the deployer for testing.
        _mint(msg.sender, 1000 * (10**decimals()));
    }

    // A user calls this on the source chain to start the bridging process.
    // It burns their tokens and emits an event for the worker to pick up.
    function initiateTransfer(uint256 amount, bytes calldata destinationAddress) public {
        require(amount > 0, "Amount must be greater than zero");
        require(balanceOf(msg.sender) >= amount, "Insufficient balance");

        // Burn the tokens from the sender on the source chain.
        _burn(msg.sender, amount);

        // Emit the event that the worker will be listening for.
        emit BridgeTransfer(msg.sender, amount, destinationAddress);
    }

    // The worker calls this on the destination chain to complete the transfer.
    // It mints new tokens for the final recipient.
    // Only the owner (the worker) can call this function.
    function completeTransfer(address recipient, uint256 amount) public onlyOwner {
        require(amount > 0, "Amount must be greater than zero");

        // Mint the equivalent amount of tokens to the recipient on the destination chain.
        _mint(recipient, amount);
    }
}
`;

const simpleTokenBridgeJs = `import { ethers } from "ethers";

export default {
  async fetch(request, env) {
    const userConfig = env.USER_CONFIG;
    if (!userConfig) {
      return new Response("USER_CONFIG not found in environment.", { status: 400 });
    }

    const { handler, event_payload } = await request.json();
    console.log(\`Invoked for handler: '\${handler}'\`);

    let destinationChain;
    if (handler === "sepoliaTransfer") {
      destinationChain = userConfig.opChain;
    } else if (handler === "opTransfer") {
      destinationChain = userConfig.sepoliaChain;
    }

    if (!destinationChain) {
      return new Response(\`Handler '\${handler}' ignored.\`, { status: 200 });
    }

    try {
      const [amount, destAddressBytes] = ethers.AbiCoder.defaultAbiCoder().decode(
        ["uint256", "bytes"],
        event_payload.data
      );
      const recipient = ethers.getAddress(ethers.dataSlice(destAddressBytes, 0, 20));

      console.log(\`Processing transfer of \${amount} to \${recipient} on \${destinationChain.rpcUrl}\`);

      const provider = new ethers.JsonRpcProvider(destinationChain.rpcUrl);
      const wallet = new ethers.Wallet(env.secrets.ethereumPrivateKey, provider);
      const contract = new ethers.Contract(
        destinationChain.contractAddress,
        JSON.parse(userConfig.contractAbi),
        wallet
      );

      const tx = await contract.completeTransfer(recipient, amount);
      const receipt = await tx.wait(1);

      console.log(\`Transaction successful: \${receipt.hash}\`);
      return new Response(JSON.stringify({ success: true, txHash: receipt.hash }));
    } catch (e) {
      console.error(\`Transfer failed: \${e.message}\`);
      return new Response(JSON.stringify({ success: false, error: e.message }), { status: 500 });
    }
  },
};
`;

const simpleTokenBridgeManifestObject = {
  bundle: { source: "file:./worker.js" },
  userdata: {
    sepoliaChain: {
      rpcUrl: "https://rpc.sepolia.org",
      contractAddress: "<YOUR_SEPOLIA_BRIDGE_CONTRACT_ADDRESS>",
    },
    opChain: {
      rpcUrl: "https://sepolia.optimism.io",
      contractAddress: "<YOUR_OP_SEPOLIA_BRIDGE_CONTRACT_ADDRESS>",
    },
    contractAbi:
      '[{"name":"BridgeTransfer","type":"event","anonymous":false,"inputs":[{"indexed":true,"name":"sender","type":"address"},{"indexed":false,"name":"amount","type":"uint256"},{"indexed":false,"name":"destinationAddress","type":"bytes"}]},{\"name\":\"completeTransfer\",\"type\":\"function\",\"stateMutability\":\"nonpayable\",\"inputs\":[{\"name\":\"recipient\",\"type\":\"address\"},{\"name\":\"amount\",\"type\":\"uint256\"}],\"outputs\":[]}]',
  },
  events: [
    {
      handler: "sepoliaTransfer",
      kind: "web3_event",
      chain: 11155111,
      address: ["<YOUR_SEPOLIA_BRIDGE_CONTRACT_ADDRESS>"],
      topics: [
        ["0x524d27dc6154634a87570a22f34b953a55834a3674955215707a8a541249e436"],
      ],
    },
    {
      handler: "opTransfer",
      kind: "web3_event",
      chain: 11155420,
      address: ["<YOUR_OP_SEPOLIA_BRIDGE_CONTRACT_ADDRESS>"],
      topics: [
        ["0x524d27dc6154634a87570a22f34b953a55834a3674955215707a8a541249e436"],
      ],
    },
  ],
};

const simpleTokenBridgePolicyObject = {
  permissions: {
    network: {
      allow: ["rpc.sepolia.org", "sepolia.optimism.io"],
    },
    compute: {
      cpuCores: { max: 1 },
      memory: { max: "512MB" },
    },
  },
};

// --- New content for the AI NFT Mover example ---
const bridgeableNftSol = `// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

// This contract is its own bridge vault.
// Locking an NFT means transferring it to this contract's address.
// Unlocking it means the owner (the worker) transfers it from the contract to a recipient.
contract BridgeableNFT is ERC721, Ownable {
    constructor(
        string memory name,
        string memory symbol,
        address initialOwner
    ) ERC721(name, symbol) Ownable(initialOwner) {}

    // A user calls this to initiate a bridge transfer.
    // It locks their NFT inside this contract.
    function lock(uint256 tokenId) public {
        // Only the current owner of the token can lock it.
        require(
            _isApprovedOrOwner(msg.sender, tokenId),
            "Not approved or owner"
        );
        // Transfer the NFT from the user to this contract.
        _transfer(msg.sender, address(this), tokenId);
    }

    // The bridge worker calls this to complete the transfer on the destination chain.
    function unlock(address recipient, uint256 tokenId) public onlyOwner {
        // The contract must own the token to unlock it.
        require(ownerOf(tokenId) == address(this), "Token not locked");
        // Transfer the NFT from this contract to the final recipient.
        _transfer(address(this), recipient, tokenId);
    }

    // Helper to mint the initial set of NFTs for the demo.
    function mint(address to, uint256 tokenId) public onlyOwner {
        _mint(to, tokenId);
    }
}
`;

const aiNftMoverWorkerJs = `import { ethers } from "ethers";
import OpenAI from "openai";

export default {
  async fetch(request, env) {
    if (request.method !== "POST") {
      return new Response("Method Not Allowed", { status: 405 });
    }

    try {
      // 1. Get config, secrets, and user prompt
      const config = env.USER_CONFIG;
      const secrets = env.secrets;
      const { prompt } = await request.json();
      if (!config || !secrets || !prompt) {
        throw new Error("Missing config, secrets, or prompt");
      }

      // 2. Call LLM to decide the move direction
      const openai = new OpenAI({ apiKey: secrets.openaiApiKey });
      const llmResponse = await openai.chat.completions.create({
        model: "gpt-4o-mini",
        response_format: { type: "json_object" },
        messages: [
          {
            role: "system",
            content: 'You are an NFT bridge decider. Based on the user\\'s request, determine the source and destination. Respond ONLY with JSON like {"source": "sepolia", "destination": "opTestnet"}. If invalid, respond with {"error": "reason"}.',
          },
          { role: "user", content: prompt },
        ],
      });

      const decision = JSON.parse(llmResponse.choices[0].message.content);
      console.log("LLM Decision:", decision);
      if (decision.error) throw new Error(\`LLM Error: \${decision.error}\`);

      // --- 3. LOCK NFT ON SOURCE CHAIN ---
      const sourceConfig = config.chains[decision.source];
      const sourceProvider = new ethers.JsonRpcProvider(sourceConfig.rpcUrl);
      const wallet = new ethers.Wallet(secrets.ethereumPrivateKey, sourceProvider);
      const sourceContract = new ethers.Contract(sourceConfig.contractAddress, config.contractAbi, wallet);
      
      console.log(\`Locking token \${config.tokenId} on \${decision.source}...\`);
      const lockTx = await sourceContract.lock(config.tokenId);
      const lockReceipt = await lockTx.wait(1);
      console.log(\`Lock successful. Tx: \${lockReceipt.hash}\`);

      // --- 4. UNLOCK NFT ON DESTINATION CHAIN ---
      const destConfig = config.chains[decision.destination];
      const destProvider = new ethers.JsonRpcProvider(destConfig.rpcUrl);
      const destWallet = new ethers.Wallet(secrets.ethereumPrivateKey, destProvider);
      const destContract = new ethers.Contract(destConfig.contractAddress, config.contractAbi, destWallet);
      
      console.log(\`Unlocking token \${config.tokenId} on \${decision.destination}...\`);
      const unlockTx = await destContract.unlock(config.recipientAddress, config.tokenId);
      const unlockReceipt = await unlockTx.wait(1);
      console.log(\`Unlock successful. Tx: \${unlockReceipt.hash}\`);

      // 5. Return success
      return new Response(JSON.stringify({
          success: true,
          message: \`Bridged NFT from \${decision.source} to \${decision.destination}\`,
          lockTx: lockReceipt.hash,
          unlockTx: unlockReceipt.hash,
        }), { headers: { "Content-Type": "application/json" } }
      );

    } catch (error) {
      console.error("Worker failed:", error.message);
      return new Response(JSON.stringify({ success: false, error: error.message }), {
        status: 500, headers: { "Content-Type": "application/json" },
      });
    }
  },
};
`;

const aiNftMoverManifestObject = {
  bundle: {
    source: "file:worker.js",
  },
  events: [
    {
      handler: "fetch",
      kind: "http_request_trigger",
    },
  ],
  userdata: {
    recipientAddress: "<ADDRESS_TO_RECEIVE_THE_NFT_ON_DESTINATION_CHAIN>",
    tokenId: "1",
    contractAbi:
      '[{"inputs":[{"internalType":"address","name":"recipient","type":"address"},{"internalType":"uint256","name":"tokenId","type":"uint256"}],"name":"unlock","outputs":[],"stateMutability":"nonpayable","type":"function"},{"inputs":[{"internalType":"uint256","name":"tokenId","type":"uint256"}],"name":"lock","outputs":[],"stateMutability":"nonpayable","type":"function"}]',
    chains: {
      sepolia: {
        rpcUrl: "https://rpc.sepolia.org",
        contractAddress: "<YOUR_CONTRACT_ADDRESS_ON_SEPOLIA>",
      },
      opTestnet: {
        rpcUrl: "https://sepolia.optimism.io",
        contractAddress: "<YOUR_CONTRACT_ADDRESS_ON_OP_SEPOLIA>",
      },
    },
  },
};

const aiNftMoverPolicyObject = {
  permissions: {
    network: {
      allow: ["api.openai.com", "rpc.sepolia.org", "sepolia.optimism.io"],
    },
    compute: {
      cpuCores: { max: 1 },
      memory: { max: "512MB" },
    },
  },
};

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
        id: "proj-token-bridge-contract",
        name: "contracts/simple-token-bridge/SimpleBridgeToken.sol",
        language: "solidity",
        content: simpleTokenBridgeSol,
      },
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
        content: JSON.stringify(simpleTokenBridgeManifestObject, null, 2),
      },
      {
        id: "proj-token-bridge-policy",
        name: "policies/simple-token-bridge/policy.js",
        language: "javascript",
        content: createPolicyJsContent(simpleTokenBridgePolicyObject),
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
    name: "4-App-AI-NFT-Mover",
    files: [
      {
        id: "proj-ai-mover-contract",
        name: "contracts/ai-nft-mover/BridgeableNFT.sol",
        language: "solidity",
        content: bridgeableNftSol,
      },
      {
        id: "proj-ai-mover-worker",
        name: "workers/ai-nft-mover/worker.js",
        language: "javascript",
        content: aiNftMoverWorkerJs,
      },
      {
        id: "proj-ai-mover-worker-manifest",
        name: "workers/ai-nft-mover/manifest.json",
        language: "json",
        content: JSON.stringify(aiNftMoverManifestObject, null, 2),
      },
      {
        id: "proj-ai-mover-policy",
        name: "policies/ai-nft-mover/policy.js",
        language: "javascript",
        content: createPolicyJsContent(aiNftMoverPolicyObject),
      },
      {
        id: "proj-ai-mover-policy-manifest",
        name: "policies/ai-nft-mover/manifest.json",
        language: "json",
        content: JSON.stringify(
          {
            name: "ai-nft-mover-policy",
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
    type: "contracts",
    demoSubfolder: "bridge",
  },
  {
    projectSourceId: "proj-token-bridge",
    type: "workers",
    demoSubfolder: "bridge",
  },
  {
    projectSourceId: "proj-ai-bridge",
    type: "contracts",
    demoSubfolder: "ai",
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
    const originalFileTypeFolder = originalPathParts[0]; // "workers", "policies", or "contracts"
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
