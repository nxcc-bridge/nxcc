<script setup lang="ts">
import { ref, onMounted, watch, onUnmounted } from "vue";
import { EditorView, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { basicSetup } from "codemirror";
import { oneDark } from "@codemirror/theme-one-dark";

const props = defineProps<{
  modelValue: string;
  language: "javascript" | "json" | "solidity";
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "save"): void;
}>();

const editorEl = ref<HTMLDivElement>();
let view: EditorView;

const languageConf = {
  javascript: () => javascript(),
  json: () => json(),
  solidity: () => javascript(), // Use JS highlighting for solidity
};

onMounted(() => {
  const state = EditorState.create({
    doc: props.modelValue,
    extensions: [
      basicSetup,
      keymap.of([
        ...defaultKeymap,
        indentWithTab,
        {
          key: "Mod-s",
          preventDefault: true,
          run: () => {
            emit("save");
            return true;
          },
        },
      ]),
      languageConf[props.language](),
      oneDark,
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          emit("update:modelValue", update.state.doc.toString());
        }
      }),
    ],
  });

  view = new EditorView({
    state,
    parent: editorEl.value,
  });
});

watch(
  () => props.modelValue,
  (newValue) => {
    if (view && newValue !== view.state.doc.toString()) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: newValue },
      });
    }
  },
);

onUnmounted(() => {
  view?.destroy();
});
</script>

<template>
  <div ref="editorEl" class="h-full w-full overflow-y-auto"></div>
</template>
