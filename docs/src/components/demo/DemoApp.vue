<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { Splitpanes, Pane } from 'splitpanes';
import 'splitpanes/dist/splitpanes.css';

import ActivityBar from './ActivityBar.vue';
import SidePanel from './SidePanel.vue';
import EditorPanel from './EditorPanel.vue';
import BottomPanel from './BottomPanel.vue';
import type { CodeFile, Project } from './types';
import { projects as projectTemplates } from './project-templates';

type View = 'explorer' | 'deploy';

const activeView = ref<View>('explorer');
const openFiles = ref<CodeFile[]>([]);
const activeFileId = ref<string | null>(null);

const isBottomPanelOpen = ref(false);
const deploymentLogs = ref<string[]>([]);

const editorPaneSize = computed(() => (isBottomPanelOpen.value ? 70 : 100));
const bottomPaneSize = computed(() => (isBottomPanelOpen.value ? 30 : 0));

watch(isBottomPanelOpen, (isOpen) => {
  if (!isOpen) {
    // Give the pane time to slide away before clearing logs
    setTimeout(() => {
      deploymentLogs.value = [];
    }, 300);
  }
});

function handleFileOpen(file: CodeFile) {
  if (!openFiles.value.some((f) => f.id === file.id)) {
    openFiles.value.push(file);
  }
  activeFileId.value = file.id;
}

function handleFileClose(fileId: string) {
  const index = openFiles.value.findIndex((f) => f.id === fileId);
  if (index === -1) return;

  openFiles.value.splice(index, 1);

  if (activeFileId.value === fileId) {
    activeFileId.value = openFiles.value[index - 1]?.id ?? null;
  }
}

function handleDeploy(project: Project) {
  isBottomPanelOpen.value = true;
  deploymentLogs.value = [];

  const addLog = (msg: string) => {
    deploymentLogs.value.push(`[${new Date().toLocaleTimeString()}] ${msg}`);
  };

  addLog(`Starting deployment for project: ${project.name}`);

  setTimeout(() => addLog('Fetching project files...'), 500);
  setTimeout(() => addLog('Compiling application...'), 1000);
  setTimeout(() => addLog('Validating policy...'), 1500);

  if (project.id === 'proj-sec-fail') {
    setTimeout(() => addLog('❌ Policy validation FAILED.'), 2000);
    setTimeout(
      () =>
        addLog('Error: App does not declare required network permissions.'),
      2100
    );
    setTimeout(() => addLog('Deployment aborted.'), 2500);
  } else {
    setTimeout(() => addLog('✅ Policy validation PASSED.'), 2000);
    setTimeout(() => addLog('Deploying to secure node...'), 2500);
    setTimeout(() => addLog('🚀 Deployment successful!'), 3000);
    setTimeout(() => addLog('Application is now running.'), 3500);
  }
}

// TODO: Implement Shepherd.js tour
function handleStartTour() {
  alert('The interactive guided tour is coming soon!');
}
</script>

<template>
  <div class="flex h-screen w-full bg-slate-900 text-slate-100">
    <ActivityBar
      :active-view="activeView"
      @update:active-view="activeView = $event"
      @start-tour="handleStartTour"
    />

    <Splitpanes class="default-theme w-full">
      <Pane :size="20" :min-size="15" class="h-full">
        <SidePanel
          :active-view="activeView"
          :projects="projectTemplates"
          @open-file="handleFileOpen"
          @deploy="handleDeploy"
        />
      </Pane>
      <Pane :size="80" class="h-full">
        <Splitpanes
          horizontal
          :push-other-panes="false"
          class="h-full"
          @resized="
            (e) => {
              if (e[1].size < 2) isBottomPanelOpen = false;
            }
          "
        >
          <Pane :size="editorPaneSize" class="h-full">
            <EditorPanel
              :open-files="openFiles"
              :active-file-id="activeFileId"
              @update:active-file-id="activeFileId = $event"
              @close-file="handleFileClose"
            />
          </Pane>
          <Pane :size="bottomPaneSize" class="h-full">
            <BottomPanel
              v-if="isBottomPanelOpen"
              :deployment-logs="deploymentLogs"
            />
          </Pane>
        </Splitpanes>
      </Pane>
    </Splitpanes>
  </div>
</template>

<style>
.splitpanes__splitter {
  background-color: #2d3748; /* slate-700 */
  position: relative;
}
.splitpanes--vertical > .splitpanes__splitter {
  width: 1px;
}
.splitpanes--horizontal > .splitpanes__splitter {
  height: 1px;
}
.splitpanes__pane {
  background-color: #1e293b; /* slate-800, should be overridden by components */
}
</style>
