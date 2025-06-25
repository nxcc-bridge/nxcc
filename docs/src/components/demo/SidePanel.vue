<script setup lang="ts">
import FileExplorer from './FileExplorer.vue';
import DeployPanel from './DeployPanel.vue';
import type { Project, CodeFile } from './types';

defineProps<{
  activeView: 'explorer' | 'deploy';
  projects: Project[];
  activeProjectId: string | null;
}>();

defineEmits<{
  (e: 'open-file', file: CodeFile): void;
  (e: 'deploy', project: Project): void;
  (e: 'reset-workspace'): void;
  (e: 'switch-project', projectId: string): void;
  (e: 'create-project'): void;
}>();
</script>

<template>
  <div class="bg-slate-800 h-full overflow-y-auto">
    <FileExplorer
      v-if="activeView === 'explorer'"
      :projects="projects"
      :active-project-id="activeProjectId"
      @open-file="(file) => $emit('open-file', file)"
      @reset-workspace="$emit('reset-workspace')"
      @switch-project="(projectId) => $emit('switch-project', projectId)"
      @create-project="$emit('create-project')"
    />
    <DeployPanel
      v-else-if="activeView === 'deploy'"
      :projects="projects"
      @deploy="(project) => $emit('deploy', project)"
    />
  </div>
</template>
