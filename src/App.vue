<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import "highlight.js/styles/github-dark.css";
import { askModel } from "./lib/ai";
import { renderMarkdown } from "./lib/markdown";
import { DEMO_ANSWER, DEMO_QUESTION } from "./lib/demo";

const appWindow = getCurrentWindow();

const HOTKEY_TOGGLE = "CommandOrControl+Alt+H";
const HOTKEY_CLICKTHROUGH = "CommandOrControl+Alt+C";
const HOTKEY_FOCUS = "CommandOrControl+Alt+Space";
const HOTKEY_QUIT = "CommandOrControl+Alt+Q";
const HOTKEY_DUAL = "CommandOrControl+Alt+X";

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

// Свой красный курсор: системный курсор над окном скрыт (его видно в демонстрации),
// а наш рисуется внутри окна, которое в демонстрацию не попадает.
const ptr = ref({ x: -100, y: -100, show: false });

function onPointerMove(e: PointerEvent) {
  if (dual.value) return;
  ptr.value = { x: e.clientX, y: e.clientY, show: true };
  moveDrag(e);
}

function onPointerLeave() {
  if (!dual.value) ptr.value.show = false;
}

// ---------- Режим «два курсора» ----------
// Системный курсор остаётся на месте (его видят зрители), а мышь управляет
// красным курсором, который живёт только внутри окна-подсказки.
const dual = ref(false);
const SENSITIVITY = 1.3;
let unlistenV: UnlistenFn | null = null;
let vDownTarget: HTMLElement | null = null;
let vSlider: HTMLInputElement | null = null;

interface VEvent {
  kind: "move" | "down" | "up" | "wheel";
  dx: number;
  dy: number;
  button: number;
  delta: number;
}

function elementAtPointer(): HTMLElement | null {
  return document.elementFromPoint(ptr.value.x, ptr.value.y) as HTMLElement | null;
}

function setSliderFromPointer(el: HTMLInputElement) {
  const r = el.getBoundingClientRect();
  const min = Number(el.min || 0);
  const max = Number(el.max || 100);
  const step = Number(el.step || 1);
  const t = Math.min(1, Math.max(0, (ptr.value.x - r.left) / r.width));
  const raw = min + t * (max - min);
  el.value = String(Math.round(raw / step) * step);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

function onVirtualEvent(p: VEvent) {
  if (!dual.value) return;
  if (p.kind === "move") {
    ptr.value = {
      x: Math.min(window.innerWidth - 1, Math.max(0, ptr.value.x + p.dx * SENSITIVITY)),
      y: Math.min(window.innerHeight - 1, Math.max(0, ptr.value.y + p.dy * SENSITIVITY)),
      show: true,
    };
    if (vSlider) setSliderFromPointer(vSlider);
  } else if (p.kind === "down" && p.button === 0) {
    const el = elementAtPointer();
    vDownTarget = el;
    if (el instanceof HTMLInputElement && el.type === "range") {
      vSlider = el;
      setSliderFromPointer(el);
    } else if (el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && el.type === "text")) {
      el.focus();
    }
  } else if (p.kind === "up" && p.button === 0) {
    vSlider = null;
    const el = elementAtPointer();
    const btn = el?.closest("button");
    // «клик» засчитываем, только если отпустили над той же кнопкой, где нажали
    if (btn && vDownTarget?.closest("button") === btn) btn.click();
    vDownTarget = null;
  } else if (p.kind === "wheel") {
    const body = document.querySelector<HTMLElement>(".body");
    if (body) body.scrollTop -= (p.delta / 120) * 60;
  }
}

async function setDual(on: boolean) {
  if (on === dual.value) return;
  if (on) {
    // сначала включаем режим (Rust запоминает окно, из которого пришли), потом забираем фокус
    await invoke("set_virtual_cursor", { active: true });
    dual.value = true;
    await appWindow.show();
    await setClickThrough(false);
    await appWindow.setFocus();
    ptr.value = { x: window.innerWidth / 2, y: window.innerHeight / 2, show: true };
  } else {
    dual.value = false;
    vSlider = null;
    vDownTarget = null;
    ptr.value.show = false;
    await invoke("set_virtual_cursor", { active: false });
  }
}

// Перемещение окна вручную (а не штатным перетаскиванием Windows),
// чтобы системный курсор оставался скрытым и в демонстрации его не было.
let drag: { sx: number; sy: number; wx: number; wy: number } | null = null;
let raf = 0;

async function startDrag(e: PointerEvent) {
  if ((e.target as HTMLElement).closest("input,label,button")) return;
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  const [scale, pos] = await Promise.all([appWindow.scaleFactor(), appWindow.outerPosition()]);
  drag = { sx: e.screenX, sy: e.screenY, wx: pos.x / scale, wy: pos.y / scale };
}

