<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue';
import { Splitpanes, Pane } from 'splitpanes';
import 'splitpanes/dist/splitpanes.css';

import ActivityBar from './ActivityBar.vue';
import SidePanel from './SidePanel.vue';
import EditorPanel from './EditorPanel.vue';
import BottomPanel from './BottomPanel.vue';
import type { CodeFile, Project } from './types';
import { projects as projectTemplates } from './project-templates';

type View = 'explorer' | 'deploy';

const WORKSPACE_STORAGE_KEY = 'remix-ide-clone-workspace';

const projects = ref<Project[]>([]);
const activeView = ref<View>('explorer');
const openFiles = ref<CodeFile[]>([]);
const activeFileId = ref<string | null>(null);

const isBottomPanelOpen = ref(false);
const deploymentLogs = ref<string[]>([]);

const editorPaneSize = computed(() => (isBottomPanelOpen.value ? 70 : 100));
const bottomPaneSize = computed(() => (isBottomPanelOpen.value ? 30 : 0));

onMounted(() => {
  const savedWorkspace = localStorage.getItem(WORKSPACE_STORAGE_KEY);
  if (savedWorkspace) {
    projects.value = JSON.parse(savedWorkspace);
  } else {
    projects.value = JSON.parse(JSON.stringify(projectTemplates));
  }
});

function saveWorkspace() {
  localStorage.setItem(WORKSPACE_STORAGE_KEY, JSON.stringify(projects.value));
}

watch(isBottomPanelOpen, (isOpen) => {
  if (!isOpen) {
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

function handleFileContentUpdate({
  fileId,
  content,
}: {
  fileId: string;
  content: string;
}) {
  for (const project of projects.value) {
    const file = project.files.find((f) => f.id === fileId);
    if (file) {
      if (file.content !== content) {
        file.content = content;
        if (!file.isModified) {
          file.isModified = true;
        }
      }
      return;
    }
  }
}

function handleFileSave(fileId: string) {
  for (const project of projects.value) {
    const file = project.files.find((f) => f.id === fileId);
    if (file && file.isModified) {
      file.isModified = false;
      saveWorkspace();
      return;
    }
  }
}

function handleResetWorkspace() {
  if (
    confirm(
      'Are you sure you want to reset the workspace? All your changes will be lost.'
    )
  ) {
    localStorage.removeItem(WORKSPACE_STORAGE_KEY);
    window.location.reload();
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
          :projects="projects"
          @open-file="handleFileOpen"
          @deploy="handleDeploy"
          @reset-workspace="handleResetWorkspace"
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
              @update:file-content="handleFileContentUpdate"
              @save-file="handleFileSave"
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
