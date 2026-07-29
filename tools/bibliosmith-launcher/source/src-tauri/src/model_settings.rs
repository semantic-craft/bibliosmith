//! Model provider configuration for the translation pipeline.
//!
//! The engine's `providers.toml` is the registry of slots (endpoint + default
//! model + which env var carries the key). This module adds the two things a
//! desktop app needs on top of that registry: somewhere for the user to store a
//! key, and a way to feed it to the engine at run time.
//!
//! Keys live in the macOS Keychain, one entry per `(profile, config)` slot —
//! never in a file, so providers and supported billing routes stay distinct.
//! The active selection (which slot and model to translate with) is not secret
//! and lives in the launcher config alongside the proxy and repo settings.
//!
//! At translate time the Rust runner reads the key from the Keychain and injects
//! it into the engine subprocess under the slot's `key_env`, so the engine's
//! credential path is unchanged: it still just reads an environment variable.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::book_pipeline::translation_engine_repo_root;

const KEYCHAIN_SERVICE: &str = "com.bibliosmith.launcher.models";

/// A slot as declared in the engine registry. Only the fields the launcher needs
/// are modelled; toml ignores the rest (timeouts, limits).
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrySlot {
    pub profile_id: String,
    pub config_id: String,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub key_env: String,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    providers: Vec<RegistrySlot>,
}

/// The user's chosen translation model. Stored in the launcher config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    pub profile_id: String,
    pub config_id: String,
    pub model: String,
}

/// One slot as presented to the settings UI: the registry facts plus whether a
/// key is stored for it. Display names and model preset lists are the frontend's
/// concern; this is only what the backend can know.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSlotView {
    pub profile_id: String,
    pub config_id: String,
    pub provider_type: String,
    pub default_model: String,
    pub configured: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub slots: Vec<ModelSlotView>,
    pub active: Option<ActiveModel>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConnectionResult {
    pub ok: bool,
    pub message: String,
}

/// Keychain account for a slot. Encodes the provider configuration so unrelated
/// credentials are always separate entries.
fn account(profile_id: &str, config_id: &str) -> String {
    format!("{profile_id}/{config_id}")
}

fn registry_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root
        .join("packages")
        .join("translation-engine")
        .join("src")
        .join("translation_engine")
        .join("providers.toml")
}

fn load_slots(repo_root: &Path) -> Result<Vec<RegistrySlot>, String> {
    let path = registry_path(repo_root);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("Could not read {}: {err}", path.display()))?;
    let parsed: RegistryFile =
        toml::from_str(&text).map_err(|err| format!("Could not parse providers.toml: {err}"))?;
    Ok(parsed.providers)
}

fn find_slot<'a>(
    slots: &'a [RegistrySlot],
    profile_id: &str,
    config_id: &str,
) -> Option<&'a RegistrySlot> {
    slots
        .iter()
        .find(|slot| slot.profile_id == profile_id && slot.config_id == config_id)
}

/// Return only a registry-backed selection to the UI. Older launchers exposed
/// Qwen Token Plan for batch translation; current provider terms do not. Map
/// that retired selection to Qwen pay-as-you-go without reading, moving, or
/// deleting the old Keychain credential.
fn active_model_for_catalog(
    slots: &[RegistrySlot],
    active: Option<ActiveModel>,
) -> Option<ActiveModel> {
    let active = active?;
    if find_slot(slots, &active.profile_id, &active.config_id).is_some() {
        return Some(active);
    }
    if active.profile_id == "qwen" && active.config_id == "token-plan" {
        let payg = find_slot(slots, "qwen", "payg")?;
        return Some(ActiveModel {
            profile_id: payg.profile_id.clone(),
            config_id: payg.config_id.clone(),
            model: payg.model.clone(),
        });
    }
    None
}

