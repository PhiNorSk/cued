use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// Keychain layout: two entries under one service — the access token (with its
/// expiry, as JSON) and the raw refresh token. Tokens never touch disk
/// elsewhere: no localStorage, no plain files, no Tauri store.
const SERVICE: &str = "app.cued.desktop";
const ACCESS_ENTRY: &str = "spotify-access";
const REFRESH_ENTRY: &str = "spotify-refresh";

/// The token set kept in the OS keychain and mirrored in memory while running.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    /// Unix seconds after which the access token is considered expired.
    pub expires_at: u64,
    pub refresh_token: String,
}

#[derive(Serialize, Deserialize)]
struct AccessEntry {
    access_token: String,
    expires_at: u64,
}

fn entry(account: &str) -> Result<keyring::Entry, AuthError> {
    keyring::Entry::new(SERVICE, account).map_err(|e| AuthError::Keychain(e.to_string()))
}

/// Persist both tokens, replacing any previous values (refresh rotation).
pub fn save(tokens: &Tokens) -> Result<(), AuthError> {
    let access_json = serde_json::to_string(&AccessEntry {
        access_token: tokens.access_token.clone(),
        expires_at: tokens.expires_at,
    })
    .map_err(|e| AuthError::Keychain(format!("cannot serialize entry: {e}")))?;
    entry(ACCESS_ENTRY)?
        .set_password(&access_json)
        .map_err(|e| AuthError::Keychain(e.to_string()))?;
    entry(REFRESH_ENTRY)?
        .set_password(&tokens.refresh_token)
        .map_err(|e| AuthError::Keychain(e.to_string()))
}

/// Load the stored token set; `None` when not logged in (either entry missing).
pub fn load() -> Result<Option<Tokens>, AuthError> {
    let access_json = match entry(ACCESS_ENTRY)?.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(AuthError::Keychain(e.to_string())),
    };
    let refresh_token = match entry(REFRESH_ENTRY)?.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(AuthError::Keychain(e.to_string())),
    };
    let access: AccessEntry = serde_json::from_str(&access_json)
        .map_err(|_| AuthError::Keychain("stored token entry is corrupt".into()))?;
    Ok(Some(Tokens {
        access_token: access.access_token,
        expires_at: access.expires_at,
        refresh_token,
    }))
}

/// Delete both entries. Missing entries are fine — logout must be idempotent.
pub fn clear() -> Result<(), AuthError> {
    for account in [ACCESS_ENTRY, REFRESH_ENTRY] {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(AuthError::Keychain(e.to_string())),
        }
    }
    Ok(())
}
