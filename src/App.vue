<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { askModel } from "./lib/ai";

const appWindow = getCurrentWindow();

const HOTKEY_TOGGLE = "CommandOrControl+Alt+H";
const HOTKEY_CLICKTHROUGH = "CommandOrControl+Alt+C";
const HOTKEY_FOCUS = "CommandOrControl+Alt+Space";

const opacity = ref(Number(localStorage.getItem("sbs.opacity") ?? 0.85));
const model = ref(localStorage.getItem("sbs.model") ?? "qwen2.5:7b");
const question = ref("");
const answer = ref("");
const error = ref("");
const loading = ref(false);
const clickThrough = ref(false);
const input = ref<HTMLTextAreaElement | null>(null);

watch(opacity, (v) => localStorage.setItem("sbs.opacity", String(v)));
watch(model, (v) => localStorage.setItem("sbs.model", v));

async function ask() {
  const q = question.value.trim();
  if (!q || loading.value) return;
  loading.value = true;
  error.value = "";
  answer.value = "";
  try {
    answer.value = await askModel(model.value, q);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    ask();
  }
}

async function toggleVisible() {
  if (await appWindow.isVisible()) {
    await appWindow.hide();
  } else {
    await appWindow.show();
  }
}

async function setClickThrough(value: boolean) {
  clickThrough.value = value;
  await appWindow.setIgnoreCursorEvents(value);
}

async function focusInput() {
  await appWindow.show();
  await setClickThrough(false);
  await appWindow.setFocus();
  input.value?.focus();
}

onMounted(async () => {
  await register(HOTKEY_TOGGLE, (e) => e.state === "Pressed" && toggleVisible());
  await register(
    HOTKEY_CLICKTHROUGH,
    (e) => e.state === "Pressed" && setClickThrough(!clickThrough.value),
  );
  await register(HOTKEY_FOCUS, (e) => e.state === "Pressed" && focusInput());
});

onBeforeUnmount(() => {
  unregister(HOTKEY_TOGGLE);
  unregister(HOTKEY_CLICKTHROUGH);
  unregister(HOTKEY_FOCUS);
});
</script>

<template>
  <div class="panel" :style="{ '--op': opacity }">
    <header data-tauri-drag-region class="bar">
      <span data-tauri-drag-region class="title">SBS</span>
      <span v-if="clickThrough" class="badge">сквозной режим</span>
      <label class="slider" title="Прозрачность">
        <input v-model.number="opacity" type="range" min="0.2" max="1" step="0.05" />
      </label>
    </header>

    <main class="body">
      <div v-if="loading" class="hint">Думаю…</div>
      <div v-else-if="error" class="error">{{ error }}</div>
      <pre v-else-if="answer" class="answer">{{ answer }}</pre>
      <div v-else class="hint">
        Ctrl+Alt+Пробел: фокус на ввод · Ctrl+Alt+H: скрыть/показать · Ctrl+Alt+C: сквозные клики
      </div>
    </main>

    <footer class="foot">
      <textarea
        ref="input"
        v-model="question"
        rows="2"
        placeholder="Вопрос студента… (Enter — отправить)"
        @keydown="onKey"
      />
      <div class="row">
        <input v-model="model" class="model" spellcheck="false" title="Модель Ollama" />
        <button :disabled="loading" @click="ask">Спросить</button>
      </div>
    </footer>
  </div>
</template>

<style>
html,
body,
#app {
  margin: 0;
  height: 100%;
  background: transparent;
  overflow: hidden;
}
* {
  box-sizing: border-box;
}
body {
  font-family: "Segoe UI", system-ui, sans-serif;
  color: #e8eaf0;
  font-size: 14px;
}
.panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  border-radius: 10px;
  background: rgba(16, 18, 26, var(--op));
  border: 1px solid rgba(255, 255, 255, 0.12);
  overflow: hidden;
}
.bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px;
  background: rgba(255, 255, 255, 0.06);
  cursor: move;
  user-select: none;
}
.title {
  font-weight: 600;
  letter-spacing: 0.08em;
}
.badge {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 8px;
  background: #b45309;
}
.slider {
  margin-left: auto;
  display: flex;
}
.slider input {
  width: 90px;
}
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 10px 12px;
}
.answer {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font: inherit;
  line-height: 1.45;
}
.hint {
  opacity: 0.55;
  line-height: 1.5;
}
.error {
  color: #fca5a5;
}
.foot {
  padding: 8px 10px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  background: rgba(255, 255, 255, 0.04);
}
textarea,
.model {
  width: 100%;
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.35);
  color: inherit;
  font: inherit;
  padding: 6px 8px;
  resize: none;
  outline: none;
}
textarea:focus,
.model:focus {
  border-color: #6366f1;
}
.row {
  display: flex;
  gap: 8px;
}
.model {
  flex: 1;
}
button {
  border: none;
  border-radius: 6px;
  padding: 6px 14px;
  background: #6366f1;
  color: white;
  font: inherit;
  cursor: pointer;
}
button:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
