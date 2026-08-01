//! Credentials for the remote OCR services the conversion pipeline can use.
//!
//! PaddleOCR (Baidu AI Studio) and MinerU both extract text from scanned or
//! low-text PDFs; a book with a proper text layer needs neither. Keys live in
//! the macOS Keychain, one account per service, and are injected into the
//! conversion worker's subprocess env at run time. The worker's own repo-root
//! `.env` lookup stays as the fallback — it only adopts `.env` values for
//! variables not already set, so a Keychain key always wins.

use serde::Serialize;

use crate::book_pipeline::translation_engine_repo_root;

const KEYCHAIN_SERVICE: &str = "com.bibliosmith.launcher.models";

const PADDLE_ACCOUNT: &str = "ocr/paddleocr";
const MINERU_ACCOUNT: &str = "ocr/mineru";

/// The exact variables packages/ocr/scripts/zotero_llm_worker.py reads.
const PADDLE_KEY_ENV: &str = "BAIDU_PADDLEOCR_TOKEN";
const MINERU_KEY_ENV: &str = "MINERU_API_TOKEN";
/// The worker accepts either MinerU spelling; both count as configured.
const MINERU_KEY_ENV_ALT: &str = "MINERU_TOKEN";

fn account_for(service: &str) -> Result<&'static str, String> {
    match service {
        "paddleocr" => Ok(PADDLE_ACCOUNT),
        "mineru" => Ok(MINERU_ACCOUNT),
        other => Err(format!("Unknown OCR service {other}.")),
    }
}

fn keychain_read(acct: &str) -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, acct)
        .ok()?
        .get_password()
        .ok()
}

fn keychain_write(acct: &str, secret: &str) -> Result<(), String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, acct)
        .and_then(|entry| entry.set_password(secret))
        .map_err(|err| format!("Keychain write failed: {err}"))
}

fn keychain_delete(acct: &str) -> Result<(), String> {
    match keyring::Entry::new(KEYCHAIN_SERVICE, acct).map(|entry| entry.delete_credential()) {
        Ok(Ok(())) => Ok(()),
        // Deleting an absent entry is success from the caller's point of view.
        Ok(Err(keyring::Error::NoEntry)) => Ok(()),
        Ok(Err(err)) => Err(format!("Keychain delete failed: {err}")),
        Err(err) => Err(format!("Keychain delete failed: {err}")),
    }
}

/// The `(key_env, secret)` pairs to inject into OCR worker subprocesses.
/// Empty when nothing is stored — the worker then falls back to `.env`.
pub fn resolve_credential_env() -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some(secret) = keychain_read(PADDLE_ACCOUNT) {
        pairs.push((PADDLE_KEY_ENV.to_string(), secret));
    }
    if let Some(secret) = keychain_read(MINERU_ACCOUNT) {
        pairs.push((MINERU_KEY_ENV.to_string(), secret));
    }
    pairs
}

/// Whether an `.env` file's text sets any of `keys` to a non-empty value.
/// Mirrors the worker's own parser: `KEY=value` lines, comments ignored.
fn env_file_declares(content: &str, keys: &[&str]) -> bool {
    content.lines().any(|raw| {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        keys.contains(&key.trim()) && !value.is_empty()
    })
}

