//! Credential for the Zotero full-text search embedding backend.
//!
//! The zfulltext vector index (`~/.local/share/zotero-cli/vectors.sqlite`) is
//! dimension-locked to whichever embedding backend first built it — switching
//! backends means dropping and re-embedding the whole index, so unlike the
//! translation model settings this panel does not offer a backend picker. It
//! only manages the key for Gemini, the backend packages/zotero-cli defaults
//! to (`ZSEARCH_EMBEDDING_BACKEND` unset).
//!
//! The key lives in the same macOS Keychain service as the translation model
//! keys, under its own account, and is injected into the zfulltext subprocess
//! env at run time. The CLI's own `.env` lookup remains the fallback when no
//! key is stored here.

const KEYCHAIN_SERVICE: &str = "com.bibliosmith.launcher.models";
const ACCOUNT: &str = "embedding/gemini";
const KEY_ENV: &str = "GEMINI_API_KEY";

fn keychain_read() -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, ACCOUNT)
        .ok()?
        .get_password()
        .ok()
}

fn keychain_write(secret: &str) -> Result<(), String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, ACCOUNT)
        .and_then(|entry| entry.set_password(secret))
        .map_err(|err| format!("Keychain write failed: {err}"))
}

fn keychain_delete() -> Result<(), String> {
    match keyring::Entry::new(KEYCHAIN_SERVICE, ACCOUNT).map(|entry| entry.delete_credential()) {
        Ok(Ok(())) => Ok(()),
        // Deleting an absent entry is success from the caller's point of view.
        Ok(Err(keyring::Error::NoEntry)) => Ok(()),
        Ok(Err(err)) => Err(format!("Keychain delete failed: {err}")),
        Err(err) => Err(format!("Keychain delete failed: {err}")),
    }
}

/// The `(key_env, secret)` pair to inject into the zfulltext subprocess, or
/// None when no key is stored — the CLI then falls back to its own `.env`
/// lookup.
pub fn resolve_credential_env() -> Option<(String, String)> {
    keychain_read().map(|secret| (KEY_ENV.to_string(), secret))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStatus {
    pub backend: String,
    pub configured: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConnectionResult {
    pub ok: bool,
    pub message: String,
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn get_embedding_status() -> EmbeddingStatus {
    EmbeddingStatus {
        backend: "gemini".into(),
        configured: keychain_read().is_some(),
    }
}

#[tauri::command]
pub fn save_embedding_credential(api_key: String) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty.".into());
    }
    keychain_write(trimmed)
}

#[tauri::command]
pub fn delete_embedding_credential() -> Result<(), String> {
    keychain_delete()
}

/// Send the smallest possible request to confirm the key works. `api_key`
/// lets the UI test a key the user just typed but has not saved yet; when
/// absent the stored key is used.
#[tauri::command]
pub fn test_embedding_connection(
    api_key: Option<String>,
) -> Result<EmbeddingConnectionResult, String> {
    let key = match api_key {
        Some(key) if !key.trim().is_empty() => key.trim().to_string(),
        _ => keychain_read().ok_or_else(|| "No API key stored for this slot.".to_string())?,
    };
    Ok(match probe_gemini_embedding(&key) {
        Ok(()) => EmbeddingConnectionResult {
            ok: true,
            message: "Gemini embedding 连接正常".into(),
        },
        Err(message) => EmbeddingConnectionResult { ok: false, message },
    })
}

fn probe_gemini_embedding(key: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;

    // Mirrors packages/zotero-cli/src/zotero_cli/embed.py's GeminiEmbedder,
    // which always calls batchEmbedContents (never the singular embedContent
    // endpoint), so a passing test here matches what indexing will actually do.
    let response = client
        .post("https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:batchEmbedContents")
        .header("x-goog-api-key", key)
        .json(&serde_json::json!({
            "requests": [{
                "model": "models/gemini-embedding-001",
                "content": {"parts": [{"text": "ping"}]},
            }],
        }))
        .send()
        .map_err(|err| format!("请求失败：{err}"))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().unwrap_or_default();
    let hint = if status.as_u16() == 401 || status.as_u16() == 403 {
        "（密钥或权限问题）"
    } else if status.as_u16() == 404 {
        "（模型名或地址可能不对）"
    } else {
        ""
    };
    Err(format!(
        "HTTP {}{hint}：{}",
        status.as_u16(),
        body.chars().take(200).collect::<String>()
    ))
}