function moveDrag(e: PointerEvent) {
  if (!drag || raf) return;
  const d = drag;
  const x = d.wx + e.screenX - d.sx;
  const y = d.wy + e.screenY - d.sy;
  raf = requestAnimationFrame(() => {
    raf = 0;
    appWindow.setPosition(new LogicalPosition(x, y));
  });
}

function endDrag(e: PointerEvent) {
  drag = null;
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
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

function quit() {
  return appWindow.close();
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
  await register(HOTKEY_QUIT, (e) => e.state === "Pressed" && quit());
  await register(HOTKEY_DUAL, (e) => e.state === "Pressed" && setDual(!dual.value));
  unlistenV = await listen<VEvent>("vcursor", (e) => onVirtualEvent(e.payload));
});

onBeforeUnmount(() => {
  unregister(HOTKEY_TOGGLE);
  unregister(HOTKEY_CLICKTHROUGH);
  unregister(HOTKEY_FOCUS);
  unregister(HOTKEY_QUIT);
  unregister(HOTKEY_DUAL);
  unlistenV?.();
  if (dual.value) invoke("set_virtual_cursor", { active: false });
});
</script>

<template>
  <div
    class="panel"
    :class="{ dual }"
    :style="{ '--op': opacity }"
    @pointermove="onPointerMove"
    @pointerleave="onPointerLeave"
  >
    <header class="bar" @pointerdown="startDrag" @pointerup="endDrag" @pointercancel="endDrag">
      <div class="brand">
        <span class="grip" title="Потяните, чтобы переместить окно">
          <svg viewBox="0 0 24 24" width="16" height="16">
            <path d="M4 2v18l4.5-4.5L12 22l3-1.5-3.5-6.5H18z" fill="#ef4444" stroke="#fff" stroke-width="1.2" stroke-linejoin="round" />
          </svg>
        </span>
        <span class="title">SBS</span>
        <span v-if="clickThrough" class="badge">сквозной режим</span>
        <span v-if="dual" class="badge red">красный курсор</span>
      </div>
      <button
        class="dualbtn"
        :class="{ on: dual }"
        title="Два курсора: мышь управляет красным курсором в окне (Ctrl+Alt+X)"
        @click="setDual(!dual)"
      >
        <svg viewBox="0 0 24 24" width="13" height="13">
          <path d="M4 2v18l4.5-4.5L12 22l3-1.5-3.5-6.5H18z" fill="currentColor" />
        </svg>
        2 курсора
      </button>
      <label class="slider" title="Прозрачность окна">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
          <path d="M12 3.5c3.6 4.2 6 7.3 6 10.5a6 6 0 0 1-12 0c0-3.2 2.4-6.3 6-10.5Z" />
        </svg>
        <input v-model.number="opacity" type="range" min="0.25" max="1" step="0.05" />
      </label>
      <button class="close" title="Закрыть (Ctrl+Alt+Q)" @click="quit">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
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
          <span><kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>X</kbd> два курсора</span>
          <span><kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>Q</kbd> закрыть</span>
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

    <svg
      v-show="ptr.show"
      class="ptr"
      viewBox="0 0 24 24"
      width="20"
      height="20"
      :style="{ transform: `translate(${ptr.x - 4}px, ${ptr.y - 2}px)` }"
    >
      <path d="M4 2v18l4.5-4.5L12 22l3-1.5-3.5-6.5H18z" fill="#ef4444" stroke="#fff" stroke-width="1.2" stroke-linejoin="round" />
    </svg>
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
  user-select: none;
  touch-action: none;
}
.grip {
  display: grid;
  place-items: center;
  width: 24px;
  height: 24px;
  border-radius: 7px;
  background: rgba(239, 68, 68, 0.12);
}
.grip:hover {
  background: rgba(239, 68, 68, 0.25);
}
/* системный курсор скрыт, вместо него рисуется красный (см. .ptr) */
.panel,
.panel * {
  cursor: none !important;
}
.ptr {
  position: fixed;
  left: 0;
  top: 0;
  pointer-events: none;
  z-index: 9999;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.5));
}
.brand {
  flex: 1;
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
.dualbtn {
  all: unset;
  margin-left: auto;
  margin-right: 10px;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  padding: 3px 9px;
  border-radius: 99px;
  color: #fca5a5;
  background: rgba(239, 68, 68, 0.12);
  transition: background 0.15s;
}
.dualbtn:hover {
  background: rgba(239, 68, 68, 0.25);
}
.dualbtn.on {
  color: #fff;
  background: #ef4444;
}
.badge.red {
  background: rgba(239, 68, 68, 0.2);
  color: #fca5a5;
}
.panel.dual {
  border-color: rgba(239, 68, 68, 0.6);
}
.slider {
  margin-right: 10px;
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

.close {
  all: unset;
  cursor: pointer;
  display: grid;
  place-items: center;
  width: 26px;
  height: 26px;
  border-radius: 7px;
  color: var(--muted);
  transition: background 0.15s, color 0.15s;
}
.close:hover {
  background: #ef4444;
  color: #fff;
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
