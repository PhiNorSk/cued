//! Read-only playback polling engine (M2).
//!
//! A single self-scheduling loop polls `GET /v1/me/player` and pushes
//! [`PlaybackState`] snapshots to the UI as the `playback://state` event —
//! only when something meaningful changed, plus a low-frequency heartbeat.
//! The UI never polls Rust; it interpolates between events.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::automation::{self, Automation, Gates};
use crate::commands::AppState;
use crate::error::AuthError;
use crate::spotify::{self, PlayerItem, PlayerResponse};
use crate::{config, token_store};

// ---------------------------------------------------------------------------
// All timing bounds of the engine live here (single source of truth).
// ---------------------------------------------------------------------------

/// Poll cadence while a track is actively playing.
pub const POLL_PLAYING: Duration = Duration::from_millis(1000);
/// Poll cadence while playback is paused.
pub const POLL_PAUSED: Duration = Duration::from_millis(2000);
/// Poll cadence while no device is active (until suspension kicks in).
pub const POLL_IDLE: Duration = Duration::from_millis(2000);
/// Suspend polling after this long without an active device.
pub const IDLE_SUSPEND_AFTER: Duration = Duration::from_secs(60);
/// First retry delay after a transient error (doubles per consecutive failure).
pub const BACKOFF_BASE: Duration = Duration::from_secs(2);
/// Upper bound for the error backoff, jitter included.
pub const BACKOFF_CAP: Duration = Duration::from_secs(30);
/// Wait after a 429 whose `Retry-After` header is missing or unparseable.
pub const RATE_LIMIT_FALLBACK: Duration = Duration::from_secs(5);
/// Max silence between emitted events while polling (drift-correction heartbeat).
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
/// A position deviating more than this from extrapolation counts as a seek.
pub const SEEK_JUMP_MS: u64 = 2000;
/// Slow-poll cadence while the window is HIDDEN and polling is suspended
/// (no active device): with no UI there is no focus/mount wake, so instead
/// of parking forever the engine re-checks this often. Deliberately the
/// slowest cadence of all — hidden must never poll faster than visible.
pub const POLL_HIDDEN_SUSPENDED: Duration = Duration::from_secs(30);

/// The transition→seek latency probe (a log-only measurement) is dropped
/// after this long without resolving — no burst + cooldown chain lasts this.
pub const BURST_PROBE_EXPIRY: Duration = Duration::from_secs(10);

/// Tauri event name the engine emits [`PlaybackState`] under.
pub const STATE_EVENT: &str = "playback://state";

// ---------------------------------------------------------------------------
// State pushed to the UI
// ---------------------------------------------------------------------------

/// Coarse playback status; serialized camelCase for the TS side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    /// Nothing displayable: no active device, or an ad is playing.
    Idle,
    /// Polling suspended after a long idle period (resumes on wake).
    Suspended,
    /// Token refresh failed for good — the engine stopped, UI must re-auth.
    AuthLost,
}

/// What kind of item is loaded (episodes get display-only support).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    Track,
    Episode,
}

/// Displayable info about the current track or episode.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    /// Stable identity for change detection (Spotify URI; local files too).
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
    pub is_local: bool,
    pub kind: TrackKind,
}

/// Why the auto-skip engine cannot act right now; the UI shows this quietly
/// next to the master toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutomationSuspension {
    /// Control calls require a Premium account.
    NoPremium,
    /// The active device rejected a control call (403); cleared when the
    /// active device changes.
    RestrictedDevice,
    /// A rate limit is being honored; clears when polling resumes.
    RateLimited,
}

/// One playback snapshot as pushed to the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub status: PlaybackStatus,
    /// True while the engine is backing off after transient errors.
    pub reconnecting: bool,
    pub track: Option<TrackInfo>,
    pub position_ms: Option<u64>,
    /// Unix ms when `position_ms` was sampled — the UI interpolates from here.
    pub fetched_at_ms: u64,
    /// Why automation cannot act right now (None = it can).
    pub automation_suspended: Option<AutomationSuspension>,
}

