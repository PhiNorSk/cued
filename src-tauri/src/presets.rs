//! Local preset storage (M3).
//!
//! Presets (per-track start/skip points) live in a single SQLite database in
//! the app data dir. All access goes through [`PresetStore`]; the frontend
//! only ever sees typed commands. Validation here is authoritative — the UI
//! mirrors the same rules for instant feedback, but nothing invalid can be
//! written even by a buggy or bypassed frontend.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::suggestions::{self, Event, EventKind, Status, Suggestion, SuggestionType};

/// Minimum distance between the start and skip points (mirrored in TS).
pub const MIN_GAP_MS: u64 = 10_000;

/// Bump when the schema changes; stored in `PRAGMA user_version`.
/// v2 (M9) added the append-only listening-insights tables alongside presets;
/// v3 (M10) added the derived-suggestion state table + the analysis cursor.
const SCHEMA_VERSION: i64 = 3;

/// Database file name inside the app data dir.
const DB_FILE: &str = "cued.db";

/// Listening-insights schema (M9), created on a fresh DB and added by the
/// v1→v2 migration. Two tables in the SAME database as presets:
/// `listening_events` is APPEND-ONLY by product decision — no caps, no
/// pruning, no expiry; the user is the only one who ever deletes it (the
/// settings "delete all" action). `tracks` snapshots display metadata so
/// future suggestion/heatmap features render without any Spotify API call.
const INSIGHTS_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS tracks (
        uri         TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        artists     TEXT NOT NULL, -- JSON array of strings
        cover_url   TEXT,
        duration_ms INTEGER NOT NULL,
        last_seen   INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS listening_events (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        track_uri   TEXT NOT NULL,
        type        TEXT NOT NULL, -- seek_forward | seek_back | skip_next
        from_ms     INTEGER NOT NULL,
        to_ms       INTEGER,       -- NULL for skip_next
        duration_ms INTEGER NOT NULL,
        created_at  INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_listening_events_track_uri
        ON listening_events(track_uri);
";

/// Derived-suggestion schema (M10), created on a fresh DB and added by the
/// v2→v3 migration. `suggestions` holds ONE row per (track, type): its
/// lifecycle status, the "shown but ignored" counter, the proposed values and
/// the honest evidence (plays considered / matching). Auto-skip is represented
/// EXPLICITLY as a row here (`type = 'auto_skip'`, `status = 'applied'`) — never
/// as a fake 0/0 preset. `analysis_state` is a tiny key/value cursor so each
/// analysis run only touches tracks with events newer than the last run.
const SUGGESTIONS_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS suggestions (
        track_uri      TEXT NOT NULL,
        type           TEXT NOT NULL, -- skip_point | start_point | auto_skip
        status         TEXT NOT NULL, -- active | applied | dismissed | retired
        shown_count    INTEGER NOT NULL DEFAULT 0,
        value_start_ms INTEGER,
        value_end_ms   INTEGER,
        plays_total    INTEGER NOT NULL DEFAULT 0,
        plays_matching INTEGER NOT NULL DEFAULT 0,
        updated_at     INTEGER NOT NULL,
        PRIMARY KEY (track_uri, type)
    );
    CREATE TABLE IF NOT EXISTS analysis_state (
        key   TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    );
";

/// `analysis_state` key: the highest `listening_events.id` already analyzed.
const ANALYSIS_CURSOR_KEY: &str = "last_event_id";

// Boundary-input caps: presets are snapshots of Spotify metadata, so these
// are generous sanity bounds, not business rules.
// Shared with the M8 edit-mode commands (same boundary-input discipline).
pub(crate) const MAX_URI_LEN: usize = 512;
const MAX_TITLE_LEN: usize = 1_000;
const MAX_ARTISTS: usize = 50;
const MAX_ARTIST_LEN: usize = 500;
const MAX_COVER_URL_LEN: usize = 2_048;
/// 12 h — far above any Spotify track, catches garbage durations. Also the
/// sanity cap for M8 UI seek positions (a seek target is always ≤ duration).
pub(crate) const MAX_DURATION_MS: u64 = 43_200_000;

/// All failure modes of the preset store. Serialized to the frontend as
/// `{ code, message }`, same contract as [`crate::error::AuthError`].
#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("Invalid preset: {0}")]
    Validation(String),
    #[error("Preset database error: {0}")]
    Db(String),
    #[error("The preset database was created by a newer version of Cued.")]
    SchemaTooNew,
    #[error("The preset database file is corrupt.")]
    Corrupt,
    #[error("Configuration error: {0}")]
    Config(String),
}

impl PresetError {
    /// Stable machine-readable code for the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            PresetError::Validation(_) => "preset_validation",
            PresetError::Db(_) => "preset_db",
            PresetError::SchemaTooNew => "preset_schema_too_new",
            PresetError::Corrupt => "preset_corrupt",
            PresetError::Config(_) => "config",
        }
    }
}

impl serde::Serialize for PresetError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("PresetError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<rusqlite::Error> for PresetError {
    fn from(e: rusqlite::Error) -> Self {
        if is_corruption(&e) {
            PresetError::Corrupt
        } else {
            PresetError::Db(e.to_string())
        }
    }
}

fn is_corruption(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(
                f.code,
                rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
            )
    )
}

/// A stored preset, including the metadata snapshot taken at save time so the
/// Library renders without any Spotify API call.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub track_uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
    pub start_ms: u64,
    pub skip_ms: u64,
    /// Unix ms, set on first save and never changed.
    pub created_at: u64,
    /// Unix ms, set on every save.
    pub updated_at: u64,
}

/// What the frontend sends to create or update a preset (timestamps are
/// assigned here, never trusted from the caller).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetInput {
    pub track_uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
    pub start_ms: u64,
    pub skip_ms: u64,
}

/// One listening-insights event plus the track-metadata snapshot to upsert
/// alongside it (M9). Assembled by the poll loop from a classified
/// [`crate::automation::InsightEvent`] and the current playback snapshot;
/// written on the background insights writer, never in the poll hot path.
#[derive(Debug, Clone)]
pub struct InsightWrite {
    pub track_uri: String,
    /// `seek_forward` | `seek_back` | `skip_next` (from [`crate::automation`]).
    pub kind: &'static str,
    pub from_ms: u64,
    /// `None` for `skip_next` (there is no destination — the track ended).
    pub to_ms: Option<u64>,
    pub duration_ms: u64,
    pub title: String,
    pub artists: Vec<String>,
    pub cover_url: Option<String>,
    /// Unix ms, stamped when the event was queued.
    pub created_at: u64,
}

/// One stored suggestion joined with its track's display metadata (M10). The
/// analysis engine (`suggestions.rs`) produces the values; this is the
/// serialized shape the Now Playing card and the Library section render. The
/// metadata always exists because an event created the `tracks` row first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSuggestion {
    pub track_uri: String,
    /// `skip_point` | `start_point` | `auto_skip`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `active` | `applied` | `dismissed` | `retired`.
    pub status: String,
    pub shown_count: u32,
    pub value_start_ms: Option<u64>,
    pub value_end_ms: Option<u64>,
    pub plays_total: u32,
    pub plays_matching: u32,
    pub updated_at: u64,
    pub title: String,
    pub artists: Vec<String>,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
}

