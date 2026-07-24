use std::time::Duration;

use serde::Deserialize;

use crate::error::AuthError;

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const ME_URL: &str = "https://api.spotify.com/v1/me";
// `additional_types=episode` is required or Spotify returns `item: null` for
// podcast episodes.
const PLAYER_URL: &str = "https://api.spotify.com/v1/me/player?additional_types=episode";
const SEEK_URL: &str = "https://api.spotify.com/v1/me/player/seek";
const NEXT_URL: &str = "https://api.spotify.com/v1/me/player/next";
const QUEUE_URL: &str = "https://api.spotify.com/v1/me/player/queue";
pub const REDIRECT_URI: &str = "http://127.0.0.1:8917/callback";
pub const SCOPES: &str =
    "user-read-playback-state user-read-currently-playing user-modify-playback-state user-read-private";

/// Error body of the token endpoint (RFC 6749 §5.2) — safe to surface, never
/// contains credentials.
#[derive(Debug, Deserialize)]
struct TokenErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Shared HTTP client; every request gets a hard timeout (a hang is worse
/// than an error).
pub fn build_http_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AuthError::Network(format!("cannot build HTTP client: {e}")))
}

/// Successful response of `POST https://accounts.spotify.com/api/token`.
///
/// `refresh_token` is optional because refresh-grant responses may omit it;
/// when present it rotates (replaces) the stored one.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// Successful response of `GET https://api.spotify.com/v1/me` (fields we use).
#[derive(Debug, Deserialize)]
pub struct MeResponse {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Requires the `user-read-private` scope; `"premium"` for Premium accounts.
    #[serde(default)]
    pub product: Option<String>,
}

/// Parse a token-endpoint response body into a typed struct.
pub fn parse_token_response(body: &str) -> Result<TokenResponse, AuthError> {
    serde_json::from_str(body).map_err(|_| AuthError::MalformedResponse)
}

/// Build the URL of Spotify's authorize page for the system browser.
pub fn build_authorize_url(
    client_id: &str,
    challenge: &str,
    state: &str,
) -> Result<String, AuthError> {
    let url = url::Url::parse_with_params(
        "https://accounts.spotify.com/authorize",
        &[
            ("client_id", client_id),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge),
            ("state", state),
            ("scope", SCOPES),
        ],
    )
    .map_err(|e| AuthError::Config(format!("cannot build authorize URL: {e}")))?;
    Ok(url.into())
}

fn network_err(e: reqwest::Error) -> AuthError {
    // reqwest's Display never includes bodies or credentials; a timeout gets
    // its own wording because it is the most actionable case.
    if e.is_timeout() {
        AuthError::Network("request timed out".into())
    } else {
        AuthError::Network(e.without_url().to_string())
    }
}

async fn token_request(
    http: &reqwest::Client,
    form: &[(&str, &str)],
) -> Result<TokenResponse, AuthError> {
    let resp = http
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(network_err)?;
    let status = resp.status();
    let body = resp.text().await.map_err(network_err)?;
    if !status.is_success() {
        let detail = serde_json::from_str::<TokenErrorBody>(&body)
            .ok()
            .map(|b| b.error_description.unwrap_or(b.error));
        return Err(AuthError::Api {
            status: status.as_u16(),
            detail,
        });
    }
    parse_token_response(&body)
}

/// Exchange the authorization code for tokens (PKCE — no client secret).
pub async fn exchange_code(
    http: &reqwest::Client,
    client_id: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse, AuthError> {
    token_request(
        http,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ],
    )
    .await
}

/// Get a fresh access token; the response may rotate the refresh token.
pub async fn refresh_access_token(
    http: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, AuthError> {
    token_request(
        http,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ],
    )
    .await
}

/// Successful 200 body of `GET /v1/me/player` (fields we use).
///
/// Tolerates the shapes listed in the M2 ticket: ads (`item` null or a
/// non-track type), local files (`is_local`, null `id`), missing album art,
/// and podcast episodes (images live on the item, not on an album).
#[derive(Debug, Deserialize)]
pub struct PlayerResponse {
    pub is_playing: bool,
    #[serde(default)]
    pub progress_ms: Option<u64>,
    #[serde(default)]
    pub item: Option<PlayerItem>,
    /// The active device — automation suspends per device after a 403.
    #[serde(default)]
    pub device: Option<PlayerDevice>,
}

/// The `device` of a player response (fields we use).
#[derive(Debug, Deserialize)]
pub struct PlayerDevice {
    /// May be null for some Connect devices.
    #[serde(default)]
    pub id: Option<String>,
}

