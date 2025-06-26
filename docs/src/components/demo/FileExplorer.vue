<script setup lang="ts">
import { computed, ref } from "vue";
import { RotateCcw, Plus } from "lucide-vue-next";
import type { Project, CodeFile } from "./types";
import FileExplorerNode, { type FileTreeNode } from "./FileExplorerNode.vue";

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

const expandedFolders = ref<Set<string>>(new Set());

function toggleFolder(folderPath: string) {
  if (expandedFolders.value.has(folderPath)) {
    expandedFolders.value.delete(folderPath);
  } else {
    expandedFolders.value.add(folderPath);
  }
}

const fileTree = computed((): FileTreeNode[] => {
  if (!activeProject.value?.files) return [];

  const root: Record<string, any> = {};

  for (const file of activeProject.value.files) {
    const parts = file.name.split("/");
    let currentLevel = root;
    parts.forEach((part, index) => {
      const currentPath = parts.slice(0, index + 1).join("/");
      if (!currentLevel[part]) {
        currentLevel[part] = {
          name: part,
          path: currentPath,
          type: index === parts.length - 1 ? "file" : "folder",
          children: index === parts.length - 1 ? undefined : {},
        };
      }
      if (index === parts.length - 1) {
        currentLevel[part].fileObject = file;
      }
      if (currentLevel[part].type === "folder") {
        currentLevel = currentLevel[part].children;
      }
    });
  }

  function convertAndSortChildren(
    nodeChildrenMap: Record<string, any>,
  ): FileTreeNode[] {
    return Object.values(nodeChildrenMap)
      .map((child: any) => {
        if (child.type === "folder" && child.children) {
          child.children = convertAndSortChildren(child.children);
        }
        return child as FileTreeNode;
      })
      .sort((a, b) => {
        if (a.type !== b.type) {
          return a.type === "folder" ? -1 : 1;
        }
        return a.name.localeCompare(b.name);
      });
  }

  return convertAndSortChildren(root);
});

function handleProjectChange(event: Event) {
  const target = event.target as HTMLSelectElement;
  emit("switch-project", target.value);
  expandedFolders.value.clear(); // Reset expanded state on project switch
}

function handleOpenFile(file: CodeFile) {
  emit("open-file", file);
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

    <ul
      v-if="activeProject && fileTree.length > 0"
      class="text-sm mt-2 px-1 flex-grow overflow-y-auto"
    >
      <FileExplorerNode
        v-for="node in fileTree"
        :key="node.path"
        :node="node"
        :expanded-folders="expandedFolders"
        :level="0"
        @toggle-folder="toggleFolder"
        @open-file="handleOpenFile"
      />
    </ul>
    <div
      v-else-if="activeProject && fileTree.length === 0"
      class="px-2 py-4 text-center text-slate-500 flex-grow"
    >
      <p>This project is empty.</p>
    </div>
    <div v-else class="px-2 py-4 text-center text-slate-500 flex-grow">
      <p>No project selected.</p>
      <p>Create or select a project to start.</p>
    </div>
  </div>
</template>
