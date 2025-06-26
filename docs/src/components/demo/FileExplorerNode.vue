<script setup lang="ts">
import { computed } from "vue";
import {
  File as FileIcon,
  FileJson,
  FileCode2,
  Folder,
  FolderOpen,
  ChevronRight,
  ChevronDown,
} from "lucide-vue-next";
import type { CodeFile } from "./types";

export interface FileTreeNode {
  name: string;
  path: string;
  type: "folder" | "file";
  children?: FileTreeNode[];
  fileObject?: CodeFile;
}

const props = defineProps<{
  node: FileTreeNode;
  expandedFolders: Set<string>;
  level: number;
}>();

const emit = defineEmits<{
  (e: "open-file", file: CodeFile): void;
  (e: "toggle-folder", path: string): void;
}>();

const isExpanded = computed(() =>
  props.node.type === "folder"
    ? props.expandedFolders.has(props.node.path)
    : false,
);

function getNodeIcon(node: FileTreeNode) {
  if (node.type === "folder") {
    return isExpanded.value ? FolderOpen : Folder;
  }
  // File icon logic
  if (node.fileObject?.language === "json") return FileJson;
  if (node.fileObject?.language === "javascript") return FileCode2;
  return FileIcon;
}
</script>

<template>
  <li :style="{ paddingLeft: `${level * 1.25}rem` }">
    <div
      @click="
        node.type === 'folder'
          ? emit('toggle-folder', node.path)
          : node.fileObject && emit('open-file', node.fileObject)
      "
      class="flex items-center gap-1.5 p-1 rounded-md cursor-pointer hover:bg-slate-700 select-none"
    >
      <template v-if="node.type === 'folder'">
        <ChevronDown v-if="isExpanded" :size="16" class="text-slate-400 shrink-0" />
        <ChevronRight v-else :size="16" class="text-slate-400 shrink-0" />
      </template>
      <template v-else>
        <span class="w-4 inline-block shrink-0"></span>
      </template>

      <component
        :is="getNodeIcon(node)"
        :size="16"
        class="shrink-0"
        :class="[
          node.type === 'folder' ? 'text-amber-400' : 'text-slate-400',
          node.fileObject?.isModified && node.type === 'file'
            ? '!text-amber-400'
            : '',
        ]"
      />
      <span
        class="truncate"
        :class="{
          'text-amber-400':
            node.fileObject?.isModified && node.type === 'file',
        }"
        :title="node.name"
      >
        {{ node.name }}
      </span>
    </div>
    <ul
      v-if="node.type === 'folder' && isExpanded && node.children?.length"
      class="text-sm"
    >
      <FileExplorerNode
        v-for="childNode in node.children"
        :key="childNode.path"
        :node="childNode"
        :expanded-folders="expandedFolders"
        :level="level + 1"
        @toggle-folder="(path) => emit('toggle-folder', path)"
        @open-file="(file) => emit('open-file', file)"
      />
    </ul>
  </li>
</template>
