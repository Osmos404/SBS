<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import "highlight.js/styles/github-dark.css";
import { askModel } from "./lib/ai";
import { renderMarkdown } from "./lib/markdown";
import { DEMO_ANSWER, DEMO_QUESTION } from "./lib/demo";

const appWindow = getCurrentWindow();

const HOTKEY_TOGGLE = "CommandOrControl+Alt+H";
const HOTKEY_CLICKTHROUGH = "CommandOrControl+Alt+C";
const HOTKEY_FOCUS = "CommandOrControl+Alt+Space";

const opacity = ref(Number(localStorage.getItem("sbs.opacity") ?? 0.88));
const model = ref(localStorage.getItem("sbs.model") ?? "qwen2.5:7b");
const draft = ref("");
const heard = ref("");
const answer = ref("");
const error = ref("");
const loading = ref(false);
const clickThrough = ref(false);
const input = ref<HTMLTextAreaElement | null>(null);

const answerHtml = computed(() => (answer.value ? renderMarkdown(answer.value) : ""));

watch(opacity, (v) => localStorage.setItem("sbs.opacity", String(v)));
watch(model, (v) => localStorage.setItem("sbs.model", v));

function reset(question: string) {
  heard.value = question;
  answer.value = "";
  error.value = "";
  loading.value = true;
}

async function ask() {
  const q = draft.value.trim();
  if (!q || loading.value) return;
  draft.value = "";
  reset(q);
  try {
    answer.value = await askModel(model.value, q);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

// Тест: имитируем, что приложение услышало вопрос студента и подготовило ответ.
function runDemo() {
  if (loading.value) return;
  reset(DEMO_QUESTION);
  setTimeout(() => {
    answer.value = DEMO_ANSWER;
    loading.value = false;
  }, 900);
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    ask();
  }
}

async function onAnswerClick(e: MouseEvent) {
  const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button.copy");
  if (!btn) return;
  const code = btn.closest(".code")?.querySelector("code")?.textContent ?? "";
  await navigator.clipboard.writeText(code);
  btn.textContent = "Скопировано";
  setTimeout(() => (btn.textContent = "Копировать"), 1200);
}

async function toggleVisible() {
  if (await appWindow.isVisible()) await appWindow.hide();
  else await appWindow.show();
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
      <div data-tauri-drag-region class="brand">
        <span class="dot" />
        <span data-tauri-drag-region class="title">SBS</span>
        <span v-if="clickThrough" class="badge">сквозной режим</span>
      </div>
      <label class="slider" title="Прозрачность окна">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
          <path d="M12 3.5c3.6 4.2 6 7.3 6 10.5a6 6 0 0 1-12 0c0-3.2 2.4-6.3 6-10.5Z" />
        </svg>
        <input v-model.number="opacity" type="range" min="0.25" max="1" step="0.05" />
      </label>
    </header>

    <main class="body" @click="onAnswerClick">
      <template v-if="heard">
        <div class="heard">
          <div class="label">Студент спрашивает</div>
          <div class="heard-text">{{ heard }}</div>
        </div>

        <div v-if="loading" class="thinking"><i /><i /><i /></div>
        <div v-else-if="error" class="error">{{ error }}</div>
        <div v-else class="answer md" v-html="answerHtml" />
      </template>

      <div v-else class="empty">
        <div class="empty-title">Готов подсказать</div>
        <div class="empty-sub">Введите вопрос студента или нажмите «Тест», чтобы посмотреть, как это выглядит.</div>
        <div class="keys">
          <span><kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>Space</kbd> ввод</span>
          <span><kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>H</kbd> скрыть</span>
          <span><kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>C</kbd> сквозной режим</span>
        </div>
      </div>
    </main>

    <footer class="foot">
      <textarea
        ref="input"
        v-model="draft"
        rows="2"
        placeholder="Вопрос студента…  Enter: отправить, Shift+Enter: новая строка"
        @keydown="onKey"
      />
      <div class="row">
        <input v-model="model" class="model" spellcheck="false" title="Модель Ollama" />
        <button class="ghost" :disabled="loading" @click="runDemo">▶ Тест</button>
        <button class="primary" :disabled="loading || !draft.trim()" @click="ask">Спросить</button>
      </div>
    </footer>
  </div>
</template>

<style>
:root {
  --bg: 14, 16, 24;
  --accent: #7c83ff;
  --accent-2: #a78bfa;
  --text: #e6e8f0;
  --muted: #8b90a3;
  --line: rgba(255, 255, 255, 0.08);
}
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
  font-family: "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
  color: var(--text);
  font-size: 14px;
}
::-webkit-scrollbar {
  width: 8px;
}
::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.14);
  border-radius: 8px;
}
::-webkit-scrollbar-track {
  background: transparent;
}

.panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  border-radius: 14px;
  background: rgba(var(--bg), var(--op));
  border: 1px solid var(--line);
  overflow: hidden;
}

