<script setup lang="ts">
import { Files, Rocket, Wand2 } from "lucide-vue-next";

defineProps<{
  activeView: "explorer" | "deploy";
}>();

const emit = defineEmits<{
  (e: "update:activeView", view: "explorer" | "deploy"): void;
  (e: "start-tour"): void;
}>();
</script>

<template>
  <div
    class="flex flex-col items-center gap-y-4 bg-slate-900 p-2 border-r border-slate-700"
  >
    <button
      @click="emit('update:activeView', 'explorer')"
      class="p-2 rounded-md transition-colors"
      :class="
        activeView === 'explorer'
          ? 'text-amber-400 bg-slate-700'
          : 'text-slate-400 hover:bg-slate-800'
      "
      aria-label="File Explorer"
    >
      <Files :size="24" />
    </button>
    <button
      @click="emit('update:activeView', 'deploy')"
      class="p-2 rounded-md transition-colors"
      :class="
        activeView === 'deploy'
          ? 'text-amber-400 bg-slate-700'
          : 'text-slate-400 hover:bg-slate-800'
      "
      aria-label="Run and Deploy"
    >
      <Rocket :size="24" />
    </button>
    <div class="mt-auto">
      <button
        @click="emit('start-tour')"
        class="p-2 rounded-md transition-colors text-slate-400 hover:bg-slate-800 hover:text-amber-400"
        aria-label="Start Guided Tour"
      >
        <Wand2 :size="24" />
      </button>
    </div>
  </div>
</template>