/// The `item` of a player response: a track or a podcast episode.
#[derive(Debug, Deserialize)]
pub struct PlayerItem {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
    pub name: String,
    pub duration_ms: u64,
    /// `"track"` or `"episode"`; anything else (e.g. `"ad"`) is not displayable.
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default)]
    pub is_local: bool,
    #[serde(default)]
    pub artists: Vec<PlayerArtist>,
    #[serde(default)]
    pub album: Option<PlayerAlbum>,
    /// Episodes carry their cover images directly (tracks: via `album.images`).
    #[serde(default)]
    pub images: Vec<PlayerImage>,
}

/// Artist entry of a track item.
#[derive(Debug, Deserialize)]
pub struct PlayerArtist {
    pub name: String,
}

/// Album of a track item; only the cover images matter here.
#[derive(Debug, Deserialize)]
pub struct PlayerAlbum {
    #[serde(default)]
    pub images: Vec<PlayerImage>,
}

/// One cover-art rendition. Spotify orders these largest-first.
#[derive(Debug, Deserialize)]
pub struct PlayerImage {
    pub url: String,
}

/// Parse a 200 body of `GET /v1/me/player` into a typed struct.
pub fn parse_player_body(body: &str) -> Result<PlayerResponse, AuthError> {
    serde_json::from_str(body).map_err(|_| AuthError::MalformedResponse)
}

/// Successful 200 body of `GET /v1/me/player/queue` (fields we use).
///
/// Queue results are hints for the M7 prediction only — every field is
/// optional-tolerant, and a short or empty queue is a valid answer.
#[derive(Debug, Deserialize)]
pub struct QueueResponse {
    #[serde(default)]
    pub queue: Vec<QueueItem>,
}

/// One entry of the play queue: a track or a podcast episode.
#[derive(Debug, Deserialize)]
pub struct QueueItem {
    #[serde(default)]
    pub uri: Option<String>,
    /// For log lines only — never displayed.
    #[serde(default)]
    pub name: Option<String>,
    /// `"track"` or `"episode"`; only real tracks can be pre-armed.
    #[serde(rename = "type", default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub is_local: bool,
}

/// Parse a 200 body of `GET /v1/me/player/queue` into a typed struct.
pub fn parse_queue_body(body: &str) -> Result<QueueResponse, AuthError> {
    serde_json::from_str(body).map_err(|_| AuthError::MalformedResponse)
}

