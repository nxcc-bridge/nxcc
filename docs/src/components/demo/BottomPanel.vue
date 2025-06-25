<script setup lang="ts">
import { ref } from "vue";

defineProps<{
  deploymentLogs: string[];
}>();

type Tab = "deploy" | "app" | "events";
const activeTab = ref<Tab>("deploy");
</script>

<template>
  <div class="flex flex-col h-full bg-slate-800 text-slate-300">
    <div class="flex-shrink-0 flex items-center border-b border-slate-700">
      <button
        @click="activeTab = 'deploy'"
        class="px-4 py-2 text-sm"
        :class="{
          'bg-slate-900 text-amber-400 border-b-2 border-amber-400':
            activeTab === 'deploy',
          'hover:bg-slate-700': activeTab !== 'deploy',
        }"
      >
        Deployment Logs
      </button>
      <button
        @click="activeTab = 'app'"
        class="px-4 py-2 text-sm"
        :class="{
          'bg-slate-900 text-amber-400 border-b-2 border-amber-400':
            activeTab === 'app',
          'hover:bg-slate-700': activeTab !== 'app',
        }"
      >
        Application Logs
      </button>
      <button
        @click="activeTab = 'events'"
        class="px-4 py-2 text-sm"
        :class="{
          'bg-slate-900 text-amber-400 border-b-2 border-amber-400':
            activeTab === 'events',
          'hover:bg-slate-700': activeTab !== 'events',
        }"
      >
        Chain Event Simulator
      </button>
    </div>
    <div class="flex-grow p-2 overflow-y-auto font-mono text-sm bg-slate-900">
      <div v-if="activeTab === 'deploy'">
        <div v-for="(log, i) in deploymentLogs" :key="i">{{ log }}</div>
      </div>
      <div v-if="activeTab === 'app'">
        <p class="text-slate-500">
          Application logs will appear here once the app is running...
        </p>
      </div>
      <div v-if="activeTab === 'events'">
        <p class="text-slate-500">
          <!-- TODO: Implement event simulator form -->
          Interact with deployed applications by simulating blockchain events.
        </p>
      </div>
    </div>
  </div>
</template>
