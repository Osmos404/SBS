import { invoke } from "@tauri-apps/api/core";

export const SYSTEM_PROMPT =
  "Ты быстрый помощник преподавателя математики и программирования. " +
  "Преподаватель ведёт онлайн-урок, ему нужна короткая подсказка прямо сейчас. " +
  "Отвечай по-русски, максимально кратко: сначала суть в 1–2 фразах, затем решение по шагам " +
  "или короткий пример кода. Не пиши длинных вступлений. Если не уверен в ответе, так и скажи.";

export function askModel(model: string, question: string): Promise<string> {
  return invoke<string>("ask_ollama", {
    model,
    system: SYSTEM_PROMPT,
    question,
  });
}
