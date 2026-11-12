//! Работа с локальной Ollama: список установленных моделей и загрузка новой.

use serde::{Deserialize, Serialize};

const BASE: &str = "http://127.0.0.1:11434";

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

#[derive(Serialize)]
struct PullRequest<'a> {
    model: &'a str,
    stream: bool,
}

fn connect_err(e: reqwest::Error) -> String {
    format!("Не удалось подключиться к Ollama ({e}). Запущена ли она?")
}

/// Названия установленных моделей.
pub async fn list_models() -> Result<Vec<String>, String> {
    let resp = reqwest::Client::new()
        .get(format!("{BASE}/api/tags"))
        .send()
        .await
        .map_err(connect_err)?;
    if !resp.status().is_success() {
        return Err(format!("Ollama вернула ошибку {}", resp.status()));
    }
    let parsed: TagsResponse = resp
        .json()
        .await
        .map_err(|e| format!("Непонятный ответ Ollama: {e}"))?;
    let mut names: Vec<String> = parsed.models.into_iter().map(|m| m.name).collect();
    names.sort();
    Ok(names)
}

/// Скачивает модель (имя из каталога Ollama или `hf.co/пользователь/репозиторий`).
/// Возвращает управление, когда загрузка завершена.
pub async fn pull_model(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Укажите название модели".into());
    }
    let resp = reqwest::Client::new()
        .post(format!("{BASE}/api/pull"))
        .json(&PullRequest { model: name, stream: false })
        .send()
        .await
        .map_err(connect_err)?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Не удалось загрузить «{name}»: {status} {text}"));
    }
    Ok(())
}