/// The `(key_env, secret)` pair to inject for a slot, or None when the slot is
/// unknown or has no stored key. Pure: the Keychain read is supplied, so this is
/// unit-testable without the real store.
pub fn credential_env(
    slots: &[RegistrySlot],
    profile_id: &str,
    config_id: &str,
    read_secret: impl Fn(&str) -> Option<String>,
) -> Option<(String, String)> {
    let slot = find_slot(slots, profile_id, config_id)?;
    let secret = read_secret(&account(profile_id, config_id))?;
    Some((slot.key_env.clone(), secret))
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

/// Resolve the injection pair for the translate/sample runner using the real
/// Keychain. Returns None (rather than erroring) whenever no key is stored, so
/// the runner simply falls back to the engine's `.env` lookup.
pub fn resolve_credential_env(
    repo_root: &Path,
    profile_id: &str,
    config_id: &str,
) -> Option<(String, String)> {
    let slots = load_slots(repo_root).ok()?;
    credential_env(&slots, profile_id, config_id, keychain_read)
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn get_model_catalog() -> Result<ModelCatalog, String> {
    let repo_root = translation_engine_repo_root()?;
    let slots = load_slots(&repo_root)?;
    let active = active_model_for_catalog(&slots, crate::read_active_model());
    let views = slots
        .into_iter()
        .map(|slot| ModelSlotView {
            configured: keychain_read(&account(&slot.profile_id, &slot.config_id)).is_some(),
            profile_id: slot.profile_id,
            config_id: slot.config_id,
            provider_type: slot.provider_type,
            default_model: slot.model,
        })
        .collect();
    Ok(ModelCatalog {
        slots: views,
        active,
    })
}

#[tauri::command]
pub fn save_model_credential(
    profile_id: String,
    config_id: String,
    api_key: String,
) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty.".into());
    }
    // Reject an unknown slot rather than storing an orphan key.
    let repo_root = translation_engine_repo_root()?;
    let slots = load_slots(&repo_root)?;
    if find_slot(&slots, &profile_id, &config_id).is_none() {
        return Err(format!("Unknown provider slot {profile_id}/{config_id}."));
    }
    keychain_write(&account(&profile_id, &config_id), trimmed)
}

#[tauri::command]
pub fn delete_model_credential(profile_id: String, config_id: String) -> Result<(), String> {
    keychain_delete(&account(&profile_id, &config_id))
}

#[tauri::command]
pub fn set_active_model(
    profile_id: String,
    config_id: String,
    model: String,
) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("Model is empty.".into());
    }
    let repo_root = translation_engine_repo_root()?;
    let slots = load_slots(&repo_root)?;
    if find_slot(&slots, &profile_id, &config_id).is_none() {
        return Err(format!("Unknown provider slot {profile_id}/{config_id}."));
    }
    crate::write_active_model(Some(ActiveModel {
        profile_id,
        config_id,
        model: model.to_string(),
    }))
}

/// Send the smallest possible request to confirm the endpoint accepts the key.
/// `api_key` lets the UI test a key the user just typed but has not saved yet;
/// when absent the stored key is used.
#[tauri::command]
pub fn test_model_connection(
    profile_id: String,
    config_id: String,
    model: String,
    api_key: Option<String>,
) -> Result<ModelConnectionResult, String> {
    let repo_root = translation_engine_repo_root()?;
    let slots = load_slots(&repo_root)?;
    let slot = find_slot(&slots, &profile_id, &config_id)
        .ok_or_else(|| format!("Unknown provider slot {profile_id}/{config_id}."))?
        .clone();

    let key = match api_key {
        Some(key) if !key.trim().is_empty() => key.trim().to_string(),
        _ => keychain_read(&account(&profile_id, &config_id))
            .ok_or_else(|| "No API key stored for this slot.".to_string())?,
    };
    let model = if model.trim().is_empty() {
        slot.model.clone()
    } else {
        model.trim().to_string()
    };

    let probe = probe_endpoint(&slot, &model, &key);
    Ok(match probe {
        Ok(()) => ModelConnectionResult {
            ok: true,
            message: format!("{profile_id} · {model} 连接正常"),
        },
        Err(message) => ModelConnectionResult { ok: false, message },
    })
}

