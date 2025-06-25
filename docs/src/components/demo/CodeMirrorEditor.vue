<script setup lang="ts">
import { ref, onMounted, watch, onUnmounted } from 'vue';
import { EditorView, keymap } from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { defaultKeymap, indentWithTab } from '@codemirror/commands';
import { javascript } from '@codemirror/lang-javascript';
import { json } from '@codemirror/lang-json';
import { basicSetup } from 'codemirror';

const props = defineProps<{
  modelValue: string;
  language: 'javascript' | 'json';
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void;
  (e: 'save'): void;
}>();

const editorEl = ref<HTMLDivElement>();
let view: EditorView;

const languageConf = {
  javascript: () => javascript(),
  json: () => json(),
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
          key: 'Mod-s',
          preventDefault: true,
          run: () => {
            emit('save');
            return true;
          },
        },
      ]),
      languageConf[props.language](),
      EditorView.theme({
        '&': {
          color: '#cbd5e1' /* slate-300 */,
          backgroundColor: '#0f172a' /* slate-900 */,
          height: '100%',
        },
        '.cm-content': {
          caretColor: '#fbbf24' /* amber-400 */,
        },
        '&.cm-focused .cm-cursor': {
          borderLeftColor: '#fbbf24' /* amber-400 */,
        },
        '&.cm-focused .cm-selectionBackground, ::selection': {
          backgroundColor: '#475569' /* slate-600 */,
        },
        '.cm-gutters': {
          backgroundColor: '#0f172a' /* slate-900 */,
          color: '#64748b' /* slate-500 */,
          border: 'none',
        },
      }),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          emit('update:modelValue', update.state.doc.toString());
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
  }
);

onUnmounted(() => {
  view?.destroy();
});
</script>

<template>
  <div ref="editorEl" class="h-full w-full overflow-y-auto"></div>
</template>