/* шапка */
.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--line);
  cursor: move;
  user-select: none;
}
.brand {
  display: flex;
  align-items: center;
  gap: 8px;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--accent), var(--accent-2));
  box-shadow: 0 0 10px var(--accent);
}
.title {
  font-weight: 650;
  letter-spacing: 0.12em;
  font-size: 12px;
}
.badge {
  font-size: 10.5px;
  padding: 2px 8px;
  border-radius: 99px;
  background: rgba(245, 158, 11, 0.18);
  color: #fbbf24;
}
.slider {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--muted);
  cursor: default;
}
.slider input {
  width: 84px;
  accent-color: var(--accent);
}

/* основная область */
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
}
.heard {
  padding: 9px 12px;
  margin-bottom: 12px;
  border-radius: 10px;
  background: rgba(124, 131, 255, 0.1);
  border-left: 3px solid var(--accent);
}
.label {
  font-size: 10.5px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--accent);
  margin-bottom: 3px;
}
.heard-text {
  line-height: 1.4;
}
.error {
  color: #fca5a5;
  background: rgba(239, 68, 68, 0.1);
  padding: 9px 12px;
  border-radius: 10px;
}

.thinking {
  display: flex;
  gap: 5px;
  padding: 6px 2px;
}
.thinking i {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--accent);
  animation: pulse 1s infinite ease-in-out;
}
.thinking i:nth-child(2) {
  animation-delay: 0.15s;
}
.thinking i:nth-child(3) {
  animation-delay: 0.3s;
}
@keyframes pulse {
  0%,
  80%,
  100% {
    opacity: 0.25;
    transform: scale(0.8);
  }
  40% {
    opacity: 1;
    transform: scale(1);
  }
}

/* пустое состояние */
.empty {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  gap: 8px;
  color: var(--muted);
}
.empty-title {
  font-size: 17px;
  font-weight: 600;
  color: var(--text);
}
.empty-sub {
  max-width: 340px;
  line-height: 1.45;
}
.keys {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12px;
}
kbd {
  font: inherit;
  font-size: 11px;
  padding: 1px 6px;
  margin-right: 3px;
  border-radius: 5px;
  background: rgba(255, 255, 255, 0.07);
  border: 1px solid var(--line);
  color: var(--text);
}
.keys span {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
}

/* ответ (markdown) */
.md {
  line-height: 1.55;
  word-break: break-word;
}
.md p {
  margin: 0 0 10px;
}
.md ol,
.md ul {
  margin: 0 0 10px;
  padding-left: 22px;
}
.md li {
  margin-bottom: 3px;
}
.md strong {
  color: #fff;
}
.md :not(pre) > code {
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 12.5px;
  padding: 1px 6px;
  border-radius: 5px;
  background: rgba(255, 255, 255, 0.08);
  color: #c4b5fd;
}
.md blockquote {
  margin: 0 0 10px;
  padding: 6px 12px;
  border-left: 3px solid var(--accent-2);
  background: rgba(167, 139, 250, 0.08);
  border-radius: 0 8px 8px 0;
  color: #cfd2e0;
}
.md blockquote p {
  margin: 0;
}
.code {
  margin: 0 0 12px;
  border-radius: 10px;
  overflow: hidden;
  border: 1px solid var(--line);
  background: rgba(0, 0, 0, 0.35);
}
.code-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 4px 10px;
  background: rgba(255, 255, 255, 0.05);
  font-size: 11px;
  color: var(--muted);
  text-transform: lowercase;
}
.copy {
  all: unset;
  cursor: pointer;
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 6px;
  color: var(--text);
  background: rgba(255, 255, 255, 0.08);
}
.copy:hover {
  background: rgba(255, 255, 255, 0.16);
}
.code pre {
  margin: 0;
  padding: 10px 12px;
  overflow-x: auto;
}
.code code.hljs {
  padding: 0;
  background: transparent;
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 12.5px;
  line-height: 1.5;
}

/* низ */
.foot {
  padding: 10px 12px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border-top: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.02);
}
textarea,
.model {
  width: 100%;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: rgba(0, 0, 0, 0.3);
  color: inherit;
  font: inherit;
  padding: 7px 10px;
  resize: none;
  outline: none;
  transition: border-color 0.15s;
}
textarea:focus,
.model:focus {
  border-color: var(--accent);
}
textarea::placeholder {
  color: #5d6277;
}
.row {
  display: flex;
  gap: 8px;
}
.model {
  flex: 1;
  font-size: 12.5px;
  color: var(--muted);
}
button.primary,
button.ghost {
  border: none;
  border-radius: 9px;
  padding: 7px 14px;
  font: inherit;
  cursor: pointer;
  white-space: nowrap;
  transition: filter 0.15s, background 0.15s;
}
button.primary {
  color: #fff;
  background: linear-gradient(135deg, var(--accent), var(--accent-2));
}
button.ghost {
  color: var(--text);
  background: rgba(255, 255, 255, 0.08);
}
button.ghost:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.14);
}
button.primary:hover:not(:disabled) {
  filter: brightness(1.1);
}
button:disabled {
  opacity: 0.45;
  cursor: default;
}
</style>
