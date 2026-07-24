use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::config::SuggestionToggles;
use crate::error::AuthError;
use crate::player::PlayerEngine;
use crate::presets::{
    Preset, PresetDb, PresetDbHealth, PresetError, PresetInput, StoredSuggestion,
};
use crate::spotify::{MeResponse, TokenResponse};
use crate::suggestions::{self, Status, SuggestionType};
use crate::token_store::Tokens;
use crate::{config, heatmap, pkce, player, server, spotify, token_store};

/// Refresh this many seconds before the reported expiry to absorb clock skew
/// and request latency.
const EXPIRY_MARGIN_SECS: u64 = 30;

/// Shared backend state: HTTP client, in-memory token mirror, login guard,
/// playback poll engine.
pub struct AppState {
    pub(crate) http: reqwest::Client,
    pub(crate) tokens: tauri::async_runtime::Mutex<Option<Tokens>>,
    login_in_progress: AtomicBool,
    pub(crate) player: PlayerEngine,
    /// Master toggle of the auto-seek/skip engine (mirrors config.json).
    pub(crate) automation_on: AtomicBool,
    /// Master toggle of listening-insights collection (mirrors config.json).
    /// The poll loop reads this each tick; while off, zero events are written.
    pub(crate) insights_on: AtomicBool,
    /// Whether the connected account is Premium — control calls are gated
    /// on this. Updated on every profile fetch, cleared on logout.
    pub(crate) premium: AtomicBool,
    /// Bumped on every preset save/delete so the poll loop invalidates its
    /// per-track preset cache.
    pub(crate) presets_version: AtomicU64,
    /// Bumped whenever an applied auto-skip flag changes (M10) so the poll
    /// loop invalidates its per-track auto-skip cache.
    pub(crate) suggestions_version: AtomicU64,
    /// Guards the opportunistic analysis pass against overlapping runs (it is
    /// triggered from several UI events, debounced but not serialized).
    analysis_running: AtomicBool,
    /// Whether the main window is currently shown (tray mode hides it).
    /// While hidden, the suspended poll loop slow-polls instead of parking.
    pub(crate) window_visible: AtomicBool,
    /// Track URI whose preset is being edited in the UI (M8 edit mode).
    /// While set, the poll loop gates automation for exactly this track.
    pub(crate) edit_hold: std::sync::Mutex<Option<String>>,
    /// Set by UI-initiated seeks (edit-mode preview / exit-restore) so the
    /// poll loop absorbs the next observed jump instead of classifying it
    /// as a manual seek (which could suppress the start jump).
    pub(crate) ui_seek_pending: AtomicBool,
}

impl AppState {
    pub fn new() -> Result<Self, AuthError> {
        Ok(Self {
            http: spotify::build_http_client()?,
            tokens: tauri::async_runtime::Mutex::new(None),
            login_in_progress: AtomicBool::new(false),
            player: PlayerEngine::default(),
            automation_on: AtomicBool::new(true),
            insights_on: AtomicBool::new(true),
            premium: AtomicBool::new(false),
            presets_version: AtomicU64::new(0),
            suggestions_version: AtomicU64::new(0),
            analysis_running: AtomicBool::new(false),
            window_visible: AtomicBool::new(true),
            edit_hold: std::sync::Mutex::new(None),
            ui_seek_pending: AtomicBool::new(false),
        })
    }
}

/// Connected-user info shown on the landing screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub display_name: String,
    pub is_premium: bool,
}

/// Result of a session-restore attempt on startup.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub connected: bool,
    pub profile: Option<Profile>,
}

fn now_unix() -> Result<u64, AuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| AuthError::Config("system clock is set before 1970".into()))
}

