mod ollama;
#[cfg(windows)]
mod vcursor_win;

use serde::{Deserialize, Serialize};

const OLLAMA_URL: &str = "http://127.0.0.1:11434/api/chat";

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

/// Отправляет вопрос в локальную модель Ollama и возвращает текст ответа.
#[tauri::command]
async fn ask_ollama(model: String, system: String, question: String) -> Result<String, String> {
    let body = ChatRequest {
        model: &model,
        messages: vec![
            ChatMessage { role: "system".into(), content: system },
            ChatMessage { role: "user".into(), content: question },
        ],
        stream: false,
    };

    let resp = reqwest::Client::new()
        .post(OLLAMA_URL)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Не удалось подключиться к Ollama ({e}). Запущена ли она?"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama вернула ошибку {status}: {text}"));
    }

    let parsed: ChatResponse = resp
        .json()
        .await
        .map_err(|e| format!("Непонятный ответ Ollama: {e}"))?;
    Ok(parsed.message.content)
}

/// Список моделей, установленных в Ollama.
#[tauri::command]
async fn list_models() -> Result<Vec<String>, String> {
    ollama::list_models().await
}

/// Загрузка модели в Ollama (по названию).
#[tauri::command]
async fn pull_model(name: String) -> Result<(), String> {
    ollama::pull_model(&name).await
}

/// Включает/выключает режим «два курсора»: системная мышь замораживается,
/// а движения и клики уходят в интерфейс (событие `vcursor`).
#[tauri::command]
fn set_virtual_cursor(window: tauri::WebviewWindow, active: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let own = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        vcursor_win::set_active(active, own);
    }
    #[cfg(not(windows))]
    {
        let _ = (window, active);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
    }

    builder
        .setup(|app| {
            #[cfg(windows)]
            {
                use tauri::Emitter;
                let handle = app.handle().clone();
                vcursor_win::init(Box::new(move |ev| {
                    let _ = handle.emit("vcursor", ev);
                }));
            }
            #[cfg(not(windows))]
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![ask_ollama, list_models, pull_model, set_virtual_cursor])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
