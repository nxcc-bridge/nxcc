<script setup lang="ts">
import { computed, watch } from "vue";
import { X } from "lucide-vue-next";
import CodeMirrorEditor from "./CodeMirrorEditor.vue";
import type { CodeFile } from "./types";

const props = defineProps<{
  openFiles: CodeFile[];
  activeFileId: string | null;
}>();

const emit = defineEmits<{
  (e: "update:activeFileId", id: string | null): void;
  (e: "close-file", id: string): void;
  (
    e: "update:file-content",
    payload: { fileId: string; content: string },
  ): void;
  (e: "save-file", id: string): void;
}>();

const activeFile = computed(() => {
  return props.openFiles.find((f) => f.id === props.activeFileId) ?? null;
});
</script>

<template>
  <div class="flex flex-col h-full bg-slate-900">
    <div class="flex-shrink-0 bg-slate-800 border-b border-slate-700">
      <div v-if="openFiles.length > 0" class="flex items-center">
        <button
          v-for="file in openFiles"
          :key="file.id"
          @click="emit('update:activeFileId', file.id)"
          class="flex items-center gap-2 px-4 py-2 text-sm border-r border-slate-700"
          :class="
            activeFileId === file.id
              ? 'bg-slate-900 text-amber-400'
              : 'text-slate-300 hover:bg-slate-700'
          "
        >
          <span>{{ file.name.split("/").pop()
            }}{{ file.isModified ? "*" : "" }}</span
          >
          <X
            :size="16"
            @click.stop="emit('close-file', file.id)"
            class="rounded-sm hover:bg-slate-600"
          />
        </button>
      </div>
      <div v-else class="px-4 py-2 text-slate-500 text-sm">
        Select a file from the explorer to begin.
      </div>
    </div>
    <div class="flex-grow relative">
      <div v-if="activeFile" class="absolute inset-0">
        <CodeMirrorEditor
          :model-value="activeFile.content"
          :language="activeFile.language"
          @update:model-value="
            emit('update:file-content', {
              fileId: activeFile.id,
              content: $event,
            })
          "
          @save="emit('save-file', activeFile.id)"
          :key="activeFile.id"
        />
      </div>
      <div
        v-else
        class="flex items-center justify-center h-full text-slate-600"
      >
        <p>No file selected</p>
      </div>
    </div>
  </div>
</template>