/// The core preset rule: 0 <= start < skip <= duration and a gap of at least
/// [`MIN_GAP_MS`]. (`start >= 0` is guaranteed by the unsigned type.)
pub fn validate_times(start_ms: u64, skip_ms: u64, duration_ms: u64) -> Result<(), PresetError> {
    if skip_ms > duration_ms {
        return Err(PresetError::Validation(
            "the skip point must not lie beyond the end of the track".into(),
        ));
    }
    // Covers start >= skip too: a non-negative gap of >= 10 s implies start < skip.
    if skip_ms < start_ms.saturating_add(MIN_GAP_MS) {
        return Err(PresetError::Validation(
            "start and skip must be at least 10 seconds apart".into(),
        ));
    }
    // The neutral state (start 0, skip at the end) means "the whole song
    // plays" — that is "no preset", never a stored row (M8).
    if start_ms == 0 && skip_ms == duration_ms {
        return Err(PresetError::Validation(
            "nothing to save — these times just play the whole song".into(),
        ));
    }
    Ok(())
}

/// Full input validation: time rule plus sanity caps on the snapshot fields.
pub fn validate_input(input: &PresetInput) -> Result<(), PresetError> {
    if input.track_uri.is_empty() || input.track_uri.len() > MAX_URI_LEN {
        return Err(PresetError::Validation(format!(
            "track URI must be 1–{MAX_URI_LEN} characters"
        )));
    }
    if input.title.len() > MAX_TITLE_LEN {
        return Err(PresetError::Validation("title is too long".into()));
    }
    if input.artists.len() > MAX_ARTISTS || input.artists.iter().any(|a| a.len() > MAX_ARTIST_LEN) {
        return Err(PresetError::Validation("artist list is too large".into()));
    }
    if input
        .cover_url
        .as_ref()
        .is_some_and(|u| u.len() > MAX_COVER_URL_LEN)
    {
        return Err(PresetError::Validation("cover URL is too long".into()));
    }
    if input.duration_ms > MAX_DURATION_MS {
        return Err(PresetError::Validation(
            "track duration is implausibly long".into(),
        ));
    }
    validate_times(input.start_ms, input.skip_ms, input.duration_ms)
}

/// Health info the UI fetches once at startup.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetDbHealth {
    /// True when a corrupt database was set aside and recreated this run.
    pub recovered: bool,
    /// Human-readable reason when the store could not be opened at all.
    pub failed: Option<String>,
}

/// Managed wrapper so a failed database open degrades to per-command errors
/// instead of aborting the whole app at startup.
pub enum PresetDb {
    Ready(PresetStore),
    Failed(String),
}

impl PresetDb {
    /// Open (or rescue) the database in the app data dir. Never fails: an
    /// unrecoverable open error is captured and surfaced per command.
    pub fn init(app: &tauri::AppHandle) -> Self {
        let result = app
            .path()
            .app_data_dir()
            .map_err(|e| PresetError::Config(format!("cannot resolve app data dir: {e}")))
            .and_then(|dir| PresetStore::open_at(&dir.join(DB_FILE)));
        match result {
            Ok(store) => PresetDb::Ready(store),
            Err(e) => {
                eprintln!("cued: preset database unavailable: {e}");
                PresetDb::Failed(e.to_string())
            }
        }
    }

    /// The store, or the captured open error.
    pub fn store(&self) -> Result<&PresetStore, PresetError> {
        match self {
            PresetDb::Ready(store) => Ok(store),
            PresetDb::Failed(msg) => Err(PresetError::Db(msg.clone())),
        }
    }

    /// Startup health for the UI notice.
    pub fn health(&self) -> PresetDbHealth {
        match self {
            PresetDb::Ready(store) => PresetDbHealth {
                recovered: store.recovered(),
                failed: None,
            },
            PresetDb::Failed(msg) => PresetDbHealth {
                recovered: false,
                failed: Some(msg.clone()),
            },
        }
    }
}

/// SQLite-backed preset storage. One connection behind a mutex — preset
/// traffic is tiny and strictly serialized writes keep reasoning simple.
#[derive(Debug)]
pub struct PresetStore {
    conn: Mutex<Connection>,
    recovered: bool,
}