impl PlaybackState {
    fn bare(status: PlaybackStatus, fetched_at_ms: u64) -> Self {
        Self {
            status,
            reconnecting: false,
            track: None,
            position_ms: None,
            fetched_at_ms,
            automation_suspended: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pure scheduling / emission logic (unit-tested)
// ---------------------------------------------------------------------------

/// What one poll observed, as far as scheduling is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    /// A device is active (covers tracks, episodes and ads).
    Active { playing: bool },
    /// HTTP 204 / empty body: nothing to observe.
    NoDevice,
}

/// The engine's next scheduling step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextPoll {
    Delay(Duration),
    Suspend,
}

/// Pick the next poll delay from what the last poll saw and how long no
/// device has been active.
pub fn plan_next_delay(outcome: PollOutcome, idle_for: Duration) -> NextPoll {
    match outcome {
        PollOutcome::Active { playing: true } => NextPoll::Delay(POLL_PLAYING),
        PollOutcome::Active { playing: false } => NextPoll::Delay(POLL_PAUSED),
        PollOutcome::NoDevice if idle_for >= IDLE_SUSPEND_AFTER => NextPoll::Suspend,
        PollOutcome::NoDevice => NextPoll::Delay(POLL_IDLE),
    }
}

/// How long to wait in the suspended state: `None` parks until an external
/// wake (window visible — the UI nudges on focus/mount), `Some(delay)`
/// slow-polls so resumed playback is detected without any UI (window hidden).
pub fn plan_suspended_wait(window_visible: bool) -> Option<Duration> {
    if window_visible {
        None
    } else {
        Some(POLL_HIDDEN_SUSPENDED)
    }
}

/// Capped exponential backoff with jitter for transient errors.
///
/// `jitter_unit` must be in `[0, 1]`; the result lands in
/// `[base/2, base]` where `base = min(BACKOFF_CAP, BACKOFF_BASE * 2^(n-1))`,
/// so it never exceeds [`BACKOFF_CAP`].
pub fn backoff_delay(consecutive_failures: u32, jitter_unit: f64) -> Duration {
    // Shift capped at 16: 2 s << 16 already exceeds BACKOFF_CAP by far, and a
    // larger shift would overflow the u32 multiplier.
    let exp = consecutive_failures.max(1).saturating_sub(1).min(16);
    let base = BACKOFF_BASE.saturating_mul(1u32 << exp).min(BACKOFF_CAP);
    let jitter = if jitter_unit.is_finite() {
        jitter_unit.clamp(0.0, 1.0)
    } else {
        0.5
    };
    base.mul_f64(0.5 + 0.5 * jitter)
}

/// Delay after a 429: exactly `Retry-After` when given, else the fallback.
pub fn rate_limit_delay(retry_after_secs: Option<u64>) -> Duration {
    retry_after_secs
        .map(Duration::from_secs)
        .unwrap_or(RATE_LIMIT_FALLBACK)
}

/// Decide whether a fresh snapshot is worth an event: any meaningful change
/// (status, reconnecting flag, track identity, seek jump) or the heartbeat.
pub fn should_emit(
    prev: Option<&PlaybackState>,
    next: &PlaybackState,
    since_last_emit: Duration,
) -> bool {
    let Some(prev) = prev else {
        return true;
    };
    if prev.status != next.status || prev.reconnecting != next.reconnecting {
        return true;
    }
    if prev.automation_suspended != next.automation_suspended {
        return true;
    }
    let track_uri = |s: &PlaybackState| s.track.as_ref().map(|t| t.uri.clone());
    if track_uri(prev) != track_uri(next) {
        return true;
    }
    if seek_jumped(prev, next) {
        return true;
    }
    since_last_emit >= HEARTBEAT_EVERY
}

/// True when the observed position deviates from linear extrapolation of the
/// previous snapshot by more than [`SEEK_JUMP_MS`] — the user seeked.
fn seek_jumped(prev: &PlaybackState, next: &PlaybackState) -> bool {
    let (Some(prev_pos), Some(next_pos)) = (prev.position_ms, next.position_ms) else {
        return false;
    };
    let expected = if prev.status == PlaybackStatus::Playing {
        prev_pos.saturating_add(next.fetched_at_ms.saturating_sub(prev.fetched_at_ms))
    } else {
        prev_pos
    };
    next_pos.abs_diff(expected) > SEEK_JUMP_MS
}

/// Map an API response (or its absence) to a UI snapshot. Ads and unknown
/// item types collapse to `Idle` — nothing to display, but not an error.
pub fn snapshot_from_response(resp: Option<&PlayerResponse>, fetched_at_ms: u64) -> PlaybackState {
    let Some(resp) = resp else {
        return PlaybackState::bare(PlaybackStatus::Idle, fetched_at_ms);
    };
    let Some(track) = resp.item.as_ref().and_then(track_info) else {
        return PlaybackState::bare(PlaybackStatus::Idle, fetched_at_ms);
    };
    PlaybackState {
        status: if resp.is_playing {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Paused
        },
        reconnecting: false,
        track: Some(track),
        position_ms: resp.progress_ms,
        fetched_at_ms,
        automation_suspended: None,
    }
}

fn track_info(item: &PlayerItem) -> Option<TrackInfo> {
    let kind = match item.item_type.as_str() {
        "track" => TrackKind::Track,
        "episode" => TrackKind::Episode,
        _ => return None,
    };
    let cover_url = match kind {
        TrackKind::Track => item
            .album
            .as_ref()
            .and_then(|a| a.images.first())
            .map(|i| i.url.clone()),
        TrackKind::Episode => item.images.first().map(|i| i.url.clone()),
    };
    Some(TrackInfo {
        // Local files have no id; the URI still identifies them. The final
        // fallback only guards against a hypothetically bare item.
        uri: item
            .uri
            .clone()
            .or_else(|| item.id.clone())
            .unwrap_or_else(|| format!("unidentified:{}:{}", item.name, item.duration_ms)),
        title: item.name.clone(),
        artists: item.artists.iter().map(|a| a.name.clone()).collect(),
        cover_url,
        duration_ms: item.duration_ms,
        is_local: item.is_local,
        kind,
    })
}

// ---------------------------------------------------------------------------
// Engine handle (owned by AppState)
// ---------------------------------------------------------------------------

/// Handle to the polling loop: at most one loop runs at a time; `stop()`
/// invalidates the running loop's generation, `wake()` nudges it to poll now.
#[derive(Default)]
pub struct PlayerEngine {
    /// Bumped by `stop()`; a loop exits once its generation is stale.
    generation: AtomicU64,
    /// Re-entrancy guard: true while a loop task is alive.
    running: AtomicBool,
    wake: Notify,
}

impl PlayerEngine {
    /// Claim the single loop slot. Returns the generation the new loop must
    /// watch, or `None` when a loop is already running.
    pub fn try_begin(&self) -> Option<u64> {
        if self.running.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(self.generation.load(Ordering::SeqCst))
    }

    /// Release the loop slot (called by the loop task on exit).
    pub fn end_run(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Whether `generation` is still the live one.
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Stop the running loop (logout): invalidate its generation and wake it
    /// so it exits without issuing another request.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// Nudge the loop to poll immediately (UI focus / wake signal).
    /// `notify_one` stores a permit, so a nudge sent while the loop is busy
    /// processing still cuts its NEXT sleep short instead of getting lost.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Sleep that a wake signal (or stop) cuts short — the normal poll delay.
    /// Returns true when the FULL delay elapsed (needed by boundary one-shots
    /// to distinguish "time to act" from "woken early, re-poll instead").
    pub async fn sleep_interruptible(&self, delay: Duration) -> bool {
        tokio::time::timeout(delay, self.wake.notified())
            .await
            .is_err()
    }

    /// Sleep the FULL delay regardless of wake nudges (Retry-After must be
    /// honored exactly); only a stale generation (stop) ends it early.
    pub async fn sleep_exact(&self, delay: Duration, generation: u64) {
        let deadline = tokio::time::Instant::now() + delay;
        loop {
            if !self.is_current(generation) {
                return;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return;
            }
            // Wake nudges land here but only shrink `remaining`, never the
            // deadline — Retry-After is enforced in full.
            let _ = tokio::time::timeout(remaining, self.wake.notified()).await;
        }
    }

    /// Park until the next wake signal (suspended state).
    pub async fn wait_for_wake(&self) {
        self.wake.notified().await;
    }
}

// ---------------------------------------------------------------------------
// The polling loop
// ---------------------------------------------------------------------------

/// How one poll attempt failed, from the loop's point of view.
enum TickError {
    /// Refresh is impossible (grant revoked, tokens/client-ID gone, repeated
    /// 401): stop polling, UI must re-authenticate.
    AuthLost,
    /// 429 with the parsed `Retry-After` seconds (None = header unusable).
    RateLimited(Option<u64>),
    /// Unparseable 200 body: skip this tick, keep the cadence.
    Malformed,
    /// Network trouble, timeouts, 5xx — retry with backoff.
    Transient(AuthError),
}

/// Start the polling loop unless one is already running. Safe to call on
/// every connect/restore/wake — extra calls are no-ops.
pub fn start(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(generation) = state.player.try_begin() else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_loop(&app, generation).await;
        app.state::<AppState>().player.end_run();
    });
}

/// Per-track preset cache of the poll loop: one store query per (track,
/// presets_version) instead of one per tick.
struct CachedCue {
    track_uri: String,
    presets_version: u64,
    /// M10: bumped when an applied auto-skip flag changes.
    suggestions_version: u64,
    cue: Option<automation::CuePoints>,
    /// M10: this track has an applied whole-song auto-skip.
    auto_skip: bool,
}