/// Turn a token-endpoint response into a storable token set. Refresh-grant
/// responses may omit the refresh token — then the previous one stays valid.
pub(crate) fn tokens_from_response(
    resp: TokenResponse,
    previous_refresh: Option<String>,
) -> Result<Tokens, AuthError> {
    let refresh_token = resp
        .refresh_token
        .or(previous_refresh)
        .ok_or(AuthError::MalformedResponse)?;
    Ok(Tokens {
        access_token: resp.access_token,
        expires_at: now_unix()? + resp.expires_in.saturating_sub(EXPIRY_MARGIN_SECS),
        refresh_token,
    })
}

fn profile_from_me(me: MeResponse) -> Profile {
    Profile {
        display_name: me.display_name.unwrap_or(me.id),
        is_premium: me.product.as_deref() == Some("premium"),
    }
}

/// Refresh the access token if expired, persisting any rotated refresh token.
pub(crate) async fn ensure_fresh(
    http: &reqwest::Client,
    client_id: &str,
    tokens: Tokens,
) -> Result<Tokens, AuthError> {
    if now_unix()? < tokens.expires_at {
        return Ok(tokens);
    }
    let resp = spotify::refresh_access_token(http, client_id, &tokens.refresh_token).await?;
    let fresh = tokens_from_response(resp, Some(tokens.refresh_token))?;
    token_store::save(&fresh)?;
    Ok(fresh)
}

/// Read the stored Client ID (None when not configured yet).
#[tauri::command]
pub async fn get_client_id(app: AppHandle) -> Result<Option<String>, AuthError> {
    config::load_client_id(&app)
}

/// Validate and persist a user-supplied Client ID.
#[tauri::command]
pub async fn save_client_id(app: AppHandle, client_id: String) -> Result<(), AuthError> {
    config::save_client_id(&app, client_id.trim())
}

/// Where wizard step 1 sends the user to create their Spotify app. Fixed
/// URL on purpose: the frontend cannot open arbitrary links through this.
const SPOTIFY_DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

/// Open the Spotify Developer Dashboard in the system browser (wizard step 1).
#[tauri::command]
pub async fn open_spotify_dashboard(app: AppHandle) -> Result<(), AuthError> {
    app.opener()
        .open_url(SPOTIFY_DASHBOARD_URL, None::<&str>)
        .map_err(|e| AuthError::Config(format!("cannot open the system browser: {e}")))
}

/// Where the Settings support row sends the user. Fixed URL on purpose:
/// the frontend cannot open arbitrary links through this. Cued is free —
/// this is a voluntary tip jar, never a feature gate.
const SUPPORT_URL: &str = "https://ko-fi.com/phinorsk";

/// Open the Ko-fi support page in the system browser (Settings footer).
#[tauri::command]
pub async fn open_support_page(app: AppHandle) -> Result<(), AuthError> {
    app.opener()
        .open_url(SUPPORT_URL, None::<&str>)
        .map_err(|e| AuthError::Config(format!("cannot open the system browser: {e}")))
}

