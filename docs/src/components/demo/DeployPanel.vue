<script setup lang="ts">
import { ref, computed, watch } from "vue";
import type { Project, CodeFile } from "./types";

interface DeployableManifest {
  id: string; // Unique ID, e.g., manifest file path
  path: string; // Manifest file path
  name: string; // Name from manifest or derived
  type: "worker" | "policy";
  entrypoint: string; // Relative entrypoint path from manifest
  project: Project;
  manifestData: Record<string, any>; // Parsed manifest content
}

const props = defineProps<{
  projects: Project[];
}>();

const emit = defineEmits<{
  (e: "deploy", project: Project): void;
}>();

const selectedProjectId = ref<string>(
  props.projects.length > 0 ? props.projects[0].id : "",
);
const selectedManifestId = ref<string | null>(null);

const selectedChainId = ref<string>(""); // For policies
const nodeUrl = ref<string>(""); // For workers
const isWalletConnected = ref<boolean>(false); // For policies

const selectedProject = computed((): Project | undefined => {
  return props.projects.find((p) => p.id === selectedProjectId.value);
});

const deployableManifests = computed((): DeployableManifest[] => {
  if (!selectedProject.value) return [];

  const manifests: DeployableManifest[] = [];
  for (const file of selectedProject.value.files) {
    if (file.name.endsWith("manifest.json") && file.language === "json") {
      try {
        const manifestContent = JSON.parse(file.content);
        if (
          manifestContent &&
          typeof manifestContent.entrypoint === "string" &&
          (manifestContent.type === "worker" ||
            manifestContent.type === "policy")
        ) {
          const manifestName =
            manifestContent.name || file.name.split("/").slice(-2, -1)[0];
          manifests.push({
            id: `${selectedProject.value.id}-${file.name}`,
            path: file.name,
            name: manifestName,
            type: manifestContent.type,
            entrypoint: manifestContent.entrypoint,
            project: selectedProject.value,
            manifestData: manifestContent,
          });
        }
      } catch (error) {
        // TODO: Consider logging parsing errors if a debug mode is available
        console.warn(`Failed to parse manifest ${file.name}:`, error);
      }
    }
  }
  return manifests;
});

const selectedManifest = computed((): DeployableManifest | undefined => {
  return deployableManifests.value.find(
    (m) => m.id === selectedManifestId.value,
  );
});

watch(selectedProjectId, () => {
  selectedManifestId.value = null;
  if (deployableManifests.value.length > 0) {
    selectedManifestId.value = deployableManifests.value[0].id;
  }
});

watch(deployableManifests, (newList) => {
  if (
    selectedManifestId.value &&
    !newList.find((item) => item.id === selectedManifestId.value)
  ) {
    selectedManifestId.value = null;
  }
  if (!selectedManifestId.value && newList.length > 0) {
    selectedManifestId.value = newList[0].id;
  }
});

watch(selectedManifestId, () => {
  // Reset specific configs when manifest selection changes
  selectedChainId.value = "";
  nodeUrl.value = "";
  isWalletConnected.value = false;
});

const chainOptions = [
  { id: "1", name: "Ethereum Mainnet" },
  { id: "5", name: "Goerli Testnet" },
  { id: "11155111", name: "Sepolia Testnet" },
  { id: "137", name: "Polygon Mainnet" },
];

function handleDeploy() {
  if (selectedProject.value && selectedManifest.value) {
    // The deploy event still emits the project, as per instruction.
    // The parent component would need to be aware of the selected manifest
    // and its configuration if it needs to use them.
    // For now, this component holds that state.
    emit("deploy", selectedProject.value);
  } else {
    alert("Please select a project and a deployable item.");
  }
}

function connectWallet() {
  // Mock wallet connection
  isWalletConnected.value = true;
  alert("Wallet connected (simulated).");
}

// Initialize selectedManifestId if deployableManifests is already populated
if (deployableManifests.value.length > 0 && !selectedManifestId.value) {
  selectedManifestId.value = deployableManifests.value[0].id;
}
</script>

