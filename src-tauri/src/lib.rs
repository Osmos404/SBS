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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
    }

    builder
        .invoke_handler(tauri::generate_handler![ask_ollama])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
