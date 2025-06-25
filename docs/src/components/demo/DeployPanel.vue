<script setup lang="ts">
import { ref } from 'vue';
import type { Project } from './types';

const props = defineProps<{
  projects: Project[];
}>();

const emit = defineEmits<{
  (e: 'deploy', project: Project): void;
}>();

const selectedProjectId = ref<string>(
  props.projects.length > 0 ? props.projects[0].id : ''
);

function handleDeploy() {
  const projectToDeploy = props.projects.find(
    (p) => p.id === selectedProjectId.value
  );
  if (projectToDeploy) {
    emit('deploy', projectToDeploy);
  }
}
</script>

<template>
  <div class="p-4 text-slate-300">
    <h3
      class="text-sm font-bold uppercase text-slate-500 mb-4 tracking-wider"
    >
      Run & Deploy
    </h3>
    <div class="space-y-4">
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
      <button
        @click="handleDeploy"
        class="w-full flex items-center justify-center gap-2 px-4 py-2 font-semibold rounded-md bg-amber-500 text-slate-900 hover:bg-amber-400 transition-colors focus:outline-none focus:ring-2 focus:ring-amber-400 focus:ring-offset-2 focus:ring-offset-slate-800"
      >
        Deploy
      </button>
    </div>
  </div>
</template>