/// Run the full PKCE login: system browser → loopback callback → state check →
/// code exchange → keychain → profile fetch. Fails fast if port 8917 is busy.
#[tauri::command]
pub async fn connect_spotify(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Profile, AuthError> {
    if state.login_in_progress.swap(true, Ordering::SeqCst) {
        return Err(AuthError::LoginInProgress);
    }
    let result = run_login(&app, &state).await;
    state.login_in_progress.store(false, Ordering::SeqCst);
    result
}

async fn run_login(app: &AppHandle, state: &State<'_, AppState>) -> Result<Profile, AuthError> {
    let client_id = config::load_client_id(app)?.ok_or(AuthError::NoClientId)?;

    let verifier = pkce::generate_verifier()?;
    let challenge = pkce::challenge_s256(&verifier);
    let oauth_state = pkce::generate_state()?;

    // Bind before opening the browser so a busy port surfaces immediately.
    let listener = server::bind_listener()?;
    let authorize_url = spotify::build_authorize_url(&client_id, &challenge, &oauth_state)?;
    app.opener()
        .open_url(authorize_url, None::<&str>)
        .map_err(|e| AuthError::Config(format!("cannot open the system browser: {e}")))?;

    let params = tauri::async_runtime::spawn_blocking(move || server::wait_for_callback(listener))
        .await
        .map_err(|e| AuthError::Config(format!("callback listener task failed: {e}")))??;

    // State first: a response that isn't tied to our request is discarded
    // entirely — its code is never exchanged, its error never trusted.
    pkce::validate_state(&oauth_state, params.state.as_deref().unwrap_or(""))?;
    if let Some(error) = params.error {
        return Err(if error == "access_denied" {
            AuthError::AccessDenied
        } else {
            AuthError::SpotifyAuth(error)
        });
    }
    let code = params.code.ok_or(AuthError::BadCallback)?;

    let resp = spotify::exchange_code(&state.http, &client_id, &code, &verifier).await?;
    let tokens = tokens_from_response(resp, None)?;
    token_store::save(&tokens)?;

    let me = spotify::fetch_me(&state.http, &tokens.access_token).await?;
    *state.tokens.lock().await = Some(tokens);
    let profile = profile_from_me(me);
    state.premium.store(profile.is_premium, Ordering::SeqCst);
    player::start(app);
    Ok(profile)
}

/// Restore the session from the keychain on startup: refresh if expired,
/// fetch the profile. Unrecoverable auth (revoked/invalid grant) degrades to
/// the disconnected state instead of erroring.
#[tauri::command]
pub async fn get_auth_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AuthStatus, AuthError> {
    const DISCONNECTED: AuthStatus = AuthStatus {
        connected: false,
        profile: None,
    };

    let mut guard = state.tokens.lock().await;
    if guard.is_none() {
        *guard = token_store::load()?;
    }
    let Some(tokens) = guard.clone() else {
        return Ok(DISCONNECTED);
    };
    let Some(client_id) = config::load_client_id(&app)? else {
        // Tokens without a Client ID cannot be refreshed — treat as logged out.
        token_store::clear()?;
        *guard = None;
        return Ok(DISCONNECTED);
    };

    let tokens = match ensure_fresh(&state.http, &client_id, tokens).await {
        Ok(t) => t,
        // 4xx from the token endpoint means the grant is gone (revoked or
        // expired) — degrade to re-auth, never loop.
        Err(AuthError::Api { status, .. }) if (400..500).contains(&status) => {
            token_store::clear()?;
            *guard = None;
            return Ok(DISCONNECTED);
        }
        Err(e) => return Err(e),
    };
    *guard = Some(tokens.clone());

    let me = match spotify::fetch_me(&state.http, &tokens.access_token).await {
        // 401 despite a fresh-looking token: refresh once, retry once, then give up.
        Err(AuthError::Api { status: 401, .. }) => {
            let resp =
                spotify::refresh_access_token(&state.http, &client_id, &tokens.refresh_token)
                    .await?;
            let fresh = tokens_from_response(resp, Some(tokens.refresh_token))?;
            token_store::save(&fresh)?;
            let me = spotify::fetch_me(&state.http, &fresh.access_token).await?;
            *guard = Some(fresh);
            me
        }
        other => other?,
    };

    drop(guard);
    let profile = profile_from_me(me);
    state.premium.store(profile.is_premium, Ordering::SeqCst);
    player::start(&app);
    Ok(AuthStatus {
        connected: true,
        profile: Some(profile),
    })
}

/// Delete both keychain entries and the in-memory copy, and stop the
/// playback poll loop so no request is issued after logout.
#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), AuthError> {
    state.player.stop();
    state.premium.store(false, Ordering::SeqCst);
    *state.tokens.lock().await = None;
    token_store::clear()
}

/// UI wake signal (mount / window focus): nudge a suspended or sleeping poll
/// loop to poll right away — and restart it if none is running (self-heal
/// after rare stop/start races). No-op while a loop is already live.
#[tauri::command]
pub async fn player_wake(app: AppHandle, state: State<'_, AppState>) -> Result<(), AuthError> {
    state.player.wake();
    player::start(&app);
    Ok(())
}

