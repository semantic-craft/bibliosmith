//! Model provider configuration for the translation pipeline.
//!
//! The engine's `providers.toml` is the registry of slots (endpoint + default
//! model + which env var carries the key). This module adds the two things a
//! desktop app needs on top of that registry: somewhere for the user to store a
//! key, and a way to feed it to the engine at run time.
//!
//! Keys live in the macOS Keychain, one entry per `(profile, config)` slot —
//! never in a file, so providers and supported billing routes stay distinct.
//! The active selection, Qwen's optional Workspace ID, and its web-search
//! preference are not secret and live in launcher config alongside the proxy
//! and repo settings.
//!
//! At translate time the Rust runner reads the key from the Keychain and injects
//! it into the engine subprocess under the slot's `key_env`, so the engine's
//! credential path is unchanged: it still just reads an environment variable.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::book_pipeline::translation_engine_repo_root;

const KEYCHAIN_SERVICE: &str = "com.bibliosmith.launcher.models";
const QWEN_PROFILE_ID: &str = "qwen";
const QWEN_CONFIG_ID: &str = "payg";

/// A slot as declared in the engine registry. Only the fields the launcher needs
/// are modelled; toml ignores the rest (timeouts, limits).
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrySlot {
    pub profile_id: String,
    pub config_id: String,
    pub provider_type: String,
    pub base_url: String,
    pub base_url_env: Option<String>,
    pub web_search_env: Option<String>,
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

/// One slot as presented to the settings UI: the registry facts, whether a key
/// is stored, and any non-secret workspace selection. Display names and model
/// preset lists are the frontend's concern; this is only what the backend knows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSlotView {
    pub profile_id: String,
    pub config_id: String,
    pub provider_type: String,
    pub default_model: String,
    pub configured: bool,
    pub workspace_id: Option<String>,
    pub web_search_enabled: Option<bool>,
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

fn normalize_qwen_workspace_id(value: &str) -> Result<Option<String>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let Some(identifier) = value.strip_prefix("ws-") else {
        return Err("Workspace ID must start with ws-.".into());
    };
    if identifier.is_empty()
        || !identifier
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    {
        return Err("Enter the Workspace ID only, for example ws-xxxxxxxx.".into());
    }
    Ok(Some(value.to_string()))
}

fn qwen_base_url(workspace_id: &str) -> String {
    format!("https://{workspace_id}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1")
}

fn qwen_slot_with_workspace(
    slot: &RegistrySlot,
    workspace_id: Option<&str>,
) -> Result<RegistrySlot, String> {
    let mut resolved = slot.clone();
    if slot.profile_id == QWEN_PROFILE_ID && slot.config_id == QWEN_CONFIG_ID {
        resolved.base_url = match workspace_id {
            Some(value) => normalize_qwen_workspace_id(value)?
                .map(|value| qwen_base_url(&value))
                .unwrap_or_else(|| slot.base_url.clone()),
            None => slot.base_url.clone(),
        };
    }
    Ok(resolved)
}

pub fn base_url_env(
    slots: &[RegistrySlot],
    profile_id: &str,
    config_id: &str,
    read_workspace_id: impl Fn(&str) -> Option<String>,
) -> Option<(String, String)> {
    let slot = find_slot(slots, profile_id, config_id)?;
    let variable = slot.base_url_env.as_ref()?;
    let workspace_id = read_workspace_id(&account(profile_id, config_id))?;
    let workspace_id = normalize_qwen_workspace_id(&workspace_id).ok()??;
    Some((variable.clone(), qwen_base_url(&workspace_id)))
}

pub fn web_search_env(
    slots: &[RegistrySlot],
    profile_id: &str,
    config_id: &str,
    read_enabled: impl Fn(&str) -> bool,
) -> Option<(String, String)> {
    let slot = find_slot(slots, profile_id, config_id)?;
    let variable = slot.web_search_env.as_ref()?;
    Some((
        variable.clone(),
        read_enabled(&account(profile_id, config_id)).to_string(),
    ))
}