<template>
  <div class="p-4 text-slate-300">
    <h3 class="text-sm font-bold uppercase text-slate-500 mb-4 tracking-wider">
      Run & Deploy
    </h3>
    <div class="space-y-6">
      <div>
        <label for="project-select" class="block text-sm font-medium mb-1"
          >Project</label
        >
        <select
          id="project-select"
          v-model="selectedProjectId"
          class="w-full p-2 bg-slate-700 border border-slate-600 rounded-md focus:outline-none focus:ring-2 focus:ring-amber-400"
        >
          <option
            v-for="project in projects"
            :key="project.id"
            :value="project.id"
          >
            {{ project.name }}
          </option>
        </select>
      </div>

      <div v-if="selectedProject">
        <label for="deployable-select" class="block text-sm font-medium mb-1"
          >Deployable Item (Worker/Policy)</label
        >
        <select
          id="deployable-select"
          v-model="selectedManifestId"
          class="w-full p-2 bg-slate-700 border border-slate-600 rounded-md focus:outline-none focus:ring-2 focus:ring-amber-400"
          :disabled="deployableManifests.length === 0"
        >
          <option
            v-if="deployableManifests.length === 0"
            :value="null"
            disabled
          >
            No deployable items found in project
          </option>
          <option
            v-for="manifest in deployableManifests"
            :key="manifest.id"
            :value="manifest.id"
          >
            {{ manifest.name }} ({{ manifest.type }}) -
            {{ manifest.path }}
          </option>
        </select>
      </div>

      <div v-if="selectedManifest">
        <h4 class="text-xs font-bold uppercase text-slate-400 mb-2">
          Deployment Configuration
        </h4>
        <div
          v-if="selectedManifest.type === 'policy'"
          class="space-y-3 p-3 bg-slate-700/50 rounded-md"
        >
          <div>
            <label for="chain-id-select" class="block text-xs font-medium mb-1"
              >Chain ID</label
            >
            <select
              id="chain-id-select"
              v-model="selectedChainId"
              class="w-full p-2 bg-slate-600 border border-slate-500 rounded-md text-sm focus:outline-none focus:ring-1 focus:ring-amber-400"
            >
              <option value="" disabled>Select Chain ID</option>
              <option
                v-for="chain in chainOptions"
                :key="chain.id"
                :value="chain.id"
              >
                {{ chain.name }} ({{ chain.id }})
              </option>
            </select>
          </div>
          <div>
            <button
              @click="connectWallet"
              :disabled="isWalletConnected"
              class="w-full px-3 py-2 text-sm font-semibold rounded-md transition-colors"
              :class="
                isWalletConnected
                  ? 'bg-green-600 text-white cursor-not-allowed'
                  : 'bg-sky-500 text-white hover:bg-sky-400 focus:outline-none focus:ring-2 focus:ring-sky-400 focus:ring-offset-2 focus:ring-offset-slate-800'
              "
            >
              {{ isWalletConnected ? "Wallet Connected" : "Connect Wallet" }}
            </button>
          </div>
        </div>

        <div
          v-if="selectedManifest.type === 'worker'"
          class="space-y-3 p-3 bg-slate-700/50 rounded-md"
        >
          <div>
            <label for="node-url-input" class="block text-xs font-medium mb-1"
              >Node URL</label
            >
            <input
              type="url"
              id="node-url-input"
              v-model="nodeUrl"
              placeholder="e.g., https://your-node-provider.com/api"
              class="w-full p-2 bg-slate-600 border border-slate-500 rounded-md text-sm focus:outline-none focus:ring-1 focus:ring-amber-400 placeholder-slate-400"
            />
          </div>
        </div>
      </div>
      <div v-else-if="selectedProject && deployableManifests.length > 0">
        <p class="text-sm text-slate-400">
          Select a worker or policy to configure its deployment.
        </p>
      </div>

      <button
        @click="handleDeploy"
        :disabled="!selectedManifest"
        class="w-full flex items-center justify-center gap-2 px-4 py-2 font-semibold rounded-md bg-amber-500 text-slate-900 hover:bg-amber-400 transition-colors focus:outline-none focus:ring-2 focus:ring-amber-400 focus:ring-offset-2 focus:ring-offset-slate-800 disabled:opacity-50 disabled:cursor-not-allowed"
      >
        Deploy
      </button>
    </div>
  </div>
</template>
