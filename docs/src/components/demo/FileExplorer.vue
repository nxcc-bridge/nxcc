<script setup lang="ts">
import { computed } from "vue";
import { File, FileJson, FileCode2, RotateCcw, Plus } from "lucide-vue-next";
import type { Project, CodeFile } from "./types";

const props = defineProps<{
  projects: Project[];
  activeProjectId: string | null;
}>();

const emit = defineEmits<{
  (e: "open-file", file: CodeFile): void;
  (e: "reset-workspace"): void;
  (e: "switch-project", projectId: string): void;
  (e: "create-project"): void;
}>();

const activeProject = computed(() =>
  props.projects.find((p) => p.id === props.activeProjectId),
);

function getFileIcon(lang: CodeFile["language"]) {
  if (lang === "json") return FileJson;
  if (lang === "javascript") return FileCode2;
  return File;
}

function handleProjectChange(event: Event) {
  const target = event.target as HTMLSelectElement;
  emit("switch-project", target.value);
}
</script>

<template>
  <div class="p-2 text-slate-300 flex flex-col h-full">
    <div class="flex justify-between items-center px-2 mb-2">
      <h3 class="text-sm font-bold uppercase text-slate-500 tracking-wider">
        Explorer
      </h3>
      <button
        @click="$emit('reset-workspace')"
        class="p-1 rounded-md text-slate-400 hover:bg-slate-700 hover:text-amber-400"
        title="Reset Workspace to Original Templates"
      >
        <RotateCcw :size="16" />
      </button>
    </div>

    <div class="px-2 mb-2">
      <label for="project-selector" class="text-xs text-slate-400"
        >Current Project</label
      >
      <div class="flex gap-1 mt-1">
        <select
          id="project-selector"
          :value="activeProjectId"
          @change="handleProjectChange"
          class="w-full p-1 bg-slate-700 border border-slate-600 rounded-md text-sm focus:outline-none focus:ring-1 focus:ring-amber-400"
        >
          <option
            v-for="project in projects"
            :key="project.id"
            :value="project.id"
          >
            {{ project.name }}
          </option>
        </select>
        <button
          @click="$emit('create-project')"
          class="p-1 rounded-md bg-slate-700 hover:bg-slate-600 text-slate-300 shrink-0"
          title="Create new project"
        >
          <Plus :size="20" />
        </button>
      </div>
    </div>
    <ul v-if="activeProject" class="text-sm mt-2 px-2 overflow-y-auto">
      <li
        v-for="file in activeProject.files"
        :key="file.id"
        @click="emit('open-file', file)"
        class="flex items-center gap-2 p-1 rounded-md cursor-pointer hover:bg-slate-700"
      >
        <component
          :is="getFileIcon(file.language)"
          :size="16"
          :class="file.isModified ? 'text-amber-400' : 'text-slate-400'"
        />
        <span :class="{ 'text-amber-400': file.isModified }">{{
          file.name
        }}</span>
      </li>
    </ul>
    <div v-else class="px-2 py-4 text-center text-slate-500">
      <p>No project selected.</p>
      <p>Create a new project to start.</p>
    </div>
  </div>
</template>