fn probe_endpoint(slot: &RegistrySlot, model: &str, key: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;

    let response = if slot.provider_type == "gemini-native" {
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            slot.base_url, model, key
        );
        client
            .post(url)
            .json(&serde_json::json!({
                "contents": [{"parts": [{"text": "ping"}]}],
                "generationConfig": {"maxOutputTokens": 1}
            }))
            .send()
    } else {
        client
            .post(format!("{}/chat/completions", slot.base_url))
            .bearer_auth(key)
            .json(&serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1
            }))
            .send()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn slots() -> Vec<RegistrySlot> {
        vec![
            RegistrySlot {
                profile_id: "qwen".into(),
                config_id: "payg".into(),
                provider_type: "openai-compatible".into(),
                base_url: "https://payg.example/v1".into(),
                model: "qwen3.7-max".into(),
                key_env: "QWEN_PAYG_API_KEYS".into(),
            },
            RegistrySlot {
                profile_id: "doubao".into(),
                config_id: "cn-beijing".into(),
                provider_type: "openai-compatible".into(),
                base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
                model: "doubao-seed-2-1-pro-260628".into(),
                key_env: "VOLCENGINE_ARK_API_KEYS".into(),
            },
        ]
    }

    #[test]
    fn credential_env_pairs_the_slot_key_env_with_the_stored_secret() {
        let got = credential_env(&slots(), "doubao", "cn-beijing", |acct| {
            assert_eq!(acct, "doubao/cn-beijing");
            Some("ark-secret".into())
        });
        assert_eq!(
            got,
            Some(("VOLCENGINE_ARK_API_KEYS".into(), "ark-secret".into()))
        );
    }

    #[test]
    fn different_provider_slots_resolve_to_different_key_envs() {
        let qwen = credential_env(&slots(), "qwen", "payg", |_| Some("k".into())).unwrap();
        let doubao =
            credential_env(&slots(), "doubao", "cn-beijing", |_| Some("k".into())).unwrap();
        assert_ne!(qwen.0, doubao.0);
    }

    #[test]
    fn no_stored_secret_yields_no_injection() {
        assert!(credential_env(&slots(), "qwen", "payg", |_| None).is_none());
    }

    #[test]
    fn an_unknown_slot_yields_no_injection() {
        assert!(credential_env(&slots(), "nope", "payg", |_| Some("k".into())).is_none());
    }

    #[test]
    fn the_shipped_registry_parses_and_carries_the_expected_slots() {
        // This checks the source tree being compiled, not the user's persisted
        // Launcher repoRoot (which may intentionally point at another worktree).
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .ancestors()
            .find(|path| {
                path.join("packages")
                    .join("translation-engine")
                    .join("src")
                    .join("translation_engine")
                    .join("providers.toml")
                    .is_file()
            })
            .expect("source repo root");
        let slots = load_slots(repo_root).expect("registry parses");
        for (profile, config) in [
            ("deepseek", "deepseek-default"),
            ("kimi", "kimi-default"),
            ("qwen", "payg"),
            ("doubao", "cn-beijing"),
            ("mimo", "payg"),
            ("mimo", "token-plan"),
        ] {
            assert!(
                find_slot(&slots, profile, config).is_some(),
                "missing slot {profile}/{config}"
            );
        }
        assert!(find_slot(&slots, "qwen", "token-plan").is_none());
    }

    #[test]
    fn legacy_qwen_token_plan_selection_moves_to_the_payg_default() {
        let active = active_model_for_catalog(
            &slots(),
            Some(ActiveModel {
                profile_id: "qwen".into(),
                config_id: "token-plan".into(),
                model: "qwen3.8-max-preview".into(),
            }),
        )
        .expect("legacy selection should migrate");

        assert_eq!(active.profile_id, "qwen");
        assert_eq!(active.config_id, "payg");
        assert_eq!(active.model, "qwen3.7-max");
    }
}