/// Shared with `zotero_settings`: both panels have to tell a person whether a
/// credential is already coming from the repository-root `.env`, and the
/// worker/CLI both read that file the same way.
pub(crate) fn env_fallback_declares(keys: &[&str]) -> bool {
    let Ok(repo_root) = translation_engine_repo_root() else {
        return false;
    };
    let Ok(content) = std::fs::read_to_string(repo_root.join(".env")) else {
        return false;
    };
    env_file_declares(&content, keys)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrServiceStatus {
    pub configured: bool,
    /// "keychain" or "env"; absent when not configured at all.
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrCredentialsStatus {
    pub paddleocr: OcrServiceStatus,
    pub mineru: OcrServiceStatus,
}

fn service_status(account: &str, env_keys: &[&str]) -> OcrServiceStatus {
    if keychain_read(account).is_some() {
        return OcrServiceStatus {
            configured: true,
            source: Some("keychain".into()),
        };
    }
    if env_fallback_declares(env_keys) {
        return OcrServiceStatus {
            configured: true,
            source: Some("env".into()),
        };
    }
    OcrServiceStatus {
        configured: false,
        source: None,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrConnectionResult {
    pub ok: bool,
    pub message: String,
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn get_ocr_credentials_status() -> OcrCredentialsStatus {
    OcrCredentialsStatus {
        paddleocr: service_status(PADDLE_ACCOUNT, &[PADDLE_KEY_ENV]),
        mineru: service_status(MINERU_ACCOUNT, &[MINERU_KEY_ENV, MINERU_KEY_ENV_ALT]),
    }
}

#[tauri::command]
pub fn save_ocr_credential(service: String, api_key: String) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty.".into());
    }
    keychain_write(account_for(&service)?, trimmed)
}

#[tauri::command]
pub fn delete_ocr_credential(service: String) -> Result<(), String> {
    keychain_delete(account_for(&service)?)
}

/// Confirm the token is accepted by the service's API. `api_key` lets the UI
/// test a key the user just typed but has not saved yet; when absent the
/// stored key is used.
#[tauri::command]
pub fn test_ocr_connection(
    service: String,
    api_key: Option<String>,
) -> Result<OcrConnectionResult, String> {
    let account = account_for(&service)?;
    let key = match api_key {
        Some(key) if !key.trim().is_empty() => key.trim().to_string(),
        _ => keychain_read(account)
            .ok_or_else(|| "No API key stored for this service.".to_string())?,
    };
    // Probe a nonexistent job/task id: a bad token gets 401/403 at the auth
    // layer, a good one reaches the handler and gets a not-found/bad-request
    // class answer. Neither submits real work or spends quota.
    let probe_url = match service.as_str() {
        "paddleocr" => {
            "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs/bibliosmith-connectivity-probe"
        }
        "mineru" => "https://mineru.net/api/v4/extract/task/bibliosmith-connectivity-probe",
        _ => unreachable!("account_for validated the service"),
    };
    Ok(match probe_auth(probe_url, &key) {
        Ok(()) => OcrConnectionResult {
            ok: true,
            message: "连接正常，密钥有效".into(),
        },
        Err(message) => OcrConnectionResult { ok: false, message },
    })
}

fn probe_auth(url: &str, key: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;
    let response = client
        .get(url)
        .bearer_auth(key)
        .send()
        .map_err(|err| format!("请求失败：{err}"))?;
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        let body = response.text().unwrap_or_default();
        return Err(format!(
            "HTTP {status}（密钥或权限问题）：{}",
            body.chars().take(200).collect::<String>()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_detection_requires_a_non_empty_value() {
        let content = "# comment\nBAIDU_PADDLEOCR_TOKEN=\nMINERU_TOKEN=abc123\n";
        assert!(!env_file_declares(content, &["BAIDU_PADDLEOCR_TOKEN"]));
        assert!(env_file_declares(
            content,
            &["MINERU_API_TOKEN", "MINERU_TOKEN"]
        ));
        assert!(!env_file_declares("", &["BAIDU_PADDLEOCR_TOKEN"]));
    }

    #[test]
    fn env_file_detection_strips_quotes() {
        assert!(env_file_declares(
            "BAIDU_PADDLEOCR_TOKEN=\"tok\"\n",
            &["BAIDU_PADDLEOCR_TOKEN"]
        ));
        assert!(!env_file_declares(
            "BAIDU_PADDLEOCR_TOKEN=\"\"\n",
            &["BAIDU_PADDLEOCR_TOKEN"]
        ));
    }

    #[test]
    fn unknown_services_are_rejected() {
        assert!(account_for("paddleocr").is_ok());
        assert!(account_for("mineru").is_ok());
        assert!(account_for("tesseract").is_err());
    }
}
