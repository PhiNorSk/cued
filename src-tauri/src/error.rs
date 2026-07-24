use serde::ser::SerializeStruct;

/// All failure modes of the auth flow. Serialized to the frontend as
/// `{ code, message }` so the UI can branch on `code` and show `message` inline.
///
/// Invariant: no variant ever carries a token, auth code, code_verifier or
/// state value — messages are safe to display and log.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Port 8917 is already in use. Close the program occupying it and try again.")]
    PortInUse,
    #[error("Timed out waiting for the Spotify login (5 minutes). Please try again.")]
    CallbackTimeout,
    #[error("Login aborted: the returned state did not match. Please try again.")]
    StateMismatch,
    #[error("Access was denied in the Spotify login page.")]
    AccessDenied,
    #[error("Spotify rejected the login: {0}")]
    SpotifyAuth(String),
    #[error("No Spotify Client ID is configured.")]
    NoClientId,
    #[error("Invalid Client ID: {0}")]
    InvalidClientId(String),
    #[error("Keychain error: {0}")]
    Keychain(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Spotify returned an error (HTTP {status}): {}", .detail.as_deref().unwrap_or("no details"))]
    Api { status: u16, detail: Option<String> },
    #[error("Could not understand Spotify's response.")]
    MalformedResponse,
    #[error("Spotify rate limit reached; backing off.")]
    RateLimited {
        /// Value of the `Retry-After` header in seconds, when present.
        retry_after_secs: Option<u64>,
    },
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("A login is already in progress.")]
    LoginInProgress,
    #[error("Received a malformed callback request.")]
    BadCallback,
}

impl AuthError {
    /// Stable machine-readable code for the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            AuthError::PortInUse => "port_in_use",
            AuthError::CallbackTimeout => "callback_timeout",
            AuthError::StateMismatch => "state_mismatch",
            AuthError::AccessDenied => "access_denied",
            AuthError::SpotifyAuth(_) => "spotify_auth",
            AuthError::NoClientId => "no_client_id",
            AuthError::InvalidClientId(_) => "invalid_client_id",
            AuthError::Keychain(_) => "keychain",
            AuthError::Network(_) => "network",
            AuthError::Api { .. } => "api",
            AuthError::MalformedResponse => "malformed_response",
            AuthError::RateLimited { .. } => "rate_limited",
            AuthError::Config(_) => "config",
            AuthError::LoginInProgress => "login_in_progress",
            AuthError::BadCallback => "bad_callback",
        }
    }
}

impl serde::Serialize for AuthError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AuthError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}