impl PresetStore {
    /// Open the database at `path`, creating file and schema when missing.
    /// A corrupt file is renamed aside (`<name>.corrupt-<unix-ms>`) and a
    /// fresh database is created in its place (`recovered()` reports this).
    pub fn open_at(path: &Path) -> Result<Self, PresetError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PresetError::Config(format!("cannot create app data dir: {e}")))?;
        }
        match open_and_migrate(path) {
            Ok(conn) => Ok(Self {
                conn: Mutex::new(conn),
                recovered: false,
            }),
            Err(PresetError::Corrupt) => {
                set_aside_corrupt_file(path)?;
                let conn = open_and_migrate(path)?;
                Ok(Self {
                    conn: Mutex::new(conn),
                    recovered: true,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// True when a corrupt database was set aside during open.
    pub fn recovered(&self) -> bool {
        self.recovered
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, PresetError> {
        self.conn
            .lock()
            .map_err(|_| PresetError::Db("preset store lock is poisoned".into()))
    }

    /// Validate and upsert a preset inside a transaction; returns the stored
    /// row. `created_at` is preserved across updates.
    pub fn save(&self, input: &PresetInput) -> Result<Preset, PresetError> {
        validate_input(input)?;
        let artists_json = serde_json::to_string(&input.artists)
            .map_err(|e| PresetError::Db(format!("cannot encode artists: {e}")))?;
        let now = i64::try_from(unix_ms()?)
            .map_err(|_| PresetError::Config("system clock is out of range".into()))?;

        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO presets (track_uri, title, artists, cover_url, duration_ms,
                                  start_ms, skip_ms, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(track_uri) DO UPDATE SET
                 title = excluded.title,
                 artists = excluded.artists,
                 cover_url = excluded.cover_url,
                 duration_ms = excluded.duration_ms,
                 start_ms = excluded.start_ms,
                 skip_ms = excluded.skip_ms,
                 updated_at = excluded.updated_at",
            params![
                input.track_uri,
                input.title,
                artists_json,
                input.cover_url,
                input.duration_ms as i64,
                input.start_ms as i64,
                input.skip_ms as i64,
                now,
            ],
        )?;
        let saved = query_one(&tx, &input.track_uri)?
            .ok_or_else(|| PresetError::Db("saved preset not found after write".into()))?;
        tx.commit()?;
        Ok(saved)
    }

    /// Read one preset by track URI.
    pub fn get(&self, track_uri: &str) -> Result<Option<Preset>, PresetError> {
        let guard = self.lock()?;
        query_one(&guard, track_uri)
    }

    /// All presets, newest first (by creation time).
    pub fn list(&self) -> Result<Vec<Preset>, PresetError> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(
            "SELECT track_uri, title, artists, cover_url, duration_ms,
                    start_ms, skip_ms, created_at, updated_at
             FROM presets
             ORDER BY created_at DESC, track_uri ASC",
        )?;
        let raws = stmt
            .query_map([], row_to_raw)?
            .collect::<Result<Vec<_>, _>>()?;
        raws.into_iter().map(raw_to_preset).collect()
    }

    /// Delete a preset (idempotent — deleting a missing row is fine).
    pub fn delete(&self, track_uri: &str) -> Result<(), PresetError> {
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        tx.execute("DELETE FROM presets WHERE track_uri = ?1", [track_uri])?;
        tx.commit()?;
        Ok(())
    }

    /// Record one listening-insights event and upsert its track snapshot in a
    /// single transaction (M9). Append-only: the event row is only ever added,
    /// never updated or pruned. Presets are never touched here.
    pub fn record_event(&self, w: &InsightWrite) -> Result<(), PresetError> {
        let artists_json = serde_json::to_string(&w.artists)
            .map_err(|e| PresetError::Db(format!("cannot encode artists: {e}")))?;
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO tracks (uri, title, artists, cover_url, duration_ms, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(uri) DO UPDATE SET
                 title = excluded.title,
                 artists = excluded.artists,
                 cover_url = excluded.cover_url,
                 duration_ms = excluded.duration_ms,
                 last_seen = excluded.last_seen",
            params![
                w.track_uri,
                w.title,
                artists_json,
                w.cover_url,
                w.duration_ms as i64,
                w.created_at as i64,
            ],
        )?;
        tx.execute(
            "INSERT INTO listening_events
                 (track_uri, type, from_ms, to_ms, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                w.track_uri,
                w.kind,
                w.from_ms as i64,
                w.to_ms.map(|v| v as i64),
                w.duration_ms as i64,
                w.created_at as i64,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Number of recorded insight events (the settings "N events collected").
    pub fn insights_count(&self) -> Result<u64, PresetError> {
        let guard = self.lock()?;
        let count: i64 = guard.query_row("SELECT COUNT(*) FROM listening_events", [], |row| {
            row.get(0)
        })?;
        u64::try_from(count).map_err(|_| PresetError::Db("negative event count".into()))
    }

    /// Empty the insights tables AND everything derived from them in one
    /// transaction (the settings "delete all" action): the raw events, the
    /// track snapshots, the derived suggestions (including any applied
    /// auto-skip flags) and the analysis cursor. Presets are deliberately left
    /// untouched — a preset created by accepting a suggestion is a normal
    /// preset and survives. This is the ONLY code path that deletes insights
    /// data — Cued never prunes on its own.
    pub fn delete_all_insights(&self) -> Result<(), PresetError> {
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        tx.execute("DELETE FROM listening_events", [])?;
        tx.execute("DELETE FROM tracks", [])?;
        tx.execute("DELETE FROM suggestions", [])?;
        tx.execute("DELETE FROM analysis_state", [])?;
        tx.commit()?;
        Ok(())
    }

    // -- M10: analysis inputs -------------------------------------------------

    /// Track URIs with at least one event newer than the analysis cursor, plus
    /// the new cursor value (the current max event id). Bounds each analysis
    /// run to only the tracks that actually changed since the last run.
    pub fn tracks_with_new_events(&self, since_id: i64) -> Result<(Vec<String>, i64), PresetError> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(
            "SELECT DISTINCT track_uri FROM listening_events
             WHERE id > ?1 ORDER BY track_uri",
        )?;
        let uris = stmt
            .query_map([since_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let max_id: i64 = guard.query_row(
            "SELECT COALESCE(MAX(id), ?1) FROM listening_events",
            [since_id],
            |r| r.get(0),
        )?;
        Ok((uris, max_id))
    }

    /// The stored analysis cursor (highest event id already analyzed); 0 when
    /// nothing has been analyzed yet.
    pub fn analysis_cursor(&self) -> Result<i64, PresetError> {
        let guard = self.lock()?;
        let value: Option<i64> = guard
            .query_row(
                "SELECT value FROM analysis_state WHERE key = ?1",
                [ANALYSIS_CURSOR_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or(0))
    }

    /// Persist the analysis cursor.
    pub fn set_analysis_cursor(&self, value: i64) -> Result<(), PresetError> {
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO analysis_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![ANALYSIS_CURSOR_KEY, value],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// All recorded events for one track, oldest first, as the pure analysis
    /// engine consumes them. Rows with an unrecognized `type` are skipped.
    pub fn events_for_track(&self, track_uri: &str) -> Result<Vec<Event>, PresetError> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(
            "SELECT type, from_ms, to_ms, created_at FROM listening_events
             WHERE track_uri = ?1 ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map([track_uri], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, from_ms, to_ms, created_at)| {
                Some(Event {
                    kind: event_kind_from_str(&kind)?,
                    from_ms: from_ms.max(0) as u64,
                    to_ms: to_ms.map(|v| v.max(0) as u64),
                    created_at: created_at.max(0) as u64,
                })
            })
            .collect())
    }

    /// A track's snapshot duration (from the insights `tracks` table), needed
    /// to analyze it. `None` when the track has never been recorded.
    pub fn track_duration_ms(&self, track_uri: &str) -> Result<Option<u64>, PresetError> {
        let guard = self.lock()?;
        let value: Option<i64> = guard
            .query_row(
                "SELECT duration_ms FROM tracks WHERE uri = ?1",
                [track_uri],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.map(|v| v.max(0) as u64))
    }

    // -- M10: suggestion state ------------------------------------------------

    /// Reconcile a track's suggestions with a fresh analysis result, all in one
    /// transaction: (re)publish each supported type as active (unless the row
    /// is dismissed — never resurrected — or applied/retired — preserved, only
    /// values refreshed), and drop any ACTIVE row whose type is no longer
    /// supported. Applied/dismissed/retired rows of unsupported types are left
    /// alone (an applied auto-skip persists until the user reverses it).
    pub fn refresh_track_suggestions(
        &self,
        track_uri: &str,
        computed: &[Suggestion],
        now_ms: u64,
    ) -> Result<(), PresetError> {
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        for kind in [
            SuggestionType::SkipPoint,
            SuggestionType::StartPoint,
            SuggestionType::AutoSkip,
        ] {
            match computed.iter().find(|s| s.suggestion_type == kind) {
                Some(s) => upsert_suggestion(&tx, track_uri, s, now_ms)?,
                None => {
                    tx.execute(
                        "DELETE FROM suggestions
                         WHERE track_uri = ?1 AND type = ?2 AND status = 'active'",
                        params![track_uri, kind.as_str()],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every non-dismissed suggestion for one track (Now Playing card input).
    pub fn suggestions_for_track(
        &self,
        track_uri: &str,
    ) -> Result<Vec<StoredSuggestion>, PresetError> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(&format!(
            "{SUGGESTION_SELECT} WHERE s.track_uri = ?1 AND s.status != 'dismissed'
             ORDER BY s.type"
        ))?;
        let raws = stmt
            .query_map([track_uri], row_to_raw_suggestion)?
            .collect::<Result<Vec<_>, _>>()?;
        raws.into_iter().map(raw_to_suggestion).collect()
    }

    /// Every non-dismissed suggestion across all tracks, newest first (the
    /// Library "Suggestions (n)" section).
    pub fn list_suggestions(&self) -> Result<Vec<StoredSuggestion>, PresetError> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(&format!(
            "{SUGGESTION_SELECT} WHERE s.status != 'dismissed'
             ORDER BY s.updated_at DESC, s.track_uri ASC, s.type ASC"
        ))?;
        let raws = stmt
            .query_map([], row_to_raw_suggestion)?
            .collect::<Result<Vec<_>, _>>()?;
        raws.into_iter().map(raw_to_suggestion).collect()
    }

    /// Set the lifecycle status of one suggestion (dismiss / apply / undo).
    /// A missing row is a no-op — the suggestion may have been re-analyzed
    /// away; the caller's intent (e.g. "don't apply") still holds.
    pub fn set_suggestion_status(
        &self,
        track_uri: &str,
        kind: SuggestionType,
        status: Status,
        now_ms: u64,
    ) -> Result<(), PresetError> {
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        tx.execute(
            "UPDATE suggestions SET status = ?3, updated_at = ?4
             WHERE track_uri = ?1 AND type = ?2",
            params![track_uri, kind.as_str(), status.as_str(), now_ms as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Record that the proactive card for one suggestion was shown and then
    /// ignored: bumps the counter and retires it once it hits the cap. Only an
    /// active suggestion is affected (see [`suggestions::status_after_ignore`]).
    pub fn ignore_suggestion(
        &self,
        track_uri: &str,
        kind: SuggestionType,
        now_ms: u64,
    ) -> Result<(), PresetError> {
        let mut guard = self.lock()?;
        let tx = guard.transaction()?;
        let current: Option<(String, i64)> = tx
            .query_row(
                "SELECT status, shown_count FROM suggestions
                 WHERE track_uri = ?1 AND type = ?2",
                params![track_uri, kind.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((status_str, shown)) = current {
            if let Some(status) = Status::from_str(&status_str) {
                let (next_status, next_shown) =
                    suggestions::status_after_ignore(status, shown.max(0) as u32);
                tx.execute(
                    "UPDATE suggestions SET status = ?3, shown_count = ?4, updated_at = ?5
                     WHERE track_uri = ?1 AND type = ?2",
                    params![
                        track_uri,
                        kind.as_str(),
                        next_status.as_str(),
                        next_shown as i64,
                        now_ms as i64
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether this track has an APPLIED whole-song auto-skip (the engine's
    /// hot-path lookup — a track flagged here is skipped on sight).
    pub fn is_auto_skip_applied(&self, track_uri: &str) -> Result<bool, PresetError> {
        let guard = self.lock()?;
        let n: i64 = guard.query_row(
            "SELECT COUNT(*) FROM suggestions
             WHERE track_uri = ?1 AND type = 'auto_skip' AND status = 'applied'",
            [track_uri],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

fn open_and_migrate(path: &Path) -> Result<Connection, PresetError> {
    let conn = Connection::open(path)?;
    // Surfaces corruption of an existing file right at startup instead of on
    // the first query; near-free on a database this small.
    let check: String = conn.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(PresetError::Corrupt);
    }
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    match version {
        0 => {
            // Fresh database: create every table at the current version.
            conn.execute_batch(&format!(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS presets (
                     track_uri   TEXT PRIMARY KEY,
                     title       TEXT NOT NULL,
                     artists     TEXT NOT NULL, -- JSON array of strings
                     cover_url   TEXT,
                     duration_ms INTEGER NOT NULL,
                     start_ms    INTEGER NOT NULL,
                     skip_ms     INTEGER NOT NULL,
                     created_at  INTEGER NOT NULL,
                     updated_at  INTEGER NOT NULL
                 );
                 {INSIGHTS_SCHEMA}
                 {SUGGESTIONS_SCHEMA}
                 PRAGMA user_version = {SCHEMA_VERSION};
                 COMMIT;"
            ))?;
            Ok(conn)
        }
        1 => {
            // v1→v3 (M9 + M10): add the insights AND suggestion tables next to
            // the existing presets, which are left completely untouched.
            conn.execute_batch(&format!(
                "BEGIN;
                 {INSIGHTS_SCHEMA}
                 {SUGGESTIONS_SCHEMA}
                 PRAGMA user_version = {SCHEMA_VERSION};
                 COMMIT;"
            ))?;
            Ok(conn)
        }
        2 => {
            // v2→v3 (M10): add the derived-suggestion tables. Presets and the
            // collected insights are left completely untouched.
            conn.execute_batch(&format!(
                "BEGIN;
                 {SUGGESTIONS_SCHEMA}
                 PRAGMA user_version = {SCHEMA_VERSION};
                 COMMIT;"
            ))?;
            Ok(conn)
        }
        SCHEMA_VERSION => Ok(conn),
        _ => Err(PresetError::SchemaTooNew),
    }
}

/// Rename a corrupt database file out of the way so a fresh one can be
/// created; the user's data is kept on disk for manual inspection.
fn set_aside_corrupt_file(path: &Path) -> Result<(), PresetError> {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or(DB_FILE);
    let target = path.with_file_name(format!("{file_name}.corrupt-{}", unix_ms()?));
    std::fs::rename(path, &target)
        .map_err(|e| PresetError::Config(format!("cannot set aside the corrupt database: {e}")))
}

/// Row shape as read from SQLite, before JSON/sign conversion.
struct RawRow {
    track_uri: String,
    title: String,
    artists_json: String,
    cover_url: Option<String>,
    duration_ms: i64,
    start_ms: i64,
    skip_ms: i64,
    created_at: i64,
    updated_at: i64,
}

fn row_to_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        track_uri: row.get(0)?,
        title: row.get(1)?,
        artists_json: row.get(2)?,
        cover_url: row.get(3)?,
        duration_ms: row.get(4)?,
        start_ms: row.get(5)?,
        skip_ms: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn raw_to_preset(raw: RawRow) -> Result<Preset, PresetError> {
    let to_u64 = |v: i64, field: &str| {
        u64::try_from(v).map_err(|_| PresetError::Db(format!("negative {field} in database")))
    };
    Ok(Preset {
        artists: serde_json::from_str(&raw.artists_json)
            .map_err(|e| PresetError::Db(format!("corrupt artists column: {e}")))?,
        track_uri: raw.track_uri,
        title: raw.title,
        cover_url: raw.cover_url,
        duration_ms: to_u64(raw.duration_ms, "duration")?,
        start_ms: to_u64(raw.start_ms, "start")?,
        skip_ms: to_u64(raw.skip_ms, "skip")?,
        created_at: to_u64(raw.created_at, "created_at")?,
        updated_at: to_u64(raw.updated_at, "updated_at")?,
    })
}

fn query_one(conn: &Connection, track_uri: &str) -> Result<Option<Preset>, PresetError> {
    conn.query_row(
        "SELECT track_uri, title, artists, cover_url, duration_ms,
                start_ms, skip_ms, created_at, updated_at
         FROM presets WHERE track_uri = ?1",
        [track_uri],
        row_to_raw,
    )
    .optional()?
    .map(raw_to_preset)
    .transpose()
}

fn unix_ms() -> Result<u64, PresetError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|_| PresetError::Config("system clock is set before 1970".into()))
}

/// Map a stored `type` string to the pure engine's [`EventKind`].
fn event_kind_from_str(kind: &str) -> Option<EventKind> {
    match kind {
        "seek_forward" => Some(EventKind::SeekForward),
        "seek_back" => Some(EventKind::SeekBack),
        "skip_next" => Some(EventKind::SkipNext),
        _ => None,
    }
}

/// Shared column list + join for reading suggestions with their track's
/// display metadata.
const SUGGESTION_SELECT: &str = "
    SELECT s.track_uri, s.type, s.status, s.shown_count, s.value_start_ms,
           s.value_end_ms, s.plays_total, s.plays_matching, s.updated_at,
           t.title, t.artists, t.cover_url, t.duration_ms
    FROM suggestions s JOIN tracks t ON t.uri = s.track_uri";

/// A suggestion row as read from SQLite, before JSON/sign conversion.
struct RawSuggestion {
    track_uri: String,
    kind: String,
    status: String,
    shown_count: i64,
    value_start_ms: Option<i64>,
    value_end_ms: Option<i64>,
    plays_total: i64,
    plays_matching: i64,
    updated_at: i64,
    title: String,
    artists_json: String,
    cover_url: Option<String>,
    duration_ms: i64,
}

fn row_to_raw_suggestion(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawSuggestion> {
    Ok(RawSuggestion {
        track_uri: row.get(0)?,
        kind: row.get(1)?,
        status: row.get(2)?,
        shown_count: row.get(3)?,
        value_start_ms: row.get(4)?,
        value_end_ms: row.get(5)?,
        plays_total: row.get(6)?,
        plays_matching: row.get(7)?,
        updated_at: row.get(8)?,
        title: row.get(9)?,
        artists_json: row.get(10)?,
        cover_url: row.get(11)?,
        duration_ms: row.get(12)?,
    })
}

fn raw_to_suggestion(raw: RawSuggestion) -> Result<StoredSuggestion, PresetError> {
    let clamp = |v: i64| v.max(0) as u64;
    Ok(StoredSuggestion {
        artists: serde_json::from_str(&raw.artists_json)
            .map_err(|e| PresetError::Db(format!("corrupt artists column: {e}")))?,
        track_uri: raw.track_uri,
        kind: raw.kind,
        status: raw.status,
        shown_count: clamp(raw.shown_count) as u32,
        value_start_ms: raw.value_start_ms.map(clamp),
        value_end_ms: raw.value_end_ms.map(clamp),
        plays_total: clamp(raw.plays_total) as u32,
        plays_matching: clamp(raw.plays_matching) as u32,
        updated_at: clamp(raw.updated_at),
        title: raw.title,
        cover_url: raw.cover_url,
        duration_ms: clamp(raw.duration_ms),
    })
}

/// Upsert one analyzed suggestion, honoring the lifecycle rules: a dismissed
/// row is frozen (never touched); an applied/retired row keeps its status but
/// gets fresh values; anything else is (re)published as active. `shown_count`
/// is preserved across updates (only ever changed by ignore/analysis-insert).
fn upsert_suggestion(
    tx: &rusqlite::Transaction<'_>,
    track_uri: &str,
    s: &Suggestion,
    now_ms: u64,
) -> Result<(), PresetError> {
    let kind = s.suggestion_type.as_str();
    let existing: Option<String> = tx
        .query_row(
            "SELECT status FROM suggestions WHERE track_uri = ?1 AND type = ?2",
            params![track_uri, kind],
            |r| r.get(0),
        )
        .optional()?;
    let existing_status = existing.as_deref().and_then(Status::from_str);
    if !suggestions::analysis_may_update(existing_status) {
        return Ok(()); // dismissed — frozen forever
    }
    let new_status = suggestions::status_after_analysis(existing_status);
    tx.execute(
        "INSERT INTO suggestions
             (track_uri, type, status, shown_count, value_start_ms, value_end_ms,
              plays_total, plays_matching, updated_at)
         VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(track_uri, type) DO UPDATE SET
             status = excluded.status,
             value_start_ms = excluded.value_start_ms,
             value_end_ms = excluded.value_end_ms,
             plays_total = excluded.plays_total,
             plays_matching = excluded.plays_matching,
             updated_at = excluded.updated_at",
        params![
            track_uri,
            kind,
            new_status.as_str(),
            s.value_start_ms.map(|v| v as i64),
            s.value_end_ms.map(|v| v as i64),
            s.plays_total as i64,
            s.plays_matching as i64,
            now_ms as i64,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Unique per-test scratch dir; removed on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("cued-presets-test-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        fn db(&self) -> std::path::PathBuf {
            self.0.join(DB_FILE)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn input(uri: &str, start_ms: u64, skip_ms: u64) -> PresetInput {
        PresetInput {
            track_uri: uri.into(),
            title: "Song \"Ünïcode\" — test".into(),
            artists: vec!["Artist A".into(), "Ünïcode & \"Quotes\"".into()],
            cover_url: Some("https://i.scdn.co/image/abc".into()),
            duration_ms: 200_000,
            start_ms,
            skip_ms,
        }
    }

    // -- validation -----------------------------------------------------------

    #[test]
    fn accepts_a_typical_preset_and_the_exact_10s_gap() {
        assert!(validate_times(5_000, 180_000, 200_000).is_ok());
        assert!(validate_times(0, MIN_GAP_MS, 200_000).is_ok());
        // skip == duration ("play to the end") is allowed once start moved
        assert!(validate_times(1_000, 200_000, 200_000).is_ok());
    }

    #[test]
    fn rejects_the_neutral_state_but_accepts_the_boundaries_next_to_it() {
        // start 0 + skip == duration is "nothing set" — never stored (M8).
        assert!(matches!(
            validate_times(0, 200_000, 200_000),
            Err(PresetError::Validation(_))
        ));
        // One ms short of the end IS a real preset…
        assert!(validate_times(0, 199_999, 200_000).is_ok());
        // …as is a start-only preset that plays to the end.
        assert!(validate_times(1, 200_000, 200_000).is_ok());
    }

    #[test]
    fn a_neutral_write_is_rejected_and_nothing_is_stored() {
        let dir = TempDir::new("neutral");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        let err = store
            .save(&input("spotify:track:a", 0, 200_000))
            .expect_err("neutral must be rejected");
        assert!(matches!(err, PresetError::Validation(_)));
        assert!(store.get("spotify:track:a").expect("get").is_none());
    }

    #[test]
    fn rejects_a_gap_below_the_minimum() {
        let err = validate_times(0, MIN_GAP_MS - 1, 200_000).expect_err("gap too small");
        assert!(matches!(err, PresetError::Validation(_)));
    }

    #[test]
    fn rejects_start_at_or_after_skip() {
        assert!(validate_times(50_000, 50_000, 200_000).is_err());
        assert!(validate_times(60_000, 50_000, 200_000).is_err());
    }

    #[test]
    fn rejects_skip_beyond_the_duration() {
        assert!(validate_times(0, 200_001, 200_000).is_err());
    }

    #[test]
    fn rejects_an_empty_or_oversized_track_uri() {
        let mut bad = input("", 0, 20_000);
        assert!(matches!(
            validate_input(&bad),
            Err(PresetError::Validation(_))
        ));
        bad.track_uri = "x".repeat(MAX_URI_LEN + 1);
        assert!(validate_input(&bad).is_err());
    }

    #[test]
    fn rejects_an_absurd_duration() {
        let mut bad = input("spotify:track:a", 0, 20_000);
        bad.duration_ms = MAX_DURATION_MS + 1;
        assert!(validate_input(&bad).is_err());
    }

    // -- round trip -----------------------------------------------------------

    #[test]
    fn save_then_get_roundtrips_all_fields() {
        let dir = TempDir::new("roundtrip");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        let saved = store
            .save(&input("spotify:track:a", 5_000, 180_000))
            .expect("save");
        assert_eq!(saved.track_uri, "spotify:track:a");
        assert_eq!(saved.start_ms, 5_000);
        assert_eq!(saved.skip_ms, 180_000);
        assert_eq!(saved.artists[1], "Ünïcode & \"Quotes\"");
        assert!(saved.created_at > 0);
        assert_eq!(saved.created_at, saved.updated_at);

        let loaded = store.get("spotify:track:a").expect("get").expect("some");
        assert_eq!(loaded, saved);
    }

    #[test]
    fn update_overwrites_times_but_preserves_created_at() {
        let dir = TempDir::new("update");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        let first = store
            .save(&input("spotify:track:a", 0, 20_000))
            .expect("save");
        std::thread::sleep(Duration::from_millis(5));
        let second = store
            .save(&input("spotify:track:a", 7_000, 30_000))
            .expect("update");
        assert_eq!(second.created_at, first.created_at);
        assert!(second.updated_at > first.updated_at);
        assert_eq!(second.start_ms, 7_000);
        assert_eq!(second.skip_ms, 30_000);
        assert_eq!(store.list().expect("list").len(), 1);
    }

    #[test]
    fn delete_removes_the_row_and_is_idempotent() {
        let dir = TempDir::new("delete");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        store
            .save(&input("spotify:track:a", 0, 20_000))
            .expect("save");
        store.delete("spotify:track:a").expect("delete");
        assert!(store.get("spotify:track:a").expect("get").is_none());
        store
            .delete("spotify:track:a")
            .expect("second delete is fine");
    }

    #[test]
    fn list_returns_newest_first() {
        let dir = TempDir::new("list-order");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        store
            .save(&input("spotify:track:older", 0, 20_000))
            .expect("save older");
        std::thread::sleep(Duration::from_millis(5));
        store
            .save(&input("spotify:track:newer", 0, 20_000))
            .expect("save newer");
        let uris: Vec<String> = store
            .list()
            .expect("list")
            .into_iter()
            .map(|p| p.track_uri)
            .collect();
        assert_eq!(uris, vec!["spotify:track:newer", "spotify:track:older"]);
    }

    #[test]
    fn invalid_times_are_rejected_and_nothing_is_written() {
        let dir = TempDir::new("reject");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        let err = store
            .save(&input("spotify:track:a", 0, 9_999))
            .expect_err("must reject");
        assert!(matches!(err, PresetError::Validation(_)));
        assert!(store.get("spotify:track:a").expect("get").is_none());
    }

    #[test]
    fn presets_survive_a_store_reopen() {
        let dir = TempDir::new("reopen");
        {
            let store = PresetStore::open_at(&dir.db()).expect("open");
            store
                .save(&input("spotify:track:a", 5_000, 180_000))
                .expect("save");
        }
        let store = PresetStore::open_at(&dir.db()).expect("reopen");
        assert!(!store.recovered());
        let loaded = store.get("spotify:track:a").expect("get").expect("some");
        assert_eq!(loaded.start_ms, 5_000);
    }

    // -- recovery ---------------------------------------------------------------

    #[test]
    fn a_missing_file_is_created_fresh_without_a_rescue() {
        let dir = TempDir::new("missing");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        assert!(!store.recovered());
        assert!(dir.db().exists());
        store
            .save(&input("spotify:track:a", 0, 20_000))
            .expect("save works on a fresh db");
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_recreated() {
        let dir = TempDir::new("corrupt");
        std::fs::write(dir.db(), vec![0xAB; 4096]).expect("write garbage");

        let store = PresetStore::open_at(&dir.db()).expect("rescue must succeed");
        assert!(store.recovered());
        store
            .save(&input("spotify:track:a", 0, 20_000))
            .expect("fresh db works");

        let set_aside: Vec<String> = std::fs::read_dir(&dir.0)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.contains(".corrupt-"))
            .collect();
        assert_eq!(
            set_aside.len(),
            1,
            "exactly one set-aside file: {set_aside:?}"
        );
    }

    #[test]
    fn a_newer_schema_is_an_error_not_a_rescue() {
        let dir = TempDir::new("newer-schema");
        {
            let _ = PresetStore::open_at(&dir.db()).expect("create");
        }
        {
            let conn = Connection::open(dir.db()).expect("raw open");
            conn.pragma_update(None, "user_version", 99)
                .expect("bump version");
        }
        let err = PresetStore::open_at(&dir.db()).expect_err("must refuse");
        assert!(matches!(err, PresetError::SchemaTooNew));
        assert!(
            dir.db().exists(),
            "a future schema must never be renamed away"
        );
    }

    // -- M9: listening insights -------------------------------------------------

    fn event(uri: &str, kind: &'static str, from_ms: u64, to_ms: Option<u64>) -> InsightWrite {
        InsightWrite {
            track_uri: uri.into(),
            kind,
            from_ms,
            to_ms,
            duration_ms: 200_000,
            title: "Song \"Ünïcode\"".into(),
            artists: vec!["Artist A".into(), "Ünïcode & \"Quotes\"".into()],
            cover_url: Some("https://i.scdn.co/image/abc".into()),
            created_at: 1_700_000_000_000,
        }
    }

    /// Read a track row's stored fields (title, artists JSON, last_seen).
    fn track_row(store: &PresetStore, uri: &str) -> Option<(String, String, i64)> {
        let guard = store.lock().expect("lock");
        guard
            .query_row(
                "SELECT title, artists, last_seen FROM tracks WHERE uri = ?1",
                [uri],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .expect("query")
    }

    #[test]
    fn record_event_inserts_the_event_and_upserts_the_track() {
        let dir = TempDir::new("insights-record");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        assert_eq!(store.insights_count().expect("count"), 0);

        store
            .record_event(&event(
                "spotify:track:a",
                "seek_forward",
                12_000,
                Some(48_000),
            ))
            .expect("record");
        assert_eq!(store.insights_count().expect("count"), 1);

        let (title, artists_json, _) = track_row(&store, "spotify:track:a").expect("track row");
        assert!(title.contains("Ünïcode"));
        let artists: Vec<String> = serde_json::from_str(&artists_json).expect("artists json");
        assert_eq!(artists[1], "Ünïcode & \"Quotes\"");
    }

    #[test]
    fn storage_is_append_only_and_never_deduplicates_events() {
        // Two events for the SAME track: the tracks row is upserted (one row,
        // refreshed last_seen), but BOTH events are kept — append-only.
        let dir = TempDir::new("insights-append");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        let mut first = event("spotify:track:a", "seek_back", 90_000, Some(30_000));
        first.created_at = 1_000;
        let mut second = event("spotify:track:a", "skip_next", 150_000, None);
        second.created_at = 2_000;
        store.record_event(&first).expect("first");
        store.record_event(&second).expect("second");

        assert_eq!(store.insights_count().expect("count"), 2);
        let (_, _, last_seen) = track_row(&store, "spotify:track:a").expect("one track row");
        assert_eq!(last_seen, 2_000, "last_seen tracks the newest event");
    }

    #[test]
    fn to_ms_is_stored_null_for_skip_next() {
        let dir = TempDir::new("insights-null");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        store
            .record_event(&event("spotify:track:a", "skip_next", 42_000, None))
            .expect("record");
        let guard = store.lock().expect("lock");
        let to_ms: Option<i64> = guard
            .query_row("SELECT to_ms FROM listening_events LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("query");
        assert_eq!(to_ms, None);
    }

    #[test]
    fn delete_all_insights_empties_both_tables_but_keeps_presets() {
        let dir = TempDir::new("insights-delete");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        store
            .save(&input("spotify:track:preset", 5_000, 180_000))
            .expect("save preset");
        store
            .record_event(&event(
                "spotify:track:a",
                "seek_forward",
                10_000,
                Some(40_000),
            ))
            .expect("record a");
        store
            .record_event(&event("spotify:track:b", "skip_next", 150_000, None))
            .expect("record b");
        assert_eq!(store.insights_count().expect("count"), 2);

        store.delete_all_insights().expect("delete all");

        assert_eq!(store.insights_count().expect("count"), 0);
        assert!(track_row(&store, "spotify:track:a").is_none());
        // Presets are deliberately untouched by the insights wipe.
        assert!(store
            .get("spotify:track:preset")
            .expect("get preset")
            .is_some());
    }

    #[test]
    fn a_v1_database_migrates_to_v2_preserving_presets() {
        let dir = TempDir::new("insights-migrate");
        // Build a genuine v1 database by hand: presets table only, a stored
        // row, user_version = 1 (M9 predates the insights tables).
        {
            let conn = Connection::open(dir.db()).expect("raw open");
            conn.execute_batch(
                "CREATE TABLE presets (
                     track_uri   TEXT PRIMARY KEY,
                     title       TEXT NOT NULL,
                     artists     TEXT NOT NULL,
                     cover_url   TEXT,
                     duration_ms INTEGER NOT NULL,
                     start_ms    INTEGER NOT NULL,
                     skip_ms     INTEGER NOT NULL,
                     created_at  INTEGER NOT NULL,
                     updated_at  INTEGER NOT NULL
                 );",
            )
            .expect("create v1 schema");
            conn.execute(
                "INSERT INTO presets VALUES
                     ('spotify:track:old', 'Legacy', '[\"A\"]', NULL, 200000, 5000, 180000, 1, 1)",
                [],
            )
            .expect("insert legacy preset");
            conn.pragma_update(None, "user_version", 1).expect("set v1");
        }

        let store = PresetStore::open_at(&dir.db()).expect("migrate");
        assert!(!store.recovered(), "a valid v1 db is migrated, not rescued");

        // The pre-existing preset survives the migration untouched.
        let preset = store
            .get("spotify:track:old")
            .expect("get")
            .expect("preset preserved");
        assert_eq!(preset.title, "Legacy");
        assert_eq!(preset.start_ms, 5_000);

        // The new insights tables exist and are usable + empty.
        assert_eq!(store.insights_count().expect("count"), 0);
        store
            .record_event(&event(
                "spotify:track:a",
                "seek_forward",
                1_000,
                Some(20_000),
            ))
            .expect("insights table works after migration");
        assert_eq!(store.insights_count().expect("count"), 1);
    }

    // -- M10: suggestions -------------------------------------------------------

    fn sugg(
        kind: SuggestionType,
        value_start_ms: Option<u64>,
        value_end_ms: Option<u64>,
        plays_total: u32,
        plays_matching: u32,
    ) -> Suggestion {
        Suggestion {
            suggestion_type: kind,
            value_start_ms,
            value_end_ms,
            plays_total,
            plays_matching,
        }
    }

    /// Seed a track row (suggestion reads join `tracks`), the way a recorded
    /// event would in the real flow.
    fn seed_track(store: &PresetStore, uri: &str) {
        store
            .record_event(&event(uri, "skip_next", 90_000, None))
            .expect("seed track");
    }

    fn status_of(store: &PresetStore, uri: &str, kind: SuggestionType) -> Option<String> {
        store
            .suggestions_for_track(uri)
            .expect("for track")
            .into_iter()
            .find(|s| s.kind == kind.as_str())
            .map(|s| s.status)
    }

    #[test]
    fn a_v2_database_migrates_to_v3_preserving_insights_and_presets() {
        let dir = TempDir::new("migrate-v2-v3");
        // Build a genuine v2 database by hand: presets + insights tables, a
        // stored preset AND a recorded event, user_version = 2.
        {
            let conn = Connection::open(dir.db()).expect("raw open");
            conn.execute_batch(&format!(
                "CREATE TABLE presets (
                     track_uri TEXT PRIMARY KEY, title TEXT NOT NULL,
                     artists TEXT NOT NULL, cover_url TEXT, duration_ms INTEGER NOT NULL,
                     start_ms INTEGER NOT NULL, skip_ms INTEGER NOT NULL,
                     created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
                 );
                 {INSIGHTS_SCHEMA}"
            ))
            .expect("create v2 schema");
            conn.execute(
                "INSERT INTO presets VALUES
                     ('spotify:track:old', 'Legacy', '[\"A\"]', NULL, 200000, 5000, 180000, 1, 1)",
                [],
            )
            .expect("insert legacy preset");
            conn.execute(
                "INSERT INTO tracks VALUES
                     ('spotify:track:old', 'Legacy', '[\"A\"]', NULL, 200000, 5)",
                [],
            )
            .expect("insert legacy track");
            conn.execute(
                "INSERT INTO listening_events
                     (track_uri, type, from_ms, to_ms, duration_ms, created_at)
                 VALUES ('spotify:track:old', 'skip_next', 90000, NULL, 200000, 5)",
                [],
            )
            .expect("insert legacy event");
            conn.pragma_update(None, "user_version", 2).expect("set v2");
        }

        let store = PresetStore::open_at(&dir.db()).expect("migrate");
        assert!(!store.recovered(), "a valid v2 db is migrated, not rescued");
        // Preset AND collected insights both survive the migration.
        assert_eq!(
            store
                .get("spotify:track:old")
                .expect("get")
                .expect("some")
                .start_ms,
            5_000
        );
        assert_eq!(store.insights_count().expect("count"), 1);
        // The new suggestions table exists and is usable.
        store
            .refresh_track_suggestions(
                "spotify:track:old",
                &[sugg(
                    SuggestionType::SkipPoint,
                    Some(90_000),
                    Some(90_000),
                    6,
                    6,
                )],
                100,
            )
            .expect("suggestions table works after migration");
        assert_eq!(store.list_suggestions().expect("list").len(), 1);
    }

    #[test]
    fn refresh_publishes_active_suggestions_then_reconciles_them_away() {
        let dir = TempDir::new("sugg-refresh");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        seed_track(&store, "spotify:track:a");

        store
            .refresh_track_suggestions(
                "spotify:track:a",
                &[sugg(
                    SuggestionType::SkipPoint,
                    Some(70_000),
                    Some(74_000),
                    6,
                    6,
                )],
                10,
            )
            .expect("refresh");
        let one = store
            .suggestions_for_track("spotify:track:a")
            .expect("for track");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].kind, "skip_point");
        assert_eq!(one[0].status, "active");
        assert_eq!(one[0].value_start_ms, Some(70_000));
        assert_eq!(one[0].value_end_ms, Some(74_000));
        assert_eq!((one[0].plays_total, one[0].plays_matching), (6, 6));

        // The pattern weakened: an empty re-analysis drops the active row.
        store
            .refresh_track_suggestions("spotify:track:a", &[], 20)
            .expect("reconcile");
        assert!(store
            .suggestions_for_track("spotify:track:a")
            .expect("for track")
            .is_empty());
    }

    #[test]
    fn a_dismissed_suggestion_is_never_resurrected_by_reanalysis() {
        let dir = TempDir::new("sugg-dismiss");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        seed_track(&store, "spotify:track:a");
        let one = [sugg(
            SuggestionType::SkipPoint,
            Some(70_000),
            Some(74_000),
            6,
            6,
        )];

        store
            .refresh_track_suggestions("spotify:track:a", &one, 10)
            .expect("refresh");
        store
            .set_suggestion_status(
                "spotify:track:a",
                SuggestionType::SkipPoint,
                Status::Dismissed,
                11,
            )
            .expect("dismiss");
        // Dismissed rows never appear on a surface…
        assert!(store
            .suggestions_for_track("spotify:track:a")
            .expect("for track")
            .is_empty());
        // …and re-analysis with the SAME strong pattern must not revive it.
        store
            .refresh_track_suggestions("spotify:track:a", &one, 12)
            .expect("re-refresh");
        assert!(store
            .suggestions_for_track("spotify:track:a")
            .expect("for track")
            .is_empty());
    }

    #[test]
    fn apply_then_undo_toggles_the_auto_skip_flag() {
        let dir = TempDir::new("sugg-autoskip");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        seed_track(&store, "spotify:track:a");
        store
            .refresh_track_suggestions(
                "spotify:track:a",
                &[sugg(SuggestionType::AutoSkip, None, None, 10, 10)],
                10,
            )
            .expect("refresh");
        assert!(!store.is_auto_skip_applied("spotify:track:a").expect("q"));

        store
            .set_suggestion_status(
                "spotify:track:a",
                SuggestionType::AutoSkip,
                Status::Applied,
                11,
            )
            .expect("apply");
        assert!(store.is_auto_skip_applied("spotify:track:a").expect("q"));
        // Applied rows survive an empty re-analysis (not "active", not dropped).
        store
            .refresh_track_suggestions("spotify:track:a", &[], 12)
            .expect("reconcile");
        assert!(store.is_auto_skip_applied("spotify:track:a").expect("q"));

        // Undo returns it to active.
        store
            .set_suggestion_status(
                "spotify:track:a",
                SuggestionType::AutoSkip,
                Status::Active,
                13,
            )
            .expect("undo");
        assert!(!store.is_auto_skip_applied("spotify:track:a").expect("q"));
    }

    #[test]
    fn three_ignores_retire_the_card_but_keep_it_in_the_library() {
        let dir = TempDir::new("sugg-ignore");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        seed_track(&store, "spotify:track:a");
        store
            .refresh_track_suggestions(
                "spotify:track:a",
                &[sugg(
                    SuggestionType::SkipPoint,
                    Some(70_000),
                    Some(74_000),
                    6,
                    6,
                )],
                10,
            )
            .expect("refresh");
        for at in 11..14 {
            store
                .ignore_suggestion("spotify:track:a", SuggestionType::SkipPoint, at)
                .expect("ignore");
        }
        assert_eq!(
            status_of(&store, "spotify:track:a", SuggestionType::SkipPoint).as_deref(),
            Some("retired")
        );
        // Retired is not dismissed: still present for the Library section.
        assert_eq!(store.list_suggestions().expect("list").len(), 1);
    }

    #[test]
    fn analysis_cursor_bounds_work_to_new_tracks() {
        let dir = TempDir::new("sugg-cursor");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        assert_eq!(store.analysis_cursor().expect("cursor"), 0);

        store
            .record_event(&event("spotify:track:a", "skip_next", 90_000, None))
            .expect("event a");
        let (uris, max_id) = store.tracks_with_new_events(0).expect("new events");
        assert_eq!(uris, vec!["spotify:track:a".to_string()]);
        assert!(max_id > 0);

        store.set_analysis_cursor(max_id).expect("set cursor");
        assert_eq!(store.analysis_cursor().expect("cursor"), max_id);
        // Nothing new since the cursor.
        let (uris, _) = store.tracks_with_new_events(max_id).expect("new events");
        assert!(uris.is_empty());
    }

    #[test]
    fn events_for_track_maps_kinds_and_skips_unknown_types() {
        let dir = TempDir::new("sugg-events");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        store
            .record_event(&event(
                "spotify:track:a",
                "seek_forward",
                3_000,
                Some(30_000),
            ))
            .expect("e1");
        store
            .record_event(&event("spotify:track:a", "skip_next", 90_000, None))
            .expect("e2");
        let events = store.events_for_track("spotify:track:a").expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, EventKind::SeekForward);
        assert_eq!(events[0].to_ms, Some(30_000));
        assert_eq!(events[1].kind, EventKind::SkipNext);
        assert_eq!(events[1].to_ms, None);
        assert_eq!(
            store.track_duration_ms("spotify:track:a").expect("dur"),
            Some(200_000)
        );
    }

    #[test]
    fn delete_all_insights_also_clears_derived_suggestions() {
        let dir = TempDir::new("sugg-delete");
        let store = PresetStore::open_at(&dir.db()).expect("open");
        seed_track(&store, "spotify:track:a");
        store
            .refresh_track_suggestions(
                "spotify:track:a",
                &[sugg(SuggestionType::AutoSkip, None, None, 10, 10)],
                10,
            )
            .expect("refresh");
        store
            .set_suggestion_status(
                "spotify:track:a",
                SuggestionType::AutoSkip,
                Status::Applied,
                11,
            )
            .expect("apply");
        store.set_analysis_cursor(5).expect("cursor");

        store.delete_all_insights().expect("delete all");

        assert!(store.list_suggestions().expect("list").is_empty());
        assert!(!store.is_auto_skip_applied("spotify:track:a").expect("q"));
        assert_eq!(store.analysis_cursor().expect("cursor"), 0);
    }
}