async fn run_loop(app: &AppHandle, generation: u64) {
    let mut last_emitted: Option<PlaybackState> = None;
    let mut last_emit_at: Option<Instant> = None;
    let mut idle_since: Option<Instant> = None;
    let mut failures: u32 = 0;
    // Automation state lives (and dies) with this loop task.
    let mut auto = Automation::default();
    let mut gates = Gates::default(); // closed until the first successful poll
    let mut cue_cache: Option<CachedCue> = None;
    let mut device_id: Option<String> = None;
    // `Some(id)` = a control call got a 403 on device `id`; automation stays
    // suspended until the active device changes.
    let mut restricted_device: Option<Option<String>> = None;
    // M7 latency probe: (predicted URI, burst start) — log-only measurement
    // of transition→seek, resolved on the seek or dropped on expiry/miss.
    let mut burst_probe: Option<(String, Instant)> = None;
    // M9: recent track metadata, so an insight event (including a skip_next
    // for the track that just ended) can be persisted with display fields.
    let mut meta_cache = MetaCache::new();

    loop {
        if !app.state::<AppState>().player.is_current(generation) {
            return;
        }

        let tick = poll_once(app).await;

        // Re-check after the request: a logout during the round-trip means
        // this result must neither be emitted nor schedule another poll.
        let state = app.state::<AppState>();
        if !state.player.is_current(generation) {
            return;
        }

        // Retry-After sleeps must run their full course; wake nudges may only
        // shorten the ordinary poll delays.
        let rate_limited = matches!(tick, Err(TickError::RateLimited(_)));
        // Boundary one-shot for this iteration (set only after a successful
        // poll of a playing preset track near its skip point).
        let mut boundary_wake: Option<Duration> = None;
        // M7: natural-end confirmation wakeup and fast-poll burst spacing.
        let mut transition_wake: Option<Duration> = None;
        let mut burst_delay: Option<Duration> = None;
        let next = match tick {
            Ok(resp) => {
                failures = 0;
                let outcome = match resp.as_ref() {
                    Some(r) => PollOutcome::Active {
                        playing: r.is_playing,
                    },
                    None => PollOutcome::NoDevice,
                };
                let idle_for = match outcome {
                    PollOutcome::NoDevice => idle_since.get_or_insert_with(Instant::now).elapsed(),
                    PollOutcome::Active { .. } => {
                        idle_since = None;
                        Duration::ZERO
                    }
                };

                // A 403-restricted device is retried once the ACTIVE device
                // changes — that is the only thing that can fix it.
                if let Some(r) = resp.as_ref() {
                    let current = r.device.as_ref().and_then(|d| d.id.clone());
                    if device_id != current {
                        device_id = current;
                    }
                    if restricted_device
                        .as_ref()
                        .is_some_and(|blocked| *blocked != device_id)
                    {
                        restricted_device = None;
                    }
                }
                let mut snapshot = snapshot_from_response(resp.as_ref(), unix_ms());
                let obs = automation_obs(&snapshot);
                gates = Gates {
                    automation_on: state.automation_on.load(Ordering::SeqCst),
                    premium: state.premium.load(Ordering::SeqCst),
                    device_ok: restricted_device.is_none(),
                    // Edit mode suspends automation for the edited track
                    // ONLY — anything else playing is handled as usual.
                    edit_hold: obs
                        .as_ref()
                        .is_some_and(|o| editing_track(&state, &o.track_uri)),
                    // M9: gates ONLY event recording (never actions).
                    insights_on: state.insights_on.load(Ordering::SeqCst),
                };
                snapshot.automation_suspended = if !gates.premium {
                    Some(AutomationSuspension::NoPremium)
                } else if restricted_device.is_some() {
                    Some(AutomationSuspension::RestrictedDevice)
                } else {
                    None
                };
                let (cue, auto_skip) = match obs.as_ref().filter(|o| o.controllable) {
                    Some(o) => lookup_track_config(app, &mut cue_cache, &o.track_uri),
                    None => (None, false),
                };
                // Remember the playing track's metadata before `snapshot` is
                // consumed, so a later skip_next for it can still render.
                if let Some(track) = snapshot.track.as_ref() {
                    if track.kind == TrackKind::Track && !track.is_local {
                        meta_cache.put(track);
                    }
                }
                maybe_emit(app, &mut last_emitted, &mut last_emit_at, snapshot);

                // A UI seek (edit-mode preview / exit-restore) happened since
                // the last poll: absorb the jump this observation shows.
                if state.ui_seek_pending.swap(false, Ordering::SeqCst) {
                    auto.note_external_seek();
                }

                // Resolve or expire the M7 latency probe: a different track
                // than predicted appeared (miss), or it simply went stale.
                if let Some((uri, started)) = burst_probe.as_ref() {
                    let current = obs.as_ref().map(|o| o.track_uri.as_str());
                    if auto.prearmed_track().is_none() && current != Some(uri.as_str()) {
                        eprintln!(
                            "cued: queue prediction missed (observed a different track) — normal handling"
                        );
                        burst_probe = None;
                    } else if started.elapsed() > BURST_PROBE_EXPIRY {
                        burst_probe = None;
                    }
                }

                // M10: arm/disarm whole-song auto-skip for this track before
                // deciding — it flows through the SAME gates as every action.
                auto.set_auto_skip(auto_skip);
                let decision = auto.on_poll(obs.as_ref(), cue, gates, unix_ms());
                // Persist any user-behavior events this poll classified — off
                // the hot path (enqueue only). Independent of the action.
                record_insights(app, &meta_cache, auto.take_events());
                if let Some(action) = decision {
                    if let Some(o) = obs.as_ref() {
                        eprintln!(
                            "cued: automation decided {action:?} (poll, position {} ms)",
                            o.position_ms
                        );
                    }
                    // The seek the burst was confirming: report the measured
                    // transition→seek latency (manual-test visibility).
                    let probe_hit = matches!(action, automation::Action::SeekToStart { .. })
                        && obs
                            .as_ref()
                            .zip(burst_probe.as_ref())
                            .is_some_and(|(o, (uri, _))| o.track_uri == *uri);
                    match run_action(app, &mut auto, action).await {
                        AfterAction::Repoll => {
                            if probe_hit {
                                if let Some((uri, started)) = burst_probe.take() {
                                    eprintln!(
                                        "cued: predictive start-jump: transition→seek {} ms ({uri})",
                                        started.elapsed().as_millis()
                                    );
                                }
                            }
                            // Our own successful skip with a prediction just
                            // armed a burst — start its clock.
                            note_burst_started(&auto, gates, &mut burst_probe, "own skip");
                        }
                        AfterAction::RestrictedDevice => {
                            restricted_device = Some(device_id.clone());
                        }
                        AfterAction::RateLimited(retry_after_secs) => {
                            auto.note_rate_limited();
                            emit_rate_limited(app, &mut last_emitted, &mut last_emit_at);
                            state
                                .player
                                .sleep_exact(rate_limit_delay(retry_after_secs), generation)
                                .await;
                        }
                    }
                    // Resync immediately after every action.
                    continue;
                }

                // M7: keep the queue prediction fresh (the pure module caps
                // this at MAX_QUEUE_FETCHES_PER_INSTANCE, errors included).
                if auto.wants_queue_fetch(gates, unix_ms()) {
                    match fetch_queue_once(app).await {
                        Ok(queue) => {
                            let next = predicted_next(&queue);
                            let prearm = next.and_then(|item| {
                                let uri = item.uri.as_deref()?;
                                prearm_from(uri, query_cue(app, uri))
                            });
                            match (next, prearm.as_ref()) {
                                (Some(item), Some(p)) => eprintln!(
                                    "cued: queue prediction: up next {:?} — pre-armed start jump to {} ms ({})",
                                    item.name.as_deref().unwrap_or("?"),
                                    p.start_ms,
                                    p.track_uri
                                ),
                                _ => eprintln!("cued: queue prediction: nothing to pre-arm"),
                            }
                            auto.queue_fetched(prearm, unix_ms());
                        }
                        Err(AuthError::RateLimited { retry_after_secs }) => {
                            auto.queue_fetch_failed(unix_ms());
                            auto.note_rate_limited();
                            eprintln!(
                                "cued: queue fetch hit a rate limit (Retry-After {retry_after_secs:?} s)"
                            );
                            emit_rate_limited(app, &mut last_emitted, &mut last_emit_at);
                            state
                                .player
                                .sleep_exact(rate_limit_delay(retry_after_secs), generation)
                                .await;
                            continue;
                        }
                        Err(e) => {
                            // Prediction is an optimization, never a
                            // dependency: log and carry on exactly as before.
                            auto.queue_fetch_failed(unix_ms());
                            eprintln!("cued: queue fetch failed (prediction skipped): {e}");
                        }
                    }
                }

                boundary_wake = auto
                    .plan_wakeup_ms(gates, unix_ms())
                    .map(Duration::from_millis);
                transition_wake = auto
                    .plan_transition_wakeup_ms(gates, unix_ms())
                    .map(Duration::from_millis);
                burst_delay = auto
                    .plan_burst_delay_ms(gates, unix_ms())
                    .map(Duration::from_millis);
                plan_next_delay(outcome, idle_for)
            }
            Err(TickError::RateLimited(retry_after_secs)) => {
                // No burst may run while a Retry-After is pending.
                auto.note_rate_limited();
                emit_rate_limited(app, &mut last_emitted, &mut last_emit_at);
                NextPoll::Delay(rate_limit_delay(retry_after_secs))
            }
            Err(TickError::Malformed) => {
                eprintln!("cued: /me/player returned an unparseable body; skipping this tick");
                NextPoll::Delay(POLL_PAUSED)
            }
            Err(TickError::AuthLost) => {
                let snapshot = PlaybackState::bare(PlaybackStatus::AuthLost, unix_ms());
                maybe_emit(app, &mut last_emitted, &mut last_emit_at, snapshot);
                return;
            }
            Err(TickError::Transient(e)) => {
                failures = failures.saturating_add(1);
                eprintln!(
                    "cued: playback poll failed ({}): {e}; consecutive failures: {failures}",
                    e.code()
                );
                let mut snapshot = last_emitted
                    .clone()
                    .unwrap_or_else(|| PlaybackState::bare(PlaybackStatus::Idle, unix_ms()));
                snapshot.reconnecting = true;
                maybe_emit(app, &mut last_emitted, &mut last_emit_at, snapshot);
                NextPoll::Delay(backoff_delay(failures, random_unit()))
            }
        };

        match (next, rate_limited) {
            (NextPoll::Delay(delay), true) => state.player.sleep_exact(delay, generation).await,
            (NextPoll::Delay(delay), false) => {
                // An active burst replaces the ordinary cadence with its
                // (shorter) fast-poll spacing. Rate-limited iterations never
                // get here, so no burst runs while a Retry-After is pending.
                let delay = burst_delay.map_or(delay, |b| b.min(delay));
                // The sooner of the two one-shots, when it lands before the
                // next poll would happen anyway.
                let one_shot = match (boundary_wake, transition_wake) {
                    (Some(b), Some(t)) if t < b => Some((OneShot::Transition, t)),
                    (Some(b), _) => Some((OneShot::SkipBoundary, b)),
                    (None, Some(t)) => Some((OneShot::Transition, t)),
                    (None, None) => None,
                }
                .filter(|(_, wake)| *wake < delay);
                match one_shot {
                    Some((OneShot::SkipBoundary, wake)) => {
                        // Sleep only until the skip boundary; if the full
                        // delay elapsed (no wake nudge), fire from
                        // interpolation, then re-poll immediately to resync.
                        let completed = state.player.sleep_interruptible(wake).await;
                        if completed && state.player.is_current(generation) {
                            if let Some(action) = auto.on_wakeup(gates, unix_ms()) {
                                eprintln!(
                                    "cued: automation decided {action:?} (boundary one-shot after {wake:?})"
                                );
                                match run_action(app, &mut auto, action).await {
                                    AfterAction::Repoll => {
                                        // A successful skip with a prediction
                                        // armed the burst here too.
                                        note_burst_started(
                                            &auto,
                                            gates,
                                            &mut burst_probe,
                                            "own skip",
                                        );
                                    }
                                    AfterAction::RestrictedDevice => {
                                        restricted_device = Some(device_id.clone());
                                    }
                                    AfterAction::RateLimited(retry_after_secs) => {
                                        auto.note_rate_limited();
                                        emit_rate_limited(
                                            app,
                                            &mut last_emitted,
                                            &mut last_emit_at,
                                        );
                                        state
                                            .player
                                            .sleep_exact(
                                                rate_limit_delay(retry_after_secs),
                                                generation,
                                            )
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                    Some((OneShot::Transition, wake)) => {
                        // Sleep until just before the predicted natural end,
                        // then start the confirmation burst — never an
                        // action, only faster polling: the loop re-polls
                        // right after this sleep either way.
                        let completed = state.player.sleep_interruptible(wake).await;
                        if completed
                            && state.player.is_current(generation)
                            && auto.on_transition_wakeup(gates, unix_ms())
                        {
                            note_burst_started(&auto, gates, &mut burst_probe, "natural end");
                        }
                    }
                    None => {
                        state.player.sleep_interruptible(delay).await;
                    }
                }
            }
            (NextPoll::Suspend, _) => {
                let snapshot = PlaybackState::bare(PlaybackStatus::Suspended, unix_ms());
                maybe_emit(app, &mut last_emitted, &mut last_emit_at, snapshot);
                match plan_suspended_wait(state.window_visible.load(Ordering::SeqCst)) {
                    // Hidden: no UI exists to nudge us, so slow-poll to
                    // detect resumed playback. `idle_since` stays put — as
                    // long as nothing plays, the loop lands right back here.
                    Some(delay) => {
                        state.player.sleep_interruptible(delay).await;
                    }
                    None => {
                        state.player.wait_for_wake().await;
                        // Fresh 60 s idle window after every resume.
                        idle_since = None;
                    }
                }
            }
        }
    }
}

/// Which one-shot timer the loop is sleeping toward (M4 skip boundary or the
/// M7 natural-end transition confirmation).
enum OneShot {
    SkipBoundary,
    Transition,
}

/// What the loop must do after a control call was executed.
enum AfterAction {
    /// Success or transient failure: re-poll right away to resync.
    Repoll,
    /// 403 — the active device refuses control; suspend for this device.
    RestrictedDevice,
    /// 429 — honor `Retry-After` in full before anything else.
    RateLimited(Option<u64>),
}

/// Whether the UI is editing the preset of `track_uri` (M8 edit mode). A
/// poisoned lock reads as "not editing": automation staying live is the
/// recoverable failure mode — the opposite would freeze automation for good.
fn editing_track(state: &AppState, track_uri: &str) -> bool {
    match state.edit_hold.lock() {
        Ok(hold) => hold.as_deref() == Some(track_uri),
        Err(_) => {
            eprintln!("cued: edit-mode lock is poisoned; treating as not editing");
            false
        }
    }
}

/// Display metadata for one recently-seen track (M9 insights).
struct TrackMeta {
    uri: String,
    title: String,
    artists: Vec<String>,
    cover_url: Option<String>,
}

/// Bounded, insertion-ordered cache of recent track metadata. It lets the
/// poll loop attach display fields to an insight event — including a
/// `skip_next` for the track that just ended, which is still cached from its
/// own polls. Bounded per the reliability rules; a couple of entries suffice.
struct MetaCache {
    entries: Vec<TrackMeta>,
}

impl MetaCache {
    const CAP: usize = 16;

    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Insert or refresh a track, evicting the oldest entry past the cap.
    fn put(&mut self, track: &TrackInfo) {
        if let Some(pos) = self.entries.iter().position(|e| e.uri == track.uri) {
            self.entries.remove(pos);
        } else if self.entries.len() >= Self::CAP {
            self.entries.remove(0);
        }
        self.entries.push(TrackMeta {
            uri: track.uri.clone(),
            title: track.title.clone(),
            artists: track.artists.clone(),
            cover_url: track.cover_url.clone(),
        });
    }

    fn get(&self, uri: &str) -> Option<&TrackMeta> {
        self.entries.iter().find(|e| e.uri == uri)
    }
}

/// Enqueue the user-behavior events classified this poll (M9). Metadata comes
/// from the loop's cache (the event's track was just playing); a missing
/// entry degrades to empty display fields rather than dropping the event.
/// Enqueue-only — the write happens on the background insights task.
fn record_insights(app: &AppHandle, meta_cache: &MetaCache, events: Vec<automation::InsightEvent>) {
    if events.is_empty() {
        return;
    }
    let sink = app.state::<crate::insights::InsightsSink>();
    for ev in events {
        let meta = meta_cache.get(&ev.track_uri);
        sink.record(crate::presets::InsightWrite {
            kind: ev.kind.as_str(),
            from_ms: ev.from_ms,
            to_ms: ev.to_ms,
            duration_ms: ev.duration_ms,
            title: meta.map(|m| m.title.clone()).unwrap_or_default(),
            artists: meta.map(|m| m.artists.clone()).unwrap_or_default(),
            cover_url: meta.and_then(|m| m.cover_url.clone()),
            created_at: unix_ms(),
            track_uri: ev.track_uri,
        });
    }
}

/// The playback observation the automation feeds on, or None when nothing
/// displayable/positionable is loaded.
fn automation_obs(snapshot: &PlaybackState) -> Option<automation::Obs> {
    let track = snapshot.track.as_ref()?;
    let position_ms = snapshot.position_ms?;
    Some(automation::Obs {
        track_uri: track.uri.clone(),
        controllable: track.kind == TrackKind::Track && !track.is_local,
        playing: snapshot.status == PlaybackStatus::Playing,
        position_ms,
        duration_ms: track.duration_ms,
        fetched_at_ms: snapshot.fetched_at_ms,
    })
}

/// Preset + auto-skip lookup for the playing track, cached per
/// (track, presets_version, suggestions_version). Returns the cue points (if
/// any) and whether the track carries an APPLIED whole-song auto-skip (M10).
fn lookup_track_config(
    app: &AppHandle,
    cache: &mut Option<CachedCue>,
    track_uri: &str,
) -> (Option<automation::CuePoints>, bool) {
    let state = app.state::<AppState>();
    let presets_version = state.presets_version.load(Ordering::SeqCst);
    let suggestions_version = state.suggestions_version.load(Ordering::SeqCst);
    if let Some(cached) = cache.as_ref() {
        if cached.track_uri == track_uri
            && cached.presets_version == presets_version
            && cached.suggestions_version == suggestions_version
        {
            return (cached.cue, cached.auto_skip);
        }
    }
    let cue = query_cue(app, track_uri);
    let auto_skip = query_auto_skip(app, track_uri);
    *cache = Some(CachedCue {
        track_uri: track_uri.to_owned(),
        presets_version,
        suggestions_version,
        cue,
        auto_skip,
    });
    (cue, auto_skip)
}

/// One uncached applied-auto-skip lookup (M10). A failing store degrades to
/// "not auto-skipped" (logged) — never fights playback on a read error.
fn query_auto_skip(app: &AppHandle, track_uri: &str) -> bool {
    match app
        .state::<crate::presets::PresetDb>()
        .store()
        .and_then(|store| store.is_auto_skip_applied(track_uri))
    {
        Ok(applied) => applied,
        Err(e) => {
            eprintln!("cued: auto-skip lookup failed: {e}");
            false
        }
    }
}

/// One uncached preset query. A failing store degrades to "no preset"
/// (logged) — never to a crash.
fn query_cue(app: &AppHandle, track_uri: &str) -> Option<automation::CuePoints> {
    match app
        .state::<crate::presets::PresetDb>()
        .store()
        .and_then(|store| store.get(track_uri))
    {
        Ok(preset) => preset.map(|p| automation::CuePoints {
            start_ms: p.start_ms,
            skip_ms: p.skip_ms,
        }),
        Err(e) => {
            eprintln!("cued: automation preset lookup failed: {e}");
            None
        }
    }
}

/// The queue's likely next item, when it is a controllable track (M7). Only
/// the FIRST entry matters — the transition goes to it regardless, so an
/// episode or local file up next simply means "nothing to predict".
fn predicted_next(resp: &crate::spotify::QueueResponse) -> Option<&crate::spotify::QueueItem> {
    let first = resp.queue.first()?;
    (first.item_type.as_deref() == Some("track") && !first.is_local && first.uri.is_some())
        .then_some(first)
}

/// A prediction is pre-armed only when the predicted track has a preset that
/// actually starts late (`start_ms == 0` never issues a seek).
fn prearm_from(
    track_uri: &str,
    cue: Option<automation::CuePoints>,
) -> Option<automation::Prearmed> {
    let cue = cue.filter(|c| c.start_ms > 0)?;
    Some(automation::Prearmed {
        track_uri: track_uri.to_owned(),
        start_ms: cue.start_ms,
    })
}

/// One `GET /v1/me/player/queue` round-trip (M7 prediction). The poll that
/// led here refreshed the access token moments ago.
async fn fetch_queue_once(app: &AppHandle) -> Result<crate::spotify::QueueResponse, AuthError> {
    let state = app.state::<AppState>();
    let token = state
        .tokens
        .lock()
        .await
        .as_ref()
        .map(|t| t.access_token.clone());
    let Some(token) = token else {
        return Err(AuthError::Config("not connected".into()));
    };
    spotify::fetch_queue(&state.http, &token).await
}

/// Remember (and log) a freshly started transition burst: the predicted URI
/// plus a clock, so the transition→seek latency can be reported later.
fn note_burst_started(
    auto: &Automation,
    gates: Gates,
    probe: &mut Option<(String, Instant)>,
    trigger: &str,
) {
    if probe.is_some() || auto.plan_burst_delay_ms(gates, unix_ms()).is_none() {
        return;
    }
    let Some(uri) = auto.prearmed_track() else {
        return;
    };
    eprintln!("cued: transition burst begins ({trigger}) — confirming {uri}");
    *probe = Some((uri.to_owned(), Instant::now()));
}

/// Execute one control action and record its outcome in the automation
/// state (cooldown, action cap, retry bookkeeping).
async fn run_action(
    app: &AppHandle,
    auto: &mut Automation,
    action: automation::Action,
) -> AfterAction {
    let started = Instant::now();
    let result = execute_action(app, action).await;
    let now = unix_ms();
    match result {
        Ok(()) => {
            eprintln!(
                "cued: automation executed {action:?} in {:?}",
                started.elapsed()
            );
            auto.action_executed(action, automation::Outcome::Success, now);
            AfterAction::Repoll
        }
        Err(AuthError::RateLimited { retry_after_secs }) => {
            eprintln!(
                "cued: automation {action:?} hit a rate limit (Retry-After {retry_after_secs:?} s)"
            );
            auto.action_executed(action, automation::Outcome::Fatal, now);
            AfterAction::RateLimited(retry_after_secs)
        }
        Err(AuthError::Api { status: 403, .. }) => {
            eprintln!(
                "cued: automation {action:?} rejected (403) — this device can't be controlled"
            );
            auto.action_executed(action, automation::Outcome::Fatal, now);
            AfterAction::RestrictedDevice
        }
        Err(e) => {
            eprintln!("cued: automation {action:?} failed transiently: {e}");
            auto.action_executed(action, automation::Outcome::Transient, now);
            AfterAction::Repoll
        }
    }
}

/// The raw control call. Uses the current access token; the poll that
/// produced this decision already refreshed it moments ago.
async fn execute_action(app: &AppHandle, action: automation::Action) -> Result<(), AuthError> {
    let state = app.state::<AppState>();
    let token = state
        .tokens
        .lock()
        .await
        .as_ref()
        .map(|t| t.access_token.clone());
    let Some(token) = token else {
        return Err(AuthError::Config("not connected".into()));
    };
    match action {
        automation::Action::SeekToStart { start_ms } => {
            spotify::seek(&state.http, &token, start_ms).await
        }
        automation::Action::SkipNext => spotify::next_track(&state.http, &token).await,
    }
}

/// Show the rate-limit suspension in the UI before the enforced sleep, so
/// the notice does not wait for the next successful poll.
fn emit_rate_limited(
    app: &AppHandle,
    last_emitted: &mut Option<PlaybackState>,
    last_emit_at: &mut Option<Instant>,
) {
    let Some(mut snapshot) = last_emitted.clone() else {
        return;
    };
    snapshot.automation_suspended = Some(AutomationSuspension::RateLimited);
    maybe_emit(app, last_emitted, last_emit_at, snapshot);
}

/// One `GET /v1/me/player` round-trip with the M1 token discipline:
/// refresh when expired, and on a 401 refresh once + retry once, then
/// degrade to `AuthLost` (never loop).
async fn poll_once(app: &AppHandle) -> Result<Option<PlayerResponse>, TickError> {
    let state = app.state::<AppState>();
    let client_id = match config::load_client_id(app) {
        Ok(Some(id)) => id,
        // No Client ID means no way to ever refresh — same as logged out.
        Ok(None) => return Err(TickError::AuthLost),
        Err(e) => return Err(TickError::Transient(e)),
    };

    let tokens = {
        let mut guard = state.tokens.lock().await;
        let Some(current) = guard.clone() else {
            return Err(TickError::AuthLost);
        };
        match crate::commands::ensure_fresh(&state.http, &client_id, current).await {
            Ok(fresh) => {
                *guard = Some(fresh.clone());
                fresh
            }
            Err(AuthError::Api { status, .. }) if (400..500).contains(&status) => {
                return Err(TickError::AuthLost);
            }
            Err(e) => return Err(TickError::Transient(e)),
        }
    };

    match spotify::fetch_player(&state.http, &tokens.access_token).await {
        Err(AuthError::Api { status: 401, .. }) => {
            let mut guard = state.tokens.lock().await;
            let Some(current) = guard.clone() else {
                return Err(TickError::AuthLost);
            };
            let resp = match spotify::refresh_access_token(
                &state.http,
                &client_id,
                &current.refresh_token,
            )
            .await
            {
                Ok(r) => r,
                Err(AuthError::Api { status, .. }) if (400..500).contains(&status) => {
                    return Err(TickError::AuthLost);
                }
                Err(e) => return Err(TickError::Transient(e)),
            };
            let fresh = crate::commands::tokens_from_response(resp, Some(current.refresh_token))
                .map_err(|_| TickError::AuthLost)?;
            token_store::save(&fresh).map_err(TickError::Transient)?;
            *guard = Some(fresh.clone());
            drop(guard);
            match spotify::fetch_player(&state.http, &fresh.access_token).await {
                // Still 401 on a freshly minted token: give up, don't loop.
                Err(AuthError::Api { status: 401, .. }) => Err(TickError::AuthLost),
                other => other.map_err(map_fetch_err),
            }
        }
        other => other.map_err(map_fetch_err),
    }
}

fn map_fetch_err(e: AuthError) -> TickError {
    match e {
        AuthError::RateLimited { retry_after_secs } => TickError::RateLimited(retry_after_secs),
        AuthError::MalformedResponse => TickError::Malformed,
        other => TickError::Transient(other),
    }
}

fn maybe_emit(
    app: &AppHandle,
    last_emitted: &mut Option<PlaybackState>,
    last_emit_at: &mut Option<Instant>,
    next: PlaybackState,
) {
    let since = last_emit_at.map(|t| t.elapsed()).unwrap_or(Duration::MAX);
    if !should_emit(last_emitted.as_ref(), &next, since) {
        return;
    }
    if let Err(e) = app.emit(STATE_EVENT, &next) {
        eprintln!("cued: failed to emit playback state: {e}");
    }
    // Same changed-only discipline for the tray menu; identical text is
    // skipped inside (heartbeats repeat unchanged snapshots).
    crate::tray::update_now_playing(app, next.track.as_ref());
    *last_emitted = Some(next);
    *last_emit_at = Some(Instant::now());
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Uniform value in `[0, 1]` from the OS CSPRNG (already a dependency for
/// PKCE); falls back to the jitter midpoint if the RNG is unavailable.
fn random_unit() -> f64 {
    let mut buf = [0u8; 8];
    if getrandom::fill(&mut buf).is_err() {
        return 0.5;
    }
    (u64::from_le_bytes(buf) >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(uri: &str) -> TrackInfo {
        TrackInfo {
            uri: uri.into(),
            title: "Song".into(),
            artists: vec!["Artist".into()],
            cover_url: None,
            duration_ms: 200_000,
            is_local: false,
            kind: TrackKind::Track,
        }
    }

    fn playing(uri: &str, position_ms: u64, fetched_at_ms: u64) -> PlaybackState {
        PlaybackState {
            status: PlaybackStatus::Playing,
            reconnecting: false,
            track: Some(track(uri)),
            position_ms: Some(position_ms),
            fetched_at_ms,
            automation_suspended: None,
        }
    }

    // -- plan_next_delay ----------------------------------------------------

    #[test]
    fn plans_1s_while_playing() {
        let plan = plan_next_delay(PollOutcome::Active { playing: true }, Duration::ZERO);
        assert_eq!(plan, NextPoll::Delay(POLL_PLAYING));
    }

    #[test]
    fn plans_2s_while_paused() {
        let plan = plan_next_delay(PollOutcome::Active { playing: false }, Duration::ZERO);
        assert_eq!(plan, NextPoll::Delay(POLL_PAUSED));
    }

    #[test]
    fn plans_idle_cadence_before_the_suspend_threshold() {
        let plan = plan_next_delay(PollOutcome::NoDevice, IDLE_SUSPEND_AFTER - POLL_IDLE);
        assert_eq!(plan, NextPoll::Delay(POLL_IDLE));
    }

    #[test]
    fn suspends_after_60s_without_a_device() {
        assert_eq!(
            plan_next_delay(PollOutcome::NoDevice, IDLE_SUSPEND_AFTER),
            NextPoll::Suspend
        );
    }

    // -- plan_suspended_wait (hidden-suspend slow poll) -----------------------

    #[test]
    fn suspended_with_a_visible_window_parks_until_a_wake() {
        assert_eq!(plan_suspended_wait(true), None);
    }

    #[test]
    fn suspended_while_hidden_slow_polls_at_the_named_cadence() {
        assert_eq!(plan_suspended_wait(false), Some(POLL_HIDDEN_SUSPENDED));
    }

    #[test]
    fn hidden_never_polls_faster_than_visible() {
        for visible_cadence in [POLL_PLAYING, POLL_PAUSED, POLL_IDLE] {
            assert!(POLL_HIDDEN_SUSPENDED >= visible_cadence);
        }
    }

    // -- backoff_delay ------------------------------------------------------

    #[test]
    fn backoff_grows_exponentially_and_respects_jitter_bounds() {
        for failures in 1..12u32 {
            let base = BACKOFF_BASE
                .saturating_mul(1u32 << failures.saturating_sub(1).min(16))
                .min(BACKOFF_CAP);
            for jitter in [0.0, 0.3, 0.5, 1.0] {
                let d = backoff_delay(failures, jitter);
                assert!(d >= base.mul_f64(0.5), "too short: {d:?} for n={failures}");
                assert!(d <= base, "too long: {d:?} for n={failures}");
                assert!(d <= BACKOFF_CAP, "cap exceeded: {d:?}");
            }
        }
    }

    #[test]
    fn backoff_caps_at_30s_even_for_huge_failure_counts() {
        assert_eq!(backoff_delay(u32::MAX, 1.0), BACKOFF_CAP);
    }

    #[test]
    fn backoff_clamps_out_of_range_jitter() {
        assert_eq!(backoff_delay(1, 7.5), BACKOFF_BASE);
        assert_eq!(backoff_delay(1, -3.0), BACKOFF_BASE.mul_f64(0.5));
    }

    // -- rate limiting ------------------------------------------------------

    #[test]
    fn rate_limit_honors_retry_after_exactly() {
        assert_eq!(rate_limit_delay(Some(7)), Duration::from_secs(7));
        assert_eq!(rate_limit_delay(Some(0)), Duration::ZERO);
    }

    #[test]
    fn rate_limit_falls_back_when_the_header_is_missing() {
        assert_eq!(rate_limit_delay(None), RATE_LIMIT_FALLBACK);
    }

    // -- should_emit ----------------------------------------------------------

    #[test]
    fn emits_the_first_snapshot() {
        assert!(should_emit(None, &playing("a", 0, 0), Duration::ZERO));
    }

    #[test]
    fn stays_quiet_when_nothing_changed_within_the_heartbeat() {
        let prev = playing("a", 1000, 10_000);
        let next = playing("a", 2010, 11_010); // natural progression
        assert!(!should_emit(Some(&prev), &next, Duration::from_secs(1)));
    }

    #[test]
    fn emits_on_track_change() {
        let prev = playing("a", 1000, 10_000);
        let next = playing("b", 0, 11_000);
        assert!(should_emit(Some(&prev), &next, Duration::from_secs(1)));
    }

    #[test]
    fn emits_on_play_pause_flip() {
        let prev = playing("a", 1000, 10_000);
        let mut next = playing("a", 2000, 11_000);
        next.status = PlaybackStatus::Paused;
        assert!(should_emit(Some(&prev), &next, Duration::from_secs(1)));
    }

    #[test]
    fn emits_on_a_seek_jump() {
        let prev = playing("a", 1000, 10_000);
        let next = playing("a", 50_000, 11_000); // expected ~2000
        assert!(should_emit(Some(&prev), &next, Duration::from_secs(1)));
    }

    #[test]
    fn emits_on_a_backwards_seek_while_paused() {
        let mut prev = playing("a", 60_000, 10_000);
        prev.status = PlaybackStatus::Paused;
        let mut next = playing("a", 1000, 11_000);
        next.status = PlaybackStatus::Paused;
        assert!(should_emit(Some(&prev), &next, Duration::from_secs(1)));
    }

    #[test]
    fn emits_the_heartbeat_after_5s_of_silence() {
        let prev = playing("a", 1000, 10_000);
        let next = playing("a", 6010, 15_010);
        assert!(should_emit(Some(&prev), &next, HEARTBEAT_EVERY));
    }

    #[test]
    fn emits_when_the_reconnecting_flag_flips() {
        let prev = playing("a", 1000, 10_000);
        let mut next = prev.clone();
        next.reconnecting = true;
        assert!(should_emit(Some(&prev), &next, Duration::ZERO));
    }

    #[test]
    fn emits_when_the_automation_suspension_changes() {
        let prev = playing("a", 1000, 10_000);
        let mut next = playing("a", 2010, 11_010); // otherwise-natural progression
        next.automation_suspended = Some(AutomationSuspension::RestrictedDevice);
        assert!(should_emit(Some(&prev), &next, Duration::from_secs(1)));
    }

    // -- snapshot_from_response ----------------------------------------------

    fn parse(body: &str) -> PlayerResponse {
        crate::spotify::parse_player_body(body).expect("test body must parse")
    }

    #[test]
    fn snapshot_of_no_device_is_idle() {
        let s = snapshot_from_response(None, 42);
        assert_eq!(s.status, PlaybackStatus::Idle);
        assert!(s.track.is_none());
        assert_eq!(s.fetched_at_ms, 42);
    }

    #[test]
    fn snapshot_of_a_playing_track_carries_display_fields() {
        let resp = parse(
            r#"{
                "is_playing": true,
                "progress_ms": 12345,
                "item": {
                    "id": "t1", "uri": "spotify:track:t1", "name": "Song",
                    "duration_ms": 200000, "type": "track", "is_local": false,
                    "artists": [{"name": "A"}, {"name": "B"}],
                    "album": {"images": [{"url": "https://i.scdn.co/big"}]}
                }
            }"#,
        );
        let s = snapshot_from_response(Some(&resp), 7);
        assert_eq!(s.status, PlaybackStatus::Playing);
        assert_eq!(s.position_ms, Some(12345));
        let t = s.track.expect("track present");
        assert_eq!(t.uri, "spotify:track:t1");
        assert_eq!(t.artists, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(t.cover_url.as_deref(), Some("https://i.scdn.co/big"));
        assert_eq!(t.kind, TrackKind::Track);
    }

    #[test]
    fn snapshot_of_an_ad_is_idle_not_an_error() {
        let resp = parse(r#"{"is_playing": true, "progress_ms": 5, "item": null}"#);
        let s = snapshot_from_response(Some(&resp), 7);
        assert_eq!(s.status, PlaybackStatus::Idle);
        assert!(s.track.is_none());
    }

    #[test]
    fn snapshot_of_a_local_file_is_marked_local_and_keeps_an_identity() {
        let resp = parse(
            r#"{
                "is_playing": false,
                "progress_ms": 10,
                "item": {
                    "id": null, "uri": "spotify:local:::My+Song:100",
                    "name": "My Song", "duration_ms": 100000, "type": "track",
                    "is_local": true, "artists": [], "album": {"images": []}
                }
            }"#,
        );
        let s = snapshot_from_response(Some(&resp), 7);
        assert_eq!(s.status, PlaybackStatus::Paused);
        let t = s.track.expect("track present");
        assert!(t.is_local);
        assert_eq!(t.uri, "spotify:local:::My+Song:100");
        assert!(t.cover_url.is_none());
    }

    #[test]
    fn snapshot_of_an_episode_uses_its_direct_images() {
        let resp = parse(
            r#"{
                "is_playing": true,
                "progress_ms": 9,
                "item": {
                    "id": "e1", "uri": "spotify:episode:e1", "name": "Ep",
                    "duration_ms": 3600000, "type": "episode",
                    "images": [{"url": "https://i.scdn.co/ep"}]
                }
            }"#,
        );
        let t = snapshot_from_response(Some(&resp), 7)
            .track
            .expect("episode displayed");
        assert_eq!(t.kind, TrackKind::Episode);
        assert_eq!(t.cover_url.as_deref(), Some("https://i.scdn.co/ep"));
    }

    // -- M7: queue prediction extraction --------------------------------------

    fn parse_queue(body: &str) -> crate::spotify::QueueResponse {
        crate::spotify::parse_queue_body(body).expect("test queue body must parse")
    }

    #[test]
    fn the_first_real_track_in_the_queue_is_the_prediction() {
        let q = parse_queue(
            r#"{"queue": [
                {"uri": "spotify:track:b", "name": "Next", "type": "track",
                 "duration_ms": 180000, "is_local": false},
                {"uri": "spotify:track:c", "type": "track"}
            ]}"#,
        );
        let item = predicted_next(&q).expect("prediction expected");
        assert_eq!(item.uri.as_deref(), Some("spotify:track:b"));
    }

    #[test]
    fn episodes_local_files_and_empty_queues_yield_no_prediction() {
        // An episode up next: no prediction (never scan past the first item —
        // the transition goes to it regardless).
        let episode = parse_queue(
            r#"{"queue": [
                {"uri": "spotify:episode:e", "type": "episode"},
                {"uri": "spotify:track:b", "type": "track"}
            ]}"#,
        );
        assert!(predicted_next(&episode).is_none());
        let local = parse_queue(
            r#"{"queue": [{"uri": "spotify:local:::x:1", "type": "track", "is_local": true}]}"#,
        );
        assert!(predicted_next(&local).is_none());
        let empty = parse_queue(r#"{"queue": []}"#);
        assert!(predicted_next(&empty).is_none());
        let bare = parse_queue(r#"{"queue": [{"type": "track"}]}"#); // no uri
        assert!(predicted_next(&bare).is_none());
    }

    #[test]
    fn a_prediction_is_prearmed_only_with_a_preset_that_starts_late() {
        let cue = automation::CuePoints {
            start_ms: 20_000,
            skip_ms: 100_000,
        };
        let armed = prearm_from("spotify:track:b", Some(cue)).expect("prearmed");
        assert_eq!(armed.track_uri, "spotify:track:b");
        assert_eq!(armed.start_ms, 20_000);
        // No preset: nothing to pre-arm.
        assert!(prearm_from("spotify:track:b", None).is_none());
        // start_ms == 0 never issues a seek, so there is nothing to pre-arm.
        let zero = automation::CuePoints {
            start_ms: 0,
            skip_ms: 100_000,
        };
        assert!(prearm_from("spotify:track:b", Some(zero)).is_none());
    }

    // -- engine primitives -----------------------------------------------------

    #[test]
    fn engine_allows_only_one_loop_at_a_time() {
        let e = PlayerEngine::default();
        let g = e.try_begin().expect("first begin must win the slot");
        assert!(e.try_begin().is_none(), "second loop must be refused");
        assert!(e.is_current(g));
        e.end_run();
        assert!(e.try_begin().is_some(), "slot reusable after end_run");
    }

    #[test]
    fn stop_invalidates_the_running_generation() {
        let e = PlayerEngine::default();
        let g = e.try_begin().expect("begin");
        e.stop();
        assert!(!e.is_current(g));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_sleep_ignores_wake_nudges() {
        let e = PlayerEngine::default();
        let g = e.try_begin().expect("begin");
        e.wake(); // a stored wake permit must NOT shorten a Retry-After sleep
        let started = tokio::time::Instant::now();
        e.sleep_exact(Duration::from_secs(7), g).await;
        assert!(started.elapsed() >= Duration::from_secs(7));
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_ends_a_rate_limit_sleep_early() {
        let e = PlayerEngine::default();
        let g = e.try_begin().expect("begin");
        e.stop();
        let started = tokio::time::Instant::now();
        e.sleep_exact(Duration::from_secs(3600), g).await;
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_wake_nudge_cuts_a_normal_poll_delay_short() {
        let e = PlayerEngine::default();
        e.wake();
        let started = tokio::time::Instant::now();
        let completed = e.sleep_interruptible(Duration::from_secs(30)).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!completed, "a nudged sleep must not report a full elapse");
    }

    #[tokio::test(start_paused = true)]
    async fn an_undisturbed_sleep_reports_a_full_elapse() {
        let e = PlayerEngine::default();
        assert!(e.sleep_interruptible(Duration::from_millis(300)).await);
    }
}