/// Return only a registry-backed selection. Older launchers exposed Qwen Token
/// Plan for batch translation; current provider terms do not. Map that retired
/// selection to Qwen pay-as-you-go without reading, moving, or deleting the old
/// Keychain credential.
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

/// The selection to write back when the normalization above replaced the stored
/// one, or None to leave the stored value alone. Pure, so the write-back rule is
/// unit-testable without a launcher config. A slot that is merely absent — a
/// repoRoot pointing at a worktree whose registry differs — normalizes to None
/// and must never clear what the user chose; only a concrete replacement is
/// persisted.
fn active_model_migration(
    stored: Option<&ActiveModel>,
    normalized: Option<&ActiveModel>,
) -> Option<ActiveModel> {
    let normalized = normalized?;
    (stored != Some(normalized)).then(|| normalized.clone())
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

/// An OpenAI-wire endpoint, resolved for a consumer that speaks the protocol
/// directly rather than through the translation engine. BabelDOC is the only one
/// today: it takes `--openai-base-url/--openai-api-key/--openai-model` and has no
/// notion of the engine's provider registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatibleEndpoint {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// The engine pools several keys per slot and rotates them; a single subprocess
/// wants one. Splitting the same way `normalize_api_keys` does keeps a pooled
/// slot usable here instead of sending the whole comma-separated blob as a key.
fn first_api_key(secret: &str) -> Option<&str> {
    secret
        .split(['\n', '\r', ','])
        .map(str::trim)
        .find(|key| !key.is_empty())
}

/// Resolve the active slot into an OpenAI-wire endpoint, or say why it cannot be.
///
/// Pure: both the Keychain read and the workspace lookup are supplied, so the
/// provider rules are unit-testable without a real store. `openai-responses`
/// slots are accepted -- Qwen and Volcengine publish OpenAI-compatible base URLs
/// and the engine merely prefers the Responses API for them -- while a native
/// Gemini endpoint is a different wire protocol and is refused by name.
pub fn openai_compatible_endpoint(
    slots: &[RegistrySlot],
    active: Option<&ActiveModel>,
    read_secret: impl Fn(&str) -> Option<String>,
    read_workspace_id: impl Fn() -> Option<String>,
) -> Result<OpenAiCompatibleEndpoint, String> {
    let active = active.ok_or_else(|| {
        "No translation model is selected. Choose one in Settings first.".to_string()
    })?;
    let slot = find_slot(slots, &active.profile_id, &active.config_id).ok_or_else(|| {
        format!(
            "The selected model {}/{} is not in the provider registry.",
            active.profile_id, active.config_id
        )
    })?;
    if !matches!(
        slot.provider_type.as_str(),
        "openai-compatible" | "openai-responses"
    ) {
        return Err(format!(
            "{}/{} speaks {}, which is not an OpenAI-compatible endpoint. The layout-preserving track needs one; pick another model in Settings.",
            active.profile_id, active.config_id, slot.provider_type
        ));
    }
    let resolved = qwen_slot_with_workspace(slot, read_workspace_id().as_deref())?;
    let secret = read_secret(&account(&active.profile_id, &active.config_id)).ok_or_else(|| {
        format!(
            "No API key is stored for {}/{}. Add one in Settings first.",
            active.profile_id, active.config_id
        )
    })?;
    let api_key = first_api_key(&secret)
        .ok_or_else(|| {
            format!(
                "The stored API key for {}/{} is empty.",
                active.profile_id, active.config_id
            )
        })?
        .to_string();
    let model = if active.model.trim().is_empty() {
        resolved.model.clone()
    } else {
        active.model.clone()
    };
    Ok(OpenAiCompatibleEndpoint {
        base_url: resolved.base_url,
        api_key,
        model,
    })
}

/// `openai_compatible_endpoint` against the real Keychain and launcher config.
pub fn resolve_openai_compatible_endpoint(
    repo_root: &Path,
) -> Result<OpenAiCompatibleEndpoint, String> {
    let slots = load_slots(repo_root)?;
    let stored = crate::read_active_model();
    let active = active_model_for_catalog(&slots, stored);
    openai_compatible_endpoint(&slots, active.as_ref(), keychain_read, || {
        crate::read_qwen_workspace_id()
    })
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

pub fn resolve_base_url_env(
    repo_root: &Path,
    profile_id: &str,
    config_id: &str,
) -> Option<(String, String)> {
    let slots = load_slots(repo_root).ok()?;
    base_url_env(&slots, profile_id, config_id, |_| {
        crate::read_qwen_workspace_id()
    })
}

pub fn resolve_web_search_env(
    repo_root: &Path,
    profile_id: &str,
    config_id: &str,
) -> Option<(String, String)> {
    let slots = load_slots(repo_root).ok()?;
    web_search_env(&slots, profile_id, config_id, |_| {
        crate::read_qwen_web_search_enabled()
    })
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn get_model_catalog() -> Result<ModelCatalog, String> {
    let repo_root = translation_engine_repo_root()?;
    let slots = load_slots(&repo_root)?;
    let stored = crate::read_active_model();
    let active = active_model_for_catalog(&slots, stored.clone());
    // The normalization used to reach the settings UI and nowhere else: every
    // other reader — apply_active_model_to_manifest above all — goes straight to
    // the stored value. That left a retired selection translating on the
    // registry default while Settings showed the replacement as already active
    // and so disabled the button that would have written it, with no way for the
    // user to correct it. Persist the replacement here instead.
    if let Some(migrated) = active_model_migration(stored.as_ref(), active.as_ref()) {
        crate::write_active_model(Some(migrated))?;
    }
    let views = slots
        .into_iter()
        .map(|slot| {
            let workspace_id =
                if slot.profile_id == QWEN_PROFILE_ID && slot.config_id == QWEN_CONFIG_ID {
                    crate::read_qwen_workspace_id()
                } else {
                    None
                };
            let web_search_enabled =
                if slot.profile_id == QWEN_PROFILE_ID && slot.config_id == QWEN_CONFIG_ID {
                    Some(crate::read_qwen_web_search_enabled())
                } else {
                    None
                };
            ModelSlotView {
                configured: keychain_read(&account(&slot.profile_id, &slot.config_id)).is_some(),
                profile_id: slot.profile_id,
                config_id: slot.config_id,
                provider_type: slot.provider_type,
                default_model: slot.model,
                workspace_id,
                web_search_enabled,
            }
        })
        .collect();
    Ok(ModelCatalog {
        slots: views,
        active,
    })
}

#[tauri::command]
pub fn save_qwen_settings(workspace_id: String, web_search_enabled: bool) -> Result<(), String> {
    let workspace_id = normalize_qwen_workspace_id(&workspace_id)?;
    crate::write_qwen_settings(workspace_id, web_search_enabled)
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
    let slot = qwen_slot_with_workspace(&slot, crate::read_qwen_workspace_id().as_deref())?;

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

    let web_search_enabled = slot.profile_id == QWEN_PROFILE_ID
        && slot.config_id == QWEN_CONFIG_ID
        && crate::read_qwen_web_search_enabled();
    let probe = probe_endpoint(&slot, &model, &key, web_search_enabled);
    Ok(match probe {
        Ok(()) => ModelConnectionResult {
            ok: true,
            message: format!("{profile_id} · {model} 连接正常"),
        },
        Err(message) => ModelConnectionResult { ok: false, message },
    })
}

fn probe_endpoint(
    slot: &RegistrySlot,
    model: &str,
    key: &str,
    web_search_enabled: bool,
) -> Result<(), String> {
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
        let (url, body) = openai_probe_request(slot, model, web_search_enabled)?;
        client.post(url).bearer_auth(key).json(&body).send()
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

fn openai_probe_request(
    slot: &RegistrySlot,
    model: &str,
    web_search_enabled: bool,
) -> Result<(String, serde_json::Value), String> {
    match slot.provider_type.as_str() {
        "openai-responses" => {
            let mut body = serde_json::json!({
                "model": model,
                "input": "ping",
                "store": false
            });
            if web_search_enabled {
                body["tools"] = serde_json::json!([{"type": "web_search"}]);
            }
            Ok((format!("{}/responses", slot.base_url), body))
        }
        "openai-compatible" => Ok((
            format!("{}/chat/completions", slot.base_url),
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1
            }),
        )),
        provider_type => Err(format!("Unsupported provider type {provider_type}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots() -> Vec<RegistrySlot> {
        vec![
            RegistrySlot {
                profile_id: "qwen".into(),
                config_id: "payg".into(),
                provider_type: "openai-responses".into(),
                base_url: "https://payg.example/v1".into(),
                base_url_env: Some("QWEN_API_BASE_URL".into()),
                web_search_env: Some("QWEN_WEB_SEARCH_ENABLED".into()),
                model: "qwen3.7-max".into(),
                key_env: "QWEN_PAYG_API_KEYS".into(),
            },
            RegistrySlot {
                profile_id: "doubao".into(),
                config_id: "cn-beijing".into(),
                provider_type: "openai-responses".into(),
                base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
                base_url_env: None,
                web_search_env: None,
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
    fn responses_slots_build_a_private_responses_probe() {
        let qwen = &slots()[0];
        let (url, body) = openai_probe_request(qwen, "qwen3.7-plus", false)
            .expect("responses slot should produce a probe");

        assert_eq!(url, "https://payg.example/v1/responses");
        assert_eq!(
            body,
            serde_json::json!({
                "model": "qwen3.7-plus",
                "input": "ping",
                "store": false
            })
        );
    }

    #[test]
    fn qwen_web_search_setting_applies_to_the_connection_probe() {
        let qwen = &slots()[0];
        let (_url, body) = openai_probe_request(qwen, "qwen3.7-plus", true)
            .expect("responses slot should produce a probe");

        assert_eq!(body["tools"], serde_json::json!([{"type": "web_search"}]));
        assert!(body.get("enable_search").is_none());
        assert_eq!(body["store"], false);
    }

    #[test]
    fn qwen_workspace_applies_to_connection_and_translation_runtime() {
        let workspace_id = "ws-abc123";
        let qwen = qwen_slot_with_workspace(&slots()[0], Some(workspace_id))
            .expect("valid workspace should produce a Qwen slot");
        let (url, _body) = openai_probe_request(&qwen, "qwen3.7-plus", false)
            .expect("workspace slot should produce a probe");

        assert_eq!(
            url,
            "https://ws-abc123.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/responses"
        );
        assert_eq!(
            base_url_env(&slots(), "qwen", "payg", |_| Some(workspace_id.into())),
            Some((
                "QWEN_API_BASE_URL".into(),
                "https://ws-abc123.cn-beijing.maas.aliyuncs.com/compatible-mode/v1".into()
            ))
        );
        assert_eq!(
            web_search_env(&slots(), "qwen", "payg", |_| true),
            Some(("QWEN_WEB_SEARCH_ENABLED".into(), "true".into()))
        );
        assert_eq!(
            web_search_env(&slots(), "qwen", "payg", |_| false),
            Some(("QWEN_WEB_SEARCH_ENABLED".into(), "false".into()))
        );
    }

    #[test]
    fn qwen_workspace_accepts_an_id_only_and_blank_restores_the_shared_host() {
        assert_eq!(normalize_qwen_workspace_id("  ").unwrap(), None);
        assert_eq!(
            normalize_qwen_workspace_id(" ws-abc123 ").unwrap(),
            Some("ws-abc123".into())
        );
        assert!(normalize_qwen_workspace_id("ws-abc123.cn-beijing.maas.aliyuncs.com").is_err());
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
        assert_eq!(
            find_slot(&slots, "qwen", "payg")
                .expect("qwen slot")
                .provider_type,
            "openai-responses"
        );
        assert_eq!(
            find_slot(&slots, "qwen", "payg")
                .expect("qwen slot")
                .base_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(
            find_slot(&slots, "qwen", "payg")
                .expect("qwen slot")
                .base_url_env
                .as_deref(),
            Some("QWEN_API_BASE_URL")
        );
        assert_eq!(
            find_slot(&slots, "qwen", "payg")
                .expect("qwen slot")
                .web_search_env
                .as_deref(),
            Some("QWEN_WEB_SEARCH_ENABLED")
        );
        assert_eq!(
            find_slot(&slots, "doubao", "cn-beijing")
                .expect("doubao slot")
                .provider_type,
            "openai-responses"
        );
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

    #[test]
    fn a_migrated_selection_is_written_back() {
        let stored = ActiveModel {
            profile_id: "qwen".into(),
            config_id: "token-plan".into(),
            model: "qwen3.8-max-preview".into(),
        };
        let normalized = active_model_for_catalog(&slots(), Some(stored.clone()));

        let migrated = active_model_migration(Some(&stored), normalized.as_ref())
            .expect("a replacement must be persisted");
        assert_eq!(migrated.config_id, "payg");
    }

    #[test]
    fn a_selection_the_registry_still_lists_is_not_rewritten() {
        let stored = ActiveModel {
            profile_id: "qwen".into(),
            config_id: "payg".into(),
            model: "qwen3.7-max".into(),
        };
        let normalized = active_model_for_catalog(&slots(), Some(stored.clone()));

        assert!(active_model_migration(Some(&stored), normalized.as_ref()).is_none());
    }

    // A repoRoot pointing at a worktree with a different registry makes an
    // otherwise valid slot look absent. Clearing the selection there would lose
    // the user's choice for a reason that has nothing to do with them.
    #[test]
    fn an_absent_slot_leaves_the_stored_selection_alone() {
        let stored = ActiveModel {
            profile_id: "nope".into(),
            config_id: "payg".into(),
            model: "whatever".into(),
        };
        let normalized = active_model_for_catalog(&slots(), Some(stored.clone()));

        assert!(normalized.is_none());
        assert!(active_model_migration(Some(&stored), normalized.as_ref()).is_none());
    }

    // ---- OpenAI-compatible endpoint (layout-preserving PDF track) ----------

    fn endpoint_slots() -> Vec<RegistrySlot> {
        let mut slots = slots();
        slots.push(RegistrySlot {
            profile_id: "deepseek".into(),
            config_id: "payg".into(),
            provider_type: "openai-compatible".into(),
            base_url: "https://api.deepseek.com".into(),
            base_url_env: None,
            web_search_env: None,
            model: "deepseek-v4-flash".into(),
            key_env: "DEEPSEEK_API_KEYS".into(),
        });
        slots.push(RegistrySlot {
            profile_id: "gemini".into(),
            config_id: "payg".into(),
            provider_type: "gemini-native".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            base_url_env: None,
            web_search_env: None,
            model: "gemini-2.5-flash".into(),
            key_env: "GEMINI_API_KEYS".into(),
        });
        slots
    }

    fn active(profile_id: &str, config_id: &str, model: &str) -> ActiveModel {
        ActiveModel {
            profile_id: profile_id.into(),
            config_id: config_id.into(),
            model: model.into(),
        }
    }

    #[test]
    fn an_openai_compatible_slot_resolves_to_its_endpoint() {
        let chosen = active("deepseek", "payg", "deepseek-v4-flash");
        let endpoint = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |acct| {
                assert_eq!(acct, "deepseek/payg");
                Some("sk-deepseek".into())
            },
            || None,
        )
        .unwrap();

        assert_eq!(
            endpoint,
            OpenAiCompatibleEndpoint {
                base_url: "https://api.deepseek.com".into(),
                api_key: "sk-deepseek".into(),
                model: "deepseek-v4-flash".into(),
            }
        );
    }

    // Qwen and Volcengine publish OpenAI-compatible base URLs; the engine merely
    // prefers the Responses API for them. Refusing these would leave the track
    // unusable for the providers this user actually runs on.
    #[test]
    fn an_openai_responses_slot_is_accepted_too() {
        let chosen = active("doubao", "cn-beijing", "doubao-seed-2-1-pro-260628");
        let endpoint = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |_| Some("ark-secret".into()),
            || None,
        )
        .unwrap();

        assert_eq!(endpoint.base_url, "https://ark.cn-beijing.volces.com/api/v3");
    }

    #[test]
    fn a_native_gemini_slot_is_refused_by_name() {
        let chosen = active("gemini", "payg", "gemini-2.5-flash");
        let error = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |_| Some("gemini-secret".into()),
            || None,
        )
        .unwrap_err();

        assert!(error.contains("gemini-native"), "unexpected error: {error}");
        assert!(
            error.contains("OpenAI-compatible"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_qwen_workspace_url_overrides_the_registry_base_url() {
        let chosen = active("qwen", "payg", "qwen3.7-max");
        let endpoint = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |_| Some("qwen-secret".into()),
            || Some("ws-abc123".into()),
        )
        .unwrap();

        assert_eq!(
            endpoint.base_url,
            "https://ws-abc123.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"
        );
    }

    // The engine pools several keys per slot and rotates them. A subprocess
    // handed the whole comma-separated blob authenticates as nothing.
    #[test]
    fn a_pooled_slot_contributes_only_its_first_key() {
        let chosen = active("deepseek", "payg", "deepseek-v4-flash");
        let endpoint = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |_| Some(" sk-first , sk-second \n sk-third ".into()),
            || None,
        )
        .unwrap();

        assert_eq!(endpoint.api_key, "sk-first");
    }

    #[test]
    fn the_model_chosen_in_settings_wins_over_the_registry_default() {
        let chosen = active("deepseek", "payg", "deepseek-v4-reasoner");
        let endpoint = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |_| Some("sk-deepseek".into()),
            || None,
        )
        .unwrap();

        assert_eq!(endpoint.model, "deepseek-v4-reasoner");
    }

    #[test]
    fn an_empty_model_selection_falls_back_to_the_registry_default() {
        let chosen = active("deepseek", "payg", "   ");
        let endpoint = openai_compatible_endpoint(
            &endpoint_slots(),
            Some(&chosen),
            |_| Some("sk-deepseek".into()),
            || None,
        )
        .unwrap();

        assert_eq!(endpoint.model, "deepseek-v4-flash");
    }

    #[test]
    fn no_selection_and_no_stored_key_both_say_what_to_do() {
        let no_selection =
            openai_compatible_endpoint(&endpoint_slots(), None, |_| None, || None).unwrap_err();
        assert!(
            no_selection.contains("Settings"),
            "unexpected error: {no_selection}"
        );

        let chosen = active("deepseek", "payg", "deepseek-v4-flash");
        let no_key =
            openai_compatible_endpoint(&endpoint_slots(), Some(&chosen), |_| None, || None)
                .unwrap_err();
        assert!(
            no_key.contains("No API key is stored"),
            "unexpected error: {no_key}"
        );
    }

    #[test]
    fn a_slot_missing_from_the_registry_is_named_rather_than_guessed_at() {
        let chosen = active("nonesuch", "payg", "whatever");
        let error =
            openai_compatible_endpoint(&endpoint_slots(), Some(&chosen), |_| None, || None)
                .unwrap_err();

        assert!(
            error.contains("nonesuch/payg"),
            "unexpected error: {error}"
        );
    }
}