/// Fetch the play queue (M7 prediction). A 429 surfaces as
/// `AuthError::RateLimited` carrying the parsed `Retry-After` value.
pub async fn fetch_queue(
    http: &reqwest::Client,
    access_token: &str,
) -> Result<QueueResponse, AuthError> {
    let resp = http
        .get(QUEUE_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(network_err)?;
    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after_secs = parse_retry_after(
            resp.headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        return Err(AuthError::RateLimited { retry_after_secs });
    }
    if !status.is_success() {
        return Err(AuthError::Api {
            status: status.as_u16(),
            detail: None,
        });
    }
    let body = resp.text().await.map_err(network_err)?;
    parse_queue_body(&body)
}

/// Parse a `Retry-After` header value (Spotify sends whole seconds).
/// Anything non-numeric (e.g. an HTTP date) yields `None` so the caller
/// falls back to a safe default.
pub fn parse_retry_after(header: Option<&str>) -> Option<u64> {
    header.and_then(|v| v.trim().parse::<u64>().ok())
}

/// Fetch the current playback state. `Ok(None)` means "no active device"
/// (HTTP 204 or an empty body). A 429 surfaces as `AuthError::RateLimited`
/// carrying the parsed `Retry-After` value.
pub async fn fetch_player(
    http: &reqwest::Client,
    access_token: &str,
) -> Result<Option<PlayerResponse>, AuthError> {
    let resp = http
        .get(PLAYER_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(network_err)?;
    let status = resp.status();
    if status.as_u16() == 204 {
        return Ok(None);
    }
    if status.as_u16() == 429 {
        let retry_after_secs = parse_retry_after(
            resp.headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        return Err(AuthError::RateLimited { retry_after_secs });
    }
    if !status.is_success() {
        return Err(AuthError::Api {
            status: status.as_u16(),
            detail: None,
        });
    }
    let body = resp.text().await.map_err(network_err)?;
    if body.trim().is_empty() {
        return Ok(None);
    }
    parse_player_body(&body).map(Some)
}

/// Send a playback-control request (no response body expected). A 429
/// surfaces as `AuthError::RateLimited` with the parsed `Retry-After`.
async fn send_control(req: reqwest::RequestBuilder) -> Result<(), AuthError> {
    let resp = req
        // Spotify rejects bodyless PUT/POST without an explicit length.
        .header(reqwest::header::CONTENT_LENGTH, 0)
        .send()
        .await
        .map_err(network_err)?;
    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after_secs = parse_retry_after(
            resp.headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        return Err(AuthError::RateLimited { retry_after_secs });
    }
    if !status.is_success() {
        return Err(AuthError::Api {
            status: status.as_u16(),
            detail: None,
        });
    }
    Ok(())
}

/// Seek the active playback to `position_ms` (`PUT /v1/me/player/seek`).
pub async fn seek(
    http: &reqwest::Client,
    access_token: &str,
    position_ms: u64,
) -> Result<(), AuthError> {
    send_control(
        http.put(SEEK_URL)
            .query(&[("position_ms", position_ms.to_string())])
            .bearer_auth(access_token),
    )
    .await
}

/// Advance to the next track (`POST /v1/me/player/next`).
pub async fn next_track(http: &reqwest::Client, access_token: &str) -> Result<(), AuthError> {
    send_control(http.post(NEXT_URL).bearer_auth(access_token)).await
}

/// Fetch the connected user's profile.
pub async fn fetch_me(http: &reqwest::Client, access_token: &str) -> Result<MeResponse, AuthError> {
    let resp = http
        .get(ME_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(network_err)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AuthError::Api {
            status: status.as_u16(),
            detail: None,
        });
    }
    let body = resp.text().await.map_err(network_err)?;
    serde_json::from_str(&body).map_err(|_| AuthError::MalformedResponse)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_BODY: &str = r#"{
        "access_token": "NgCXRK...MzYjw",
        "token_type": "Bearer",
        "scope": "user-read-private user-read-playback-state",
        "expires_in": 3600,
        "refresh_token": "NgAagA...Um_SHo"
    }"#;

    #[test]
    fn parses_a_valid_token_response() {
        let t = parse_token_response(VALID_BODY).expect("valid body must parse");
        assert_eq!(t.access_token, "NgCXRK...MzYjw");
        assert_eq!(t.expires_in, 3600);
        assert_eq!(t.refresh_token.as_deref(), Some("NgAagA...Um_SHo"));
    }

    #[test]
    fn parses_a_refresh_response_without_rotated_refresh_token() {
        let body = r#"{
            "access_token": "abc",
            "token_type": "Bearer",
            "expires_in": 3600
        }"#;
        let t = parse_token_response(body).expect("refresh body must parse");
        assert!(t.refresh_token.is_none());
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_token_response("not json at all").expect_err("garbage must fail");
        assert!(matches!(err, AuthError::MalformedResponse));
    }

    #[test]
    fn rejects_json_missing_required_fields() {
        let err = parse_token_response(r#"{"token_type": "Bearer"}"#)
            .expect_err("missing access_token must fail");
        assert!(matches!(err, AuthError::MalformedResponse));
    }

    #[test]
    fn player_body_parses_a_full_track() {
        let body = r#"{
            "is_playing": true,
            "progress_ms": 12345,
            "currently_playing_type": "track",
            "item": {
                "id": "track1",
                "uri": "spotify:track:track1",
                "name": "Song",
                "duration_ms": 200000,
                "type": "track",
                "is_local": false,
                "artists": [{"name": "Artist A"}, {"name": "Artist B"}],
                "album": {"images": [{"url": "https://i.scdn.co/img/large"}, {"url": "https://i.scdn.co/img/small"}]}
            }
        }"#;
        let p = parse_player_body(body).expect("track body must parse");
        assert!(p.is_playing);
        assert_eq!(p.progress_ms, Some(12345));
        let item = p.item.expect("item present");
        assert_eq!(item.item_type, "track");
        assert_eq!(item.artists.len(), 2);
        assert_eq!(
            item.album.expect("album").images[0].url,
            "https://i.scdn.co/img/large"
        );
    }

    #[test]
    fn player_body_parses_an_ad_with_null_item() {
        let body = r#"{"is_playing": true, "progress_ms": 100, "currently_playing_type": "ad", "item": null}"#;
        let p = parse_player_body(body).expect("ad body must parse");
        assert!(p.item.is_none());
    }

    #[test]
    fn player_body_parses_a_local_file_without_id_or_album_art() {
        let body = r#"{
            "is_playing": false,
            "progress_ms": 0,
            "item": {
                "id": null,
                "uri": "spotify:local:::My+Song:100",
                "name": "My Song",
                "duration_ms": 100000,
                "type": "track",
                "is_local": true,
                "artists": [],
                "album": {"images": []}
            }
        }"#;
        let item = parse_player_body(body)
            .expect("local file must parse")
            .item
            .expect("item present");
        assert!(item.is_local);
        assert!(item.id.is_none());
        assert!(item.album.expect("album").images.is_empty());
    }

    #[test]
    fn player_body_parses_an_episode_with_direct_images() {
        let body = r#"{
            "is_playing": true,
            "progress_ms": 5000,
            "currently_playing_type": "episode",
            "item": {
                "id": "ep1",
                "uri": "spotify:episode:ep1",
                "name": "Episode 1",
                "duration_ms": 3600000,
                "type": "episode",
                "images": [{"url": "https://i.scdn.co/img/ep"}]
            }
        }"#;
        let item = parse_player_body(body)
            .expect("episode must parse")
            .item
            .expect("item present");
        assert_eq!(item.item_type, "episode");
        assert!(item.artists.is_empty());
        assert_eq!(item.images[0].url, "https://i.scdn.co/img/ep");
    }

    #[test]
    fn player_body_parses_the_active_device_and_tolerates_its_absence() {
        let body = r#"{
            "is_playing": true,
            "progress_ms": 1,
            "item": null,
            "device": {"id": "dev1", "name": "Desk speaker"}
        }"#;
        let p = parse_player_body(body).expect("device body must parse");
        assert_eq!(p.device.expect("device").id.as_deref(), Some("dev1"));

        let no_device = r#"{"is_playing": true, "progress_ms": 1, "item": null}"#;
        assert!(parse_player_body(no_device)
            .expect("must parse")
            .device
            .is_none());

        let null_id =
            r#"{"is_playing": true, "progress_ms": 1, "item": null, "device": {"id": null}}"#;
        assert!(parse_player_body(null_id)
            .expect("must parse")
            .device
            .expect("device")
            .id
            .is_none());
    }

    #[test]
    fn player_body_rejects_malformed_json() {
        let err = parse_player_body("<html>gateway error</html>").expect_err("must fail");
        assert!(matches!(err, AuthError::MalformedResponse));
    }

    #[test]
    fn retry_after_parses_whole_seconds_and_rejects_the_rest() {
        assert_eq!(parse_retry_after(Some("5")), Some(5));
        assert_eq!(parse_retry_after(Some(" 12 ")), Some(12));
        assert_eq!(parse_retry_after(Some("0")), Some(0));
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            None
        );
        assert_eq!(parse_retry_after(Some("-3")), None);
        assert_eq!(parse_retry_after(None), None);
    }

    #[test]
    fn queue_body_parses_tracks_and_tolerates_sparse_items() {
        let body = r#"{
            "currently_playing": {"uri": "spotify:track:a"},
            "queue": [
                {"uri": "spotify:track:b", "name": "Next", "type": "track",
                 "duration_ms": 180000, "is_local": false},
                {"uri": "spotify:track:c", "type": "track"}
            ]
        }"#;
        let q = parse_queue_body(body).expect("queue body must parse");
        assert_eq!(q.queue.len(), 2);
        assert_eq!(q.queue[0].uri.as_deref(), Some("spotify:track:b"));
        assert_eq!(q.queue[0].item_type.as_deref(), Some("track"));
        assert_eq!(q.queue[0].name.as_deref(), Some("Next"));
        assert!(!q.queue[1].is_local);
    }

    #[test]
    fn queue_body_tolerates_an_empty_or_missing_queue() {
        assert!(parse_queue_body(r#"{"queue": []}"#)
            .expect("empty queue must parse")
            .queue
            .is_empty());
        assert!(parse_queue_body(r#"{"currently_playing": null}"#)
            .expect("missing queue must parse")
            .queue
            .is_empty());
    }

    #[test]
    fn queue_body_parses_episodes_and_local_files() {
        let body = r#"{
            "queue": [
                {"uri": "spotify:episode:e1", "name": "Ep", "type": "episode"},
                {"uri": "spotify:local:::x:1", "type": "track", "is_local": true}
            ]
        }"#;
        let q = parse_queue_body(body).expect("must parse");
        assert_eq!(q.queue[0].item_type.as_deref(), Some("episode"));
        assert!(q.queue[1].is_local);
    }

    #[test]
    fn queue_body_rejects_malformed_json() {
        let err = parse_queue_body("<html>bad gateway</html>").expect_err("must fail");
        assert!(matches!(err, AuthError::MalformedResponse));
    }

    #[test]
    fn me_response_parses_with_null_display_name() {
        let me: MeResponse =
            serde_json::from_str(r#"{"id": "user1", "display_name": null, "product": "premium"}"#)
                .expect("me body must parse");
        assert_eq!(me.id, "user1");
        assert!(me.display_name.is_none());
        assert_eq!(me.product.as_deref(), Some("premium"));
    }
}