/// Tell the poll loop that presets changed: invalidate its per-track cache
/// and nudge it so a preset saved for the PLAYING track applies right away.
fn notify_presets_changed(state: &State<'_, AppState>) {
    state.presets_version.fetch_add(1, Ordering::SeqCst);
    state.player.wake();
}

/// Validate and upsert a preset (transactional); returns the stored row.
#[tauri::command]
pub async fn save_preset(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
    preset: PresetInput,
) -> Result<Preset, PresetError> {
    let saved = db.store()?.save(&preset)?;
    notify_presets_changed(&state);
    Ok(saved)
}

/// Read the preset for one track URI (null when none exists).
#[tauri::command]
pub async fn get_preset(
    db: State<'_, PresetDb>,
    track_uri: String,
) -> Result<Option<Preset>, PresetError> {
    db.store()?.get(&track_uri)
}

/// All stored presets, newest first.
#[tauri::command]
pub async fn list_presets(db: State<'_, PresetDb>) -> Result<Vec<Preset>, PresetError> {
    db.store()?.list()
}

/// Delete the preset for one track URI (idempotent, transactional).
#[tauri::command]
pub async fn delete_preset(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
    track_uri: String,
) -> Result<(), PresetError> {
    db.store()?.delete(&track_uri)?;
    notify_presets_changed(&state);
    Ok(())
}

/// Whether the auto-seek/skip engine is enabled (persisted in config.json).
#[tauri::command]
pub async fn get_automation_enabled(state: State<'_, AppState>) -> Result<bool, AuthError> {
    Ok(state.automation_on.load(Ordering::SeqCst))
}

/// Event name master-toggle changes are pushed to the UI under, so the
/// in-app switch follows toggles made from the tray menu.
pub const AUTOMATION_EVENT: &str = "automation://enabled";

/// Persist + apply the master toggle and fan the new value out to every
/// mirror (engine gate, tray checkbox, UI event). Single entry point for
/// both the IPC command and the tray menu item.
pub(crate) fn apply_automation_enabled(app: &AppHandle, enabled: bool) -> Result<(), AuthError> {
    config::save_automation_enabled(app, enabled)?;
    let state = app.state::<AppState>();
    state.automation_on.store(enabled, Ordering::SeqCst);
    state.player.wake();
    crate::tray::sync_automation(app, enabled);
    if let Err(e) = app.emit(AUTOMATION_EVENT, enabled) {
        eprintln!("cued: failed to emit the automation toggle: {e}");
    }
    Ok(())
}

/// Persist and apply the automation master toggle.
#[tauri::command]
pub async fn set_automation_enabled(app: AppHandle, enabled: bool) -> Result<(), AuthError> {
    apply_automation_enabled(&app, enabled)
}

/// Enter or leave preset edit mode (M8): while a track URI is set, the poll
/// loop suspends automation for exactly that track (engine-level gate; the
/// master toggle stays untouched). `None` leaves edit mode. The wake makes
/// the gate apply before any already-scheduled boundary one-shot can fire.
#[tauri::command]
pub async fn set_edit_mode(
    state: State<'_, AppState>,
    track_uri: Option<String>,
) -> Result<(), AuthError> {
    if track_uri
        .as_ref()
        .is_some_and(|u| u.is_empty() || u.len() > crate::presets::MAX_URI_LEN)
    {
        return Err(AuthError::Config("invalid track URI".into()));
    }
    match state.edit_hold.lock() {
        Ok(mut hold) => *hold = track_uri,
        Err(_) => {
            return Err(AuthError::Config(
                "edit-mode state is unavailable — please restart Cued".into(),
            ))
        }
    }
    state.player.wake();
    Ok(())
}

