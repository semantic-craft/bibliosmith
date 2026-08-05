//! Credentials for writing back into the user's Zotero library.
//!
//! Reading a library needs nothing: the conversion worker talks to the local
//! Zotero SQLite. Putting a finished book *into* Zotero goes through the Web
//! API, which needs a personal API key plus the library it belongs to. The
//! three values live in the operating system Keychain, one account each, and
//! are injected into the `zsearch` subprocess environment at run time.

use serde::Serialize;

const KEYCHAIN_SERVICE: &str = "com.bibliosmith.launcher.models";

const API_KEY_ACCOUNT: &str = "zotero/api-key";
const LIBRARY_ID_ACCOUNT: &str = "zotero/library-id";
const LIBRARY_TYPE_ACCOUNT: &str = "zotero/library-type";

/// The exact variables packages/zotero-cli reads in `zotero_api._config`.
const API_KEY_ENV: &str = "ZOTERO_API_KEY";
const LIBRARY_ID_ENV: &str = "ZOTERO_LIBRARY_ID";
const LIBRARY_TYPE_ENV: &str = "ZOTERO_LIBRARY_TYPE";

const FIELDS: [(&str, &str, &str); 3] = [
    ("api_key", API_KEY_ACCOUNT, API_KEY_ENV),
    ("library_id", LIBRARY_ID_ACCOUNT, LIBRARY_ID_ENV),
    ("library_type", LIBRARY_TYPE_ACCOUNT, LIBRARY_TYPE_ENV),
];

fn account_for(field: &str) -> Result<&'static str, String> {
    FIELDS
        .iter()
        .find(|(name, _, _)| *name == field)
        .map(|(_, account, _)| *account)
        .ok_or_else(|| format!("Unknown Zotero credential field {field}."))
}

#[cfg(not(test))]
fn keychain_read(acct: &str) -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, acct)
        .ok()?
        .get_password()
        .ok()
        .filter(|secret| !secret.trim().is_empty())
}

#[cfg(test)]
fn keychain_read(_acct: &str) -> Option<String> {
    None
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

/// Normalise what a person typed into what the Zotero Web API path expects.
/// `zotero_api._config` already forgives the singular spelling of `users`; this
/// rejects everything else outright so a typo surfaces at the settings panel
/// rather than as a 404 halfway through an upload.
fn normalized_library_type(value: &str) -> Result<String, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "user" | "users" => Ok("users".into()),
        "group" | "groups" => Ok("groups".into()),
        other => Err(format!(
            "Zotero library type must be \"users\" or \"groups\", not {other:?}."
        )),
    }
}

/// A Zotero library id is the numeric id from the API key page, not the
/// library's name — a wrong one authenticates fine and then writes into
/// nothing, so it is worth refusing here.
fn normalized_library_id(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err("Zotero library id must be the numeric library id.".into());
    }
    Ok(trimmed.into())
}

fn normalized(field: &str, value: &str) -> Result<String, String> {
    match field {
        "api_key" => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err("Zotero API key is empty.".into());
            }
            Ok(trimmed.into())
        }
        "library_id" => normalized_library_id(value),
        "library_type" => normalized_library_type(value),
        other => Err(format!("Unknown Zotero credential field {other}.")),
    }
}

/// The `(key_env, value)` pairs to inject into a `zsearch` write subprocess.
/// Empty when nothing is stored.
pub fn resolve_credential_env() -> Vec<(String, String)> {
    FIELDS
        .iter()
        .filter_map(|(_, account, key_env)| {
            keychain_read(account).map(|secret| (key_env.to_string(), secret))
        })
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroFieldStatus {
    pub configured: bool,
    /// "keychain"; absent when not configured at all.
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroCredentialsStatus {
    pub api_key: ZoteroFieldStatus,
    pub library_id: ZoteroFieldStatus,
    pub library_type: ZoteroFieldStatus,
}

fn field_status(account: &str) -> ZoteroFieldStatus {
    if keychain_read(account).is_some() {
        return ZoteroFieldStatus {
            configured: true,
            source: Some("keychain".into()),
        };
    }
    ZoteroFieldStatus {
        configured: false,
        source: None,
    }
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn get_zotero_credentials_status() -> ZoteroCredentialsStatus {
    ZoteroCredentialsStatus {
        api_key: field_status(API_KEY_ACCOUNT),
        library_id: field_status(LIBRARY_ID_ACCOUNT),
        library_type: field_status(LIBRARY_TYPE_ACCOUNT),
    }
}

#[tauri::command]
pub fn save_zotero_credential(field: String, value: String) -> Result<(), String> {
    keychain_write(account_for(&field)?, &normalized(&field, &value)?)
}

#[tauri::command]
pub fn delete_zotero_credential(field: String) -> Result<(), String> {
    keychain_delete(account_for(&field)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_type_accepts_both_spellings_and_rejects_the_rest() {
        assert_eq!(normalized_library_type(" Users ").unwrap(), "users");
        assert_eq!(normalized_library_type("user").unwrap(), "users");
        assert_eq!(normalized_library_type("GROUP").unwrap(), "groups");
        assert!(normalized_library_type("library").is_err());
        assert!(normalized_library_type("").is_err());
    }

    #[test]
    fn library_id_must_be_the_numeric_id() {
        assert_eq!(normalized_library_id(" 12345 ").unwrap(), "12345");
        // The library's display name is the mistake this catches.
        assert!(normalized_library_id("My Library").is_err());
        assert!(normalized_library_id("").is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(account_for("api_key").is_ok());
        assert!(account_for("library_id").is_ok());
        assert!(account_for("library_type").is_ok());
        assert!(account_for("api-key").is_err());
        assert!(normalized("collection_key", "ABCD").is_err());
    }

    /// The env names are the contract with `zotero_api._config`; a rename on
    /// either side has to be a deliberate edit in both places.
    #[test]
    fn injected_variables_are_the_ones_the_cli_reads() {
        assert_eq!(
            FIELDS.map(|(_, _, key_env)| key_env),
            ["ZOTERO_API_KEY", "ZOTERO_LIBRARY_ID", "ZOTERO_LIBRARY_TYPE"]
        );
    }
}
