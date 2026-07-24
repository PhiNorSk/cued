use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::error::AuthError;

/// Spotify Client IDs are 32 hex chars today; we accept 20–40 alphanumeric
/// ASCII chars to survive minor format changes without letting garbage through.
const CLIENT_ID_MIN: usize = 20;
const CLIENT_ID_MAX: usize = 40;

/// Validate a user-supplied Spotify Client ID before it is used or stored.
pub fn validate_client_id(client_id: &str) -> Result<(), AuthError> {
    if !(CLIENT_ID_MIN..=CLIENT_ID_MAX).contains(&client_id.len()) {
        return Err(AuthError::InvalidClientId(format!(
            "expected {CLIENT_ID_MIN}–{CLIENT_ID_MAX} characters, got {}",
            client_id.len()
        )));
    }
    if !client_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(AuthError::InvalidClientId(
            "only letters and digits are allowed".into(),
        ));
    }
    Ok(())
}

/// On-disk app configuration. The Client ID is not a secret (it is public in
/// every authorize URL), so a plain JSON file is fine — tokens are NOT here.
#[derive(Debug, Default, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    client_id: Option<String>,
    /// Master toggle of the auto-seek/skip engine; absent means ON.
    #[serde(default)]
    automation_enabled: Option<bool>,
    /// Master toggle of listening-insights collection (M9); absent means ON.
    #[serde(default)]
    insights_enabled: Option<bool>,
    /// Per-type suggestion toggles (M10); each absent means ON.
    #[serde(default)]
    suggest_skip_points: Option<bool>,
    #[serde(default)]
    suggest_start_points: Option<bool>,
    #[serde(default)]
    suggest_auto_skip: Option<bool>,
}

/// The three per-type suggestion toggles (M10). Each gates ONLY whether that
/// kind of suggestion is surfaced; the master insights toggle gates all of
/// them. Applied auto-skips keep working regardless (they behave like a
/// committed preset, reversed from the Library).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionToggles {
    pub skip_points: bool,
    pub start_points: bool,
    pub auto_skip: bool,
}

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, AuthError> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AuthError::Config(format!("cannot resolve config dir: {e}")))?;
    Ok(dir.join("config.json"))
}

/// Read the whole config. A missing file is the default config, not an error.
fn load_config(app: &tauri::AppHandle) -> Result<AppConfig, AuthError> {
    let path = config_path(app)?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AppConfig::default()),
        Err(e) => return Err(AuthError::Config(format!("cannot read config file: {e}"))),
    };
    serde_json::from_str(&raw).map_err(|e| AuthError::Config(format!("bad config file: {e}")))
}

/// Write the whole config file (creating the directory if needed).
fn write_config(app: &tauri::AppHandle, cfg: &AppConfig) -> Result<(), AuthError> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AuthError::Config(format!("cannot create config dir: {e}")))?;
    }
    let raw = serde_json::to_string_pretty(cfg)
        .map_err(|e| AuthError::Config(format!("cannot serialize config: {e}")))?;
    fs::write(&path, raw).map_err(|e| AuthError::Config(format!("cannot write config file: {e}")))
}

/// Read the stored Client ID, if any. A missing or unreadable config file is
/// treated as "not configured", not as an error.
pub fn load_client_id(app: &tauri::AppHandle) -> Result<Option<String>, AuthError> {
    match load_config(app)?.client_id {
        Some(id) => {
            validate_client_id(&id)?;
            Ok(Some(id))
        }
        None => Ok(None),
    }
}

/// Validate and persist the Client ID to the config file.
pub fn save_client_id(app: &tauri::AppHandle, client_id: &str) -> Result<(), AuthError> {
    validate_client_id(client_id)?;
    // An unparseable existing file heals here instead of blocking the save.
    let mut cfg = load_config(app).unwrap_or_default();
    cfg.client_id = Some(client_id.to_owned());
    write_config(app, &cfg)
}

/// Whether the auto-seek/skip engine is enabled (default: on).
pub fn load_automation_enabled(app: &tauri::AppHandle) -> Result<bool, AuthError> {
    Ok(load_config(app)?.automation_enabled.unwrap_or(true))
}

/// Persist the automation master toggle.
pub fn save_automation_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), AuthError> {
    let mut cfg = load_config(app).unwrap_or_default();
    cfg.automation_enabled = Some(enabled);
    write_config(app, &cfg)
}

/// Whether listening-insights collection is enabled (default: on).
pub fn load_insights_enabled(app: &tauri::AppHandle) -> Result<bool, AuthError> {
    Ok(load_config(app)?.insights_enabled.unwrap_or(true))
}

/// Persist the listening-insights master toggle.
pub fn save_insights_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), AuthError> {
    let mut cfg = load_config(app).unwrap_or_default();
    cfg.insights_enabled = Some(enabled);
    write_config(app, &cfg)
}

/// The per-type suggestion toggles (each defaults on).
pub fn load_suggestion_toggles(app: &tauri::AppHandle) -> Result<SuggestionToggles, AuthError> {
    let cfg = load_config(app)?;
    Ok(SuggestionToggles {
        skip_points: cfg.suggest_skip_points.unwrap_or(true),
        start_points: cfg.suggest_start_points.unwrap_or(true),
        auto_skip: cfg.suggest_auto_skip.unwrap_or(true),
    })
}

/// Persist the per-type suggestion toggles.
pub fn save_suggestion_toggles(
    app: &tauri::AppHandle,
    toggles: SuggestionToggles,
) -> Result<(), AuthError> {
    let mut cfg = load_config(app).unwrap_or_default();
    cfg.suggest_skip_points = Some(toggles.skip_points);
    cfg.suggest_start_points = Some(toggles.start_points);
    cfg.suggest_auto_skip = Some(toggles.auto_skip);
    write_config(app, &cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_real_looking_client_id() {
        // 32 hex chars, the current Spotify format
        assert!(validate_client_id("5f573c9620494bae87890c0f08a60293").is_ok());
    }

    #[test]
    fn accepts_mixed_case_alphanumeric() {
        assert!(validate_client_id("Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z").is_ok());
    }

    #[test]
    fn rejects_too_short() {
        let err = validate_client_id("abc123").expect_err("too-short id must be rejected");
        assert!(matches!(err, AuthError::InvalidClientId(_)));
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(CLIENT_ID_MAX + 1);
        assert!(validate_client_id(&long).is_err());
    }

    #[test]
    fn rejects_illegal_chars() {
        let err = validate_client_id("5f573c96-2049-4bae-8789-0c0f08a602")
            .expect_err("dashes must be rejected");
        assert!(matches!(err, AuthError::InvalidClientId(_)));
    }

    #[test]
    fn rejects_whitespace_and_empty() {
        assert!(validate_client_id("").is_err());
        assert!(validate_client_id("5f573c9620494bae87890c0f08a6029 ").is_err());
    }

    #[test]
    fn boundary_lengths_are_accepted() {
        assert!(validate_client_id(&"a".repeat(CLIENT_ID_MIN)).is_ok());
        assert!(validate_client_id(&"a".repeat(CLIENT_ID_MAX)).is_ok());
    }
}