/// UI-initiated seek (M8 edit-mode preview / exit-restore) through the
/// existing seek path. Flags the poll loop so the observed jump is absorbed
/// rather than classified as a manual seek; never counts as an automation
/// action (no cooldown, no action cap, no retry bookkeeping).
#[tauri::command]
pub async fn ui_seek(state: State<'_, AppState>, position_ms: u64) -> Result<(), AuthError> {
    if position_ms > crate::presets::MAX_DURATION_MS {
        return Err(AuthError::Config(
            "seek position is implausibly large".into(),
        ));
    }
    let token = state
        .tokens
        .lock()
        .await
        .as_ref()
        .map(|t| t.access_token.clone());
    let Some(token) = token else {
        return Err(AuthError::Config("not connected".into()));
    };
    spotify::seek(&state.http, &token, position_ms).await?;
    state.ui_seek_pending.store(true, Ordering::SeqCst);
    // Re-poll right away so the UI playhead reflects the seek promptly.
    state.player.wake();
    Ok(())
}

/// Startup health of the preset database (corrupt-file rescue / open failure)
/// so the UI can show a clear notice.
#[tauri::command]
pub async fn get_preset_db_health(db: State<'_, PresetDb>) -> Result<PresetDbHealth, PresetError> {
    Ok(db.health())
}

/// Whether listening-insights collection is enabled (persisted in config.json).
#[tauri::command]
pub async fn get_insights_enabled(state: State<'_, AppState>) -> Result<bool, AuthError> {
    Ok(state.insights_on.load(Ordering::SeqCst))
}

/// Persist and apply the listening-insights master toggle. While off, the
/// poll loop writes zero events (the gate is in the engine, not just the UI).
#[tauri::command]
pub async fn set_insights_enabled(app: AppHandle, enabled: bool) -> Result<(), AuthError> {
    config::save_insights_enabled(&app, enabled)?;
    app.state::<AppState>()
        .insights_on
        .store(enabled, Ordering::SeqCst);
    Ok(())
}

/// How many listening-insights events have been collected (settings surface).
#[tauri::command]
pub async fn get_insights_count(db: State<'_, PresetDb>) -> Result<u64, PresetError> {
    db.store()?.insights_count()
}

/// Delete ALL collected insights (both tables, one transaction). Presets are
/// untouched. This is the only path that ever erases insights data.
#[tauri::command]
pub async fn delete_all_insights(
    state: State<'_, AppState>,
    db: State<'_, PresetDb>,
) -> Result<(), PresetError> {
    db.store()?.delete_all_insights()?;
    // Derived suggestions (incl. applied auto-skips) were wiped too — let the
    // poll loop drop its cached auto-skip flags.
    notify_suggestions_changed(&state);
    Ok(())
}

// ---------------------------------------------------------------------------
// M11: skip-density heatmap
// ---------------------------------------------------------------------------

/// A track's normalized skip-density curve for the timeline overlay (M11).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapDto {
    /// [`heatmap::HEATMAP_BUCKETS`] peak-normalized values in `[0, 1]`
    /// (track start → end).
    pub buckets: Vec<f64>,
    /// Eligible skip-away events behind the curve — the honest evidence count.
    pub event_count: u32,
}

/// Compute the skip-density curve for one track, or `null` when there is too
/// little data. Reads the SAME recorded events the suggestion engine uses and
/// applies the SAME 15 s rejection exclusion; the pure bucketing lives in
/// `heatmap.rs`. Off the polling hot path — the UI calls it once per track
/// change and caches the result.
#[tauri::command]
pub async fn get_track_heatmap(
    db: State<'_, PresetDb>,
    track_uri: String,
) -> Result<Option<HeatmapDto>, PresetError> {
    let store = db.store()?;
    let Some(duration_ms) = store.track_duration_ms(&track_uri)? else {
        return Ok(None);
    };
    let events = store.events_for_track(&track_uri)?;
    Ok(heatmap::compute(&events, duration_ms).map(|hm| HeatmapDto {
        buckets: hm.buckets,
        event_count: hm.event_count,
    }))
}

// ---------------------------------------------------------------------------
// M10: suggestions
// ---------------------------------------------------------------------------

