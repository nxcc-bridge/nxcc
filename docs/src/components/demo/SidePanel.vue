<script setup lang="ts">
import FileExplorer from './FileExplorer.vue';
import DeployPanel from './DeployPanel.vue';
import type { Project, CodeFile } from './types';

defineProps<{
  activeView: 'explorer' | 'deploy';
  projects: Project[];
}>();

defineEmits<{
  (e: 'open-file', file: CodeFile, project: Project): void;
  (e: 'deploy', project: Project): void;
}>();
</script>

<template>
  <div class="bg-slate-800 h-full overflow-y-auto">
    <FileExplorer
      v-if="activeView === 'explorer'"
      :projects="projects"
      @open-file="(file, project) => $emit('open-file', file, project)"
    />
    <DeployPanel
      v-else-if="activeView === 'deploy'"
      :projects="projects"
      @deploy="(project) => $emit('deploy', project)"
    />
  </div>
</template>
