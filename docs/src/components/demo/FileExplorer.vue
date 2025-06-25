<script setup lang="ts">
import { ref } from 'vue';
import {
  Folder,
  FolderOpen,
  File,
  FileJson,
  FileCode2,
  RotateCcw,
} from 'lucide-vue-next';
import type { Project, CodeFile } from './types';

defineProps<{
  projects: Project[];
}>();

const emit = defineEmits<{
  (e: 'open-file', file: CodeFile, project: Project): void;
  (e: 'reset-workspace'): void;
}>();

const openFolders = ref<Record<string, boolean>>({});

function toggleFolder(projectId: string) {
  openFolders.value[projectId] = !openFolders.value[projectId];
}

function getFileIcon(lang: CodeFile['language']) {
  if (lang === 'json') return FileJson;
  if (lang === 'javascript') return FileCode2;
  return File;
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
    <ul>
      <li v-for="project in projects" :key="project.id" class="text-sm">
        <div
          @click="toggleFolder(project.id)"
          class="flex items-center gap-2 p-1 rounded-md cursor-pointer hover:bg-slate-700"
        >
          <component
            :is="openFolders[project.id] ? FolderOpen : Folder"
            :size="16"
            class="text-amber-400"
          />
          <span>{{ project.name }}</span>
        </div>
        <ul v-if="openFolders[project.id]" class="pl-4">
          <li
            v-for="file in project.files"
            :key="file.id"
            @click="emit('open-file', file, project)"
            class="flex items-center gap-2 p-1 rounded-md cursor-pointer hover:bg-slate-700"
          >
            <component
              :is="getFileIcon(file.language)"
              :size="16"
              :class="
                file.isModified ? 'text-amber-400' : 'text-slate-400'
              "
            />
            <span :class="{ 'text-amber-400': file.isModified }">{{
              file.name
            }}</span>
          </li>
        </ul>
      </li>
    </ul>
  </div>
</template>