/// Bump the suggestions cache epoch and nudge the poll loop so an applied /
/// reverted auto-skip flag takes effect on the next tick.
fn notify_suggestions_changed(state: &State<'_, AppState>) {
    state.suggestions_version.fetch_add(1, Ordering::SeqCst);
    state.player.wake();
}

fn parse_suggestion_type(s: &str) -> Result<SuggestionType, PresetError> {
    SuggestionType::from_str(s)
        .ok_or_else(|| PresetError::Validation(format!("unknown suggestion type: {s}")))
}

/// Run the opportunistic analysis pass (M10): for every track with events
/// newer than the last run, recompute its suggestions from the full history
/// and reconcile the stored rows. Bounded to changed tracks; guarded against
/// overlapping runs; never touches the polling hot path.
#[tauri::command]
pub async fn analyze_suggestions(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
) -> Result<(), PresetError> {
    if state.analysis_running.swap(true, Ordering::SeqCst) {
        return Ok(()); // a run is already in flight — this trigger coalesces
    }
    let out = run_analysis(&db);
    state.analysis_running.store(false, Ordering::SeqCst);
    out
}

fn run_analysis(db: &State<'_, PresetDb>) -> Result<(), PresetError> {
    let store = db.store()?;
    let cursor = store.analysis_cursor()?;
    let (uris, max_id) = store.tracks_with_new_events(cursor)?;
    let now = suggestions::now_ms();
    for uri in &uris {
        let events = store.events_for_track(uri)?;
        let Some(duration_ms) = store.track_duration_ms(uri)? else {
            continue;
        };
        let preset = store.get(uri)?.map(|p| (p.start_ms, p.skip_ms));
        let computed = suggestions::analyze(&events, preset, duration_ms, now);
        store.refresh_track_suggestions(uri, &computed, now)?;
    }
    store.set_analysis_cursor(max_id)?;
    Ok(())
}

/// Non-dismissed suggestions for one track (the Now Playing card picks the
/// strongest active one to surface).
#[tauri::command]
pub async fn get_track_suggestions(
    db: State<'_, PresetDb>,
    track_uri: String,
) -> Result<Vec<StoredSuggestion>, PresetError> {
    db.store()?.suggestions_for_track(&track_uri)
}

/// Every non-dismissed suggestion (the Library "Suggestions (n)" section).
#[tauri::command]
pub async fn list_suggestions(
    db: State<'_, PresetDb>,
) -> Result<Vec<StoredSuggestion>, PresetError> {
    db.store()?.list_suggestions()
}

/// What accepting a skip/start suggestion did, so the UI can offer a full undo.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptResult {
    /// The preset saved (null for auto-skip, which saves no preset).
    pub saved: Option<Preset>,
    /// The preset that existed before (null when there was none) — the exact
    /// state Undo restores.
    pub previous: Option<Preset>,
}

/// Accept a suggestion. Skip/start suggestions apply INSTANTLY as a normal
/// preset (start-point sets the start; skip-point sets the skip, preserving
/// any existing start) and the row flips to `applied`. Auto-skip only flips
/// the row to `applied` (the engine reads it as a whole-song skip). The
/// previous preset is returned so Undo can revert fully.
#[tauri::command]
pub async fn accept_suggestion(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
    track_uri: String,
    suggestion_type: String,
) -> Result<AcceptResult, PresetError> {
    let kind = parse_suggestion_type(&suggestion_type)?;
    let store = db.store()?;
    let now = suggestions::now_ms();

    if kind == SuggestionType::AutoSkip {
        store.set_suggestion_status(&track_uri, kind, Status::Applied, now)?;
        notify_suggestions_changed(&state);
        return Ok(AcceptResult {
            saved: None,
            previous: None,
        });
    }

    let meta = store
        .suggestions_for_track(&track_uri)?
        .into_iter()
        .find(|s| s.kind == kind.as_str())
        .ok_or_else(|| PresetError::Validation("this suggestion is no longer available".into()))?;
    let value = meta
        .value_start_ms
        .ok_or_else(|| PresetError::Validation("this suggestion has no value to apply".into()))?;
    let previous = store.get(&track_uri)?;
    let (start_ms, skip_ms) = match kind {
        SuggestionType::SkipPoint => (previous.as_ref().map_or(0, |p| p.start_ms), value),
        SuggestionType::StartPoint => (
            value,
            previous.as_ref().map_or(meta.duration_ms, |p| p.skip_ms),
        ),
        SuggestionType::AutoSkip => unreachable!("auto-skip handled above"),
    };
    let saved = store.save(&PresetInput {
        track_uri: track_uri.clone(),
        title: meta.title,
        artists: meta.artists,
        cover_url: meta.cover_url,
        duration_ms: meta.duration_ms,
        start_ms,
        skip_ms,
    })?;
    store.set_suggestion_status(&track_uri, kind, Status::Applied, now)?;
    notify_presets_changed(&state);
    notify_suggestions_changed(&state);
    Ok(AcceptResult {
        saved: Some(saved),
        previous,
    })
}

