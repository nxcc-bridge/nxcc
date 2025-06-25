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
const activeProjectId = ref<string | null>(null);
const activeProject = computed(() => {
  return projects.value.find((p) => p.id === activeProjectId.value) ?? null;
});

const activeView = ref<View>('explorer');
const openFiles = ref<CodeFile[]>([]);
const activeFileId = ref<string | null>(null);

const isBottomPanelOpen = ref(false);
const deploymentLogs = ref<string[]>([]);

const editorPaneSize = computed(() => (isBottomPanelOpen.value ? 70 : 100));
const bottomPaneSize = computed(() => (isBottomPanelOpen.value ? 30 : 0));

onMounted(() => {
  const savedWorkspace = localStorage.getItem(WORKSPACE_STORAGE_KEY);
  projects.value = savedWorkspace
    ? JSON.parse(savedWorkspace)
    : JSON.parse(JSON.stringify(projectTemplates));

  if (projects.value.length > 0) {
    activeProjectId.value = projects.value[0].id;
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
    activeFileId.value =
      openFiles.value[index]?.id ??
      openFiles.value[index - 1]?.id ??
      null;
  }
}

function handleFileContentUpdate({
  fileId,
  content,
}: {
  fileId: string;
  content: string;
}) {
  if (!activeProject.value) return;
  const file = activeProject.value.files.find((f) => f.id === fileId);
  if (file && file.content !== content) {
    file.content = content;
    if (!file.isModified) {
      file.isModified = true;
    }
  }
}

function handleFileSave(fileId: string) {
  if (!activeProject.value) return;
  const file = activeProject.value.files.find((f) => f.id === fileId);
  if (file && file.isModified) {
    file.isModified = false;
    saveWorkspace();
  }
}

function saveAllModifiedFiles() {
  if (!activeProject.value) return;

  let wasModified = false;
  for (const file of activeProject.value.files) {
    if (file.isModified) {
      file.isModified = false;
      wasModified = true;
    }
  }

  if (wasModified) {
    saveWorkspace();
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

function switchProject(newProjectId: string) {
  if (newProjectId === activeProjectId.value) {
    return;
  }

  saveAllModifiedFiles();

  openFiles.value = [];
  activeFileId.value = null;

  activeProjectId.value = newProjectId;
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

function handleCreateProject() {
  const name = prompt('Enter new project name:');
  if (!name) return;

  const newProjectId = `proj-user-${Date.now()}`;
  const slugName = name.toLowerCase().replace(/\s+/g, '-');

  const newProject: Project = {
    id: newProjectId,
    name: name,
    files: [
      {
        id: `${newProjectId}-app`,
        name: 'app.js',
        language: 'javascript',
        content: `/**
 * A new app.
 */
function main() {
  console.log("Hello from your new secure app: ${name}!");
}

main();
`,
      },
      {
        id: `${newProjectId}-policy`,
        name: 'policy.json',
        language: 'json',
        content: JSON.stringify(
          {
            description: 'A default policy for a new project.',
            permissions: {
              filesystem: { access: 'none' },
              network: { allow: [] },
              compute: {
                cpuCores: { max: 1 },
                memory: { max: '256MB' },
              },
            },
          },
          null,
          2
        ),
      },
      {
        id: `${newProjectId}-manifest`,
        name: 'manifest.json',
        language: 'json',
        content: JSON.stringify(
          { name: slugName, entrypoint: 'app.js', version: '1.0.0' },
          null,
          2
        ),
      },
    ],
  };

  projects.value.push(newProject);
  saveWorkspace();
  switchProject(newProjectId);
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
          :active-project-id="activeProjectId"
          @open-file="handleFileOpen"
          @deploy="handleDeploy"
          @reset-workspace="handleResetWorkspace"
          @switch-project="switchProject"
          @create-project="handleCreateProject"
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