/// Undo an accepted suggestion: restore the preset to `previous` (or delete
/// the one just created when there was none) and return the row to `active`.
#[tauri::command]
pub async fn undo_suggestion(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
    track_uri: String,
    suggestion_type: String,
    previous: Option<PresetInput>,
) -> Result<(), PresetError> {
    let kind = parse_suggestion_type(&suggestion_type)?;
    let store = db.store()?;
    let now = suggestions::now_ms();
    if kind != SuggestionType::AutoSkip {
        match previous {
            Some(input) => {
                store.save(&input)?;
            }
            None => store.delete(&track_uri)?,
        }
        notify_presets_changed(&state);
    }
    store.set_suggestion_status(&track_uri, kind, Status::Active, now)?;
    notify_suggestions_changed(&state);
    Ok(())
}

/// "No thanks" / ×: never surface this suggestion type for this track again.
#[tauri::command]
pub async fn dismiss_suggestion(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
    track_uri: String,
    suggestion_type: String,
) -> Result<(), PresetError> {
    let kind = parse_suggestion_type(&suggestion_type)?;
    let now = suggestions::now_ms();
    db.store()?
        .set_suggestion_status(&track_uri, kind, Status::Dismissed, now)?;
    notify_suggestions_changed(&state);
    Ok(())
}

/// Record that a shown proactive card was ignored (the track moved on without
/// an accept/dismiss): bumps the counter and retires it at the cap.
#[tauri::command]
pub async fn ignore_suggestion(
    db: State<'_, PresetDb>,
    track_uri: String,
    suggestion_type: String,
) -> Result<(), PresetError> {
    let kind = parse_suggestion_type(&suggestion_type)?;
    db.store()?
        .ignore_suggestion(&track_uri, kind, suggestions::now_ms())
}

/// Reverse an applied auto-skip from the Library (applied ↔ active). Music
/// must never disappear silently — this is the visible off switch.
#[tauri::command]
pub async fn set_auto_skip_applied(
    db: State<'_, PresetDb>,
    state: State<'_, AppState>,
    track_uri: String,
    applied: bool,
) -> Result<(), PresetError> {
    let status = if applied {
        Status::Applied
    } else {
        Status::Active
    };
    db.store()?.set_suggestion_status(
        &track_uri,
        SuggestionType::AutoSkip,
        status,
        suggestions::now_ms(),
    )?;
    notify_suggestions_changed(&state);
    Ok(())
}

/// The per-type suggestion toggles (persisted in config.json).
#[tauri::command]
pub async fn get_suggestion_toggles(app: AppHandle) -> Result<SuggestionToggles, AuthError> {
    config::load_suggestion_toggles(&app)
}

/// Persist the per-type suggestion toggles.
#[tauri::command]
pub async fn set_suggestion_toggles(
    app: AppHandle,
    toggles: SuggestionToggles,
) -> Result<(), AuthError> {
    config::save_suggestion_toggles(&app, toggles)
}
