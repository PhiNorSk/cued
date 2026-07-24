//! Pure decision logic of the auto-skip engine (M4).
//!
//! This module holds NO I/O: the poll loop in [`crate::player`] feeds it
//! observations and executes the actions it returns. Everything here is
//! deterministic — time comes in as parameters — so the whole behavior is
//! unit-testable without a network.
//!
//! Core promises (see the M4 ticket):
//! - Never fight the user: a manual seek is respected, never corrected.
//! - Never loop: at most [`MAX_ACTIONS_PER_INSTANCE`] actions per playback
//!   instance, plus a cooldown after every action.
//! - Never act on stale data (older than [`MAX_SNAPSHOT_AGE_MS`]).

// ---------------------------------------------------------------------------
// All automation thresholds live here (single source of truth).
// ---------------------------------------------------------------------------

/// A new instance whose FIRST observation lies within this window counts as
/// "the track started from the beginning" — only then is the start jump due.
/// Must comfortably cover the poll cadence (the track may be ~1 s in before
/// the engine first sees it).
pub const START_NEAR_MS: u64 = 5_000;

/// A backwards jump landing at or below this position counts as the track
/// restarting (repeat-one or the user restarting it) — a NEW playback
/// instance, so the start jump fires again. Larger backwards jumps are
/// manual seeks and are left alone.
pub const RESTART_WINDOW_MS: u64 = 2_000;

/// A track that changes with its last observed position within this window of
/// the end counts as a NATURAL end (the song simply finished) — never a
/// user skip, so no `skip_next` insight event is recorded (M9).
pub const NATURAL_END_WINDOW_MS: u64 = 5_000;

/// Boundary one-shots fire this many ms of playback early, so the seek/skip
/// lands on the boundary despite request latency.
pub const ACTION_LEAD_MS: u64 = 300;

/// After ANY control action, further actions are suppressed for this long.
pub const ACTION_COOLDOWN_MS: u64 = 2_000;

/// An observed position deviating more than this from the extrapolation
/// interval of the previous observation counts as a manual seek.
pub const MANUAL_SEEK_JUMP_MS: u64 = 2_000;

/// Hard upper bound of control actions per playback instance — the
/// guaranteed can-never-loop backstop.
pub const MAX_ACTIONS_PER_INSTANCE: u32 = 4;

/// Never act on an observation older than this (2 poll cycles): the engine
/// might have been asleep (laptop lid) since it was taken.
pub const MAX_SNAPSHOT_AGE_MS: u64 = 2_000;

/// A start jump is attempted at most this often (first try + one retry after
/// a re-poll confirmed the state still warrants it).
pub const MAX_START_ATTEMPTS: u32 = 2;

/// Boundary one-shots are only scheduled within this horizon; anything
/// further away is re-planned on the next regular poll (which is sooner).
pub const WAKEUP_HORIZON_MS: u64 = 1_100;

// -- M7: queue prediction & transition burst ---------------------------------

/// The queue is (re-)fetched once fewer than this many ms remain to the next
/// transition boundary (skip point if our skip will fire, track end
/// otherwise) — late enough that last-minute queue edits are usually in.
pub const PREDICT_HORIZON_MS: u64 = 15_000;

/// Hard cap of queue fetches per playback instance (one on the track change
/// plus one near the boundary). Failed fetches count — never-loop bound.
pub const MAX_QUEUE_FETCHES_PER_INSTANCE: u32 = 2;

/// The natural-end transition wakeup fires this many ms before the
/// interpolated track end: the observed position lags the real one by the
/// request latency, so the real boundary arrives slightly early.
pub const TRANSITION_WAKE_LEAD_MS: u64 = 200;

/// A transition burst is at most this many fast polls…
pub const BURST_POLL_COUNT: u32 = 3;

/// …spaced this far apart…
pub const BURST_POLL_SPACING_MS: u64 = 300;

/// …and never longer than this in total (hard wall-clock bound).
pub const BURST_MAX_TOTAL_MS: u64 = 1_500;

// ---------------------------------------------------------------------------
// Inputs & outputs
// ---------------------------------------------------------------------------

/// A control action the engine must execute against the Spotify API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `PUT /v1/me/player/seek` to the preset's start point.
    SeekToStart { start_ms: u64 },
    /// `POST /v1/me/player/next`.
    SkipNext,
}

/// The preset boundaries for the current track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CuePoints {
    pub start_ms: u64,
    pub skip_ms: u64,
}

/// A pre-armed start jump for the QUEUE-predicted next track (M7). Purely a
/// hint: it only makes the engine confirm the transition faster — the seek
/// itself always comes out of the normal, observation-confirmed start-jump
/// path with every M4 rule intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prearmed {
    pub track_uri: String,
    /// The predicted track's preset start (informational / logging).
    pub start_ms: u64,
}

/// A recorded listening-insights event (M9): a genuine, user-driven
/// skip/seek classified purely from observations. The poll loop attaches the
/// track-metadata snapshot and a timestamp before persisting it; this module
/// stays free of any I/O or display metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightEvent {
    pub track_uri: String,
    pub kind: InsightKind,
    /// Where playback was (interpolated) when the user acted.
    pub from_ms: u64,
    /// Destination of a seek; `None` for `skip_next` (the track ended).
    pub to_ms: Option<u64>,
    pub duration_ms: u64,
}

/// The kind of user action an [`InsightEvent`] captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightKind {
    SeekForward,
    SeekBack,
    SkipNext,
}

impl InsightKind {
    /// The stable string persisted in the `type` column.
    pub fn as_str(self) -> &'static str {
        match self {
            InsightKind::SeekForward => "seek_forward",
            InsightKind::SeekBack => "seek_back",
            InsightKind::SkipNext => "skip_next",
        }
    }
}

/// Track-independent conditions that must ALL hold for any control call.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gates {
    /// The master toggle.
    pub automation_on: bool,
    /// Control calls require a Premium account.
    pub premium: bool,
    /// False while the active device rejected a control call (403) and has
    /// not changed since.
    pub device_ok: bool,
    /// True while the UI edits the preset of the CURRENTLY PLAYING track
    /// (M8 edit mode): automation is suspended for it, the master toggle
    /// stays untouched. The per-track comparison happens in the poll loop.
    pub edit_hold: bool,
    /// The listening-insights master toggle (M9). Orthogonal to automation:
    /// it gates ONLY event recording, never actions, so it is deliberately
    /// NOT part of [`Gates::all`]. Manual skips are recorded even with
    /// automation off — that is still genuine user behavior.
    pub insights_on: bool,
}

impl Gates {
    fn all(&self) -> bool {
        self.automation_on && self.premium && self.device_ok && !self.edit_hold
    }

    /// Whether user events may be recorded now: insights on, and NOT while the
    /// playing track's preset is being edited (edit mode is a sandbox — its
    /// preview/restore seeks and any stray manual seeks are never listening
    /// data).
    fn records_insights(&self) -> bool {
        self.insights_on && !self.edit_hold
    }
}

/// One playback observation, as far as automation is concerned.
#[derive(Debug, Clone)]
pub struct Obs {
    pub track_uri: String,
    /// Real, non-local track — episodes/local files are never controlled.
    pub controllable: bool,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// Unix ms when `position_ms` was sampled.
    pub fetched_at_ms: u64,
}

/// How the execution of an action went, as reported back by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Timeout / 5xx / other transient failure — a single retry is allowed
    /// once a re-poll confirms the state still warrants the action.
    Transient,
    /// Definitive failure (403 restricted device, rate limit) — no retry.
    Fatal,
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct LastObs {
    position_ms: u64,
    fetched_at_ms: u64,
    playing: bool,
}

/// An active transition-confirmation burst (M7): a short run of fast polls
/// around a predicted track change.
#[derive(Debug, Clone, Copy)]
struct Burst {
    polls: u32,
    started_at_ms: u64,
}

/// Automation state across polls. One value lives inside the poll loop and
/// dies with it. All methods are pure state transitions.
#[derive(Debug, Default)]
pub struct Automation {
    // -- per playback instance (reset on track change / restart) --
    track_uri: Option<String>,
    /// First observation of this instance was near the track start.
    eligible_for_start: bool,
    start_fired: bool,
    /// A manual seek into the intro was observed — never jump again.
    start_suppressed: bool,
    start_attempts: u32,
    skip_retry_pending: bool,
    skip_retried: bool,
    actions: u32,
    /// Absorb the next observation without classifying it (it reflects our
    /// own control action, not user input).
    rebase: bool,
    // -- M7 queue prediction, also per instance --
    queue_fetches: u32,
    /// The near-boundary refetch already happened (the budget's second slot).
    queue_fetched_near_boundary: bool,
    prearmed: Option<Prearmed>,
    burst: Option<Burst>,
    // -- latest observation (interpolation + manual-seek baseline) --
    last: Option<LastObs>,
    cue: Option<CuePoints>,
    duration_ms: u64,
    // -- M10 whole-song auto-skip (per instance) --
    /// The current track is flagged for whole-song auto-skip (an applied
    /// rejection suggestion). Set by the loop each poll via
    /// [`Automation::set_auto_skip`]; defaults false, so pre-M10 behavior and
    /// every existing test are unchanged.
    auto_skip_flag: bool,
    /// A manual seek was observed on this auto-skip track: the user
    /// deliberately wants to hear it now, so do not fight them this instance.
    auto_skip_suppressed: bool,
    // -- survives instance changes (global action rate bound) --
    last_action_at_ms: Option<u64>,
    // -- M9: user-behavior events classified this poll, drained by the loop.
    //    Never more than a couple per poll (one seek or one track-change),
    //    and taken every poll, so this is effectively bounded at ~1.
    events: Vec<InsightEvent>,
}

/// A skip point at (or beyond) the track end means "play to the end":
/// no skip is ever due, the next track follows naturally.
fn skip_active(cue: CuePoints, duration_ms: u64) -> bool {
    cue.skip_ms < duration_ms
}

impl Automation {
    /// Feed one fresh poll result. Returns the action to execute now, if any.
    pub fn on_poll(
        &mut self,
        obs: Option<&Obs>,
        cue: Option<CuePoints>,
        gates: Gates,
        now_ms: u64,
    ) -> Option<Action> {
        let Some(obs) = obs.filter(|o| o.controllable) else {
            self.clear_playback();
            return None;
        };

        // A burst poll is an ordinary poll that merely came sooner — count it
        // against the burst budget.
        if let Some(burst) = self.burst.as_mut() {
            burst.polls = burst.polls.saturating_add(1);
        }

        // 1. Classify the observation against the previous one: same
        //    instance continuing naturally, a restart (new instance), or a
        //    manual seek. `prev` stays Some only for natural continuation.
        let mut prev: Option<LastObs> = None;
        let mut manual = false;
        if self.track_uri.as_deref() != Some(obs.track_uri.as_str()) {
            // The previous track just ended. Classify how (user skip vs
            // natural end vs our own skip) BEFORE its state is reset — this
            // is the same signal automation already tracks, reused for M9.
            if gates.records_insights() {
                self.classify_ended_track();
            }
            self.begin_instance(obs);
        } else if let Some(last) = self.last {
            // While playing, the true position at fetch time lies anywhere
            // in [last observed, last + elapsed]; anything clearly outside
            // that interval is a jump.
            let expected = if last.playing {
                last.position_ms
                    .saturating_add(obs.fetched_at_ms.saturating_sub(last.fetched_at_ms))
            } else {
                last.position_ms
            };
            let lo = last.position_ms.min(expected);
            let hi = last.position_ms.max(expected);
            let jumped_back = obs.position_ms.saturating_add(MANUAL_SEEK_JUMP_MS) < lo;
            let jumped_forward = obs.position_ms > hi.saturating_add(MANUAL_SEEK_JUMP_MS);
            if jumped_back && obs.position_ms <= RESTART_WINDOW_MS {
                // The track restarted from the beginning (repeat-one or the
                // user restarting it): a NEW instance — recognized even
                // right after an own skip (repeat-one wraps to zero).
                self.begin_instance(obs);
            } else if self.rebase {
                // First observation after an own control call: it reflects
                // our own seek/skip, so only re-baseline (never a user event).
            } else if jumped_forward {
                manual = true;
                if gates.records_insights() {
                    self.push_seek_event(InsightKind::SeekForward, obs, expected);
                }
            } else if jumped_back {
                manual = true;
                if gates.records_insights() {
                    self.push_seek_event(InsightKind::SeekBack, obs, expected);
                }
            } else {
                prev = Some(last);
            }
        }
        self.rebase = false;
        self.cue = cue;
        self.duration_ms = obs.duration_ms;
        self.last = Some(LastObs {
            position_ms: obs.position_ms,
            fetched_at_ms: obs.fetched_at_ms,
            playing: obs.playing,
        });

        // 2. A manual seek is user intent: never corrected, never acted on.
        if manual {
            self.skip_retry_pending = false;
            // The user moved away from (or toward) the boundary the burst was
            // watching — its prediction of WHEN no longer holds.
            self.burst = None;
            // A deliberate seek within an auto-skip track means "I want to
            // hear it now" — stop auto-skipping it for this instance.
            if !gates.edit_hold && self.auto_skip_flag {
                self.auto_skip_suppressed = true;
            }
            // While the track's preset is being edited (M8), seeks are part
            // of the editing sandbox and must never suppress the start jump
            // for good — the edit exit restores the position anyway.
            if !gates.edit_hold && cue.is_some_and(|c| obs.position_ms < c.start_ms) {
                self.start_suppressed = true;
            }
            return None;
        }

        // 3. Hard no-action conditions (independent of any preset — an
        //    auto-skip track may have no cue at all, so these run before the
        //    `cue?` bail below).
        if !gates.all() || !obs.playing {
            return None;
        }
        if now_ms.saturating_sub(obs.fetched_at_ms) > MAX_SNAPSHOT_AGE_MS {
            return None;
        }
        if self.in_cooldown(now_ms) || self.actions >= MAX_ACTIONS_PER_INSTANCE {
            return None;
        }

        // 3b. Whole-song auto-skip (M10): a user-applied rejection. Fires
        //     through the SAME gates just checked (toggle, premium, device,
        //     staleness, cooldown, action cap) and takes priority over any
        //     preset. A manual seek into the song (handled above) suppresses
        //     it, so we never fight a deliberate listen.
        if self.auto_skip_flag && !self.auto_skip_suppressed {
            return Some(Action::SkipNext);
        }

        // From here on a preset is required.
        let cue = cue?;

        // 4. Single retry of a transiently failed skip, now that this fresh
        //    poll confirms we are still past the skip point.
        if self.skip_retry_pending {
            self.skip_retry_pending = false;
            self.skip_retried = true;
            if skip_active(cue, obs.duration_ms) && obs.position_ms >= cue.skip_ms {
                return Some(Action::SkipNext);
            }
        }

        // 5. The start jump (once per instance).
        if self.start_pending(cue) && obs.position_ms < cue.start_ms {
            return Some(Action::SeekToStart {
                start_ms: cue.start_ms,
            });
        }

        // 6. Observed natural crossing of the skip point (fallback for when
        //    the boundary one-shot could not fire, e.g. during a cooldown).
        if skip_active(cue, obs.duration_ms) {
            if let Some(p) = prev {
                if p.position_ms < cue.skip_ms && obs.position_ms >= cue.skip_ms {
                    return Some(Action::SkipNext);
                }
            }
        }
        None
    }

    /// Ms until a boundary one-shot should fire, when one is due within
    /// [`WAKEUP_HORIZON_MS`]. Based on interpolation of the last observation.
    pub fn plan_wakeup_ms(&self, gates: Gates, now_ms: u64) -> Option<u64> {
        let cue = self.cue?;
        let last = self.last?;
        if !gates.all() || !last.playing {
            return None;
        }
        if self.actions >= MAX_ACTIONS_PER_INSTANCE || !skip_active(cue, self.duration_ms) {
            return None;
        }
        let interpolated = last
            .position_ms
            .saturating_add(now_ms.saturating_sub(last.fetched_at_ms));
        if interpolated >= cue.skip_ms {
            // Already past the boundary: only an observed natural crossing
            // (on_poll) may still act — never a blind one-shot.
            return None;
        }
        let delay = cue
            .skip_ms
            .saturating_sub(ACTION_LEAD_MS)
            .saturating_sub(interpolated);
        (delay <= WAKEUP_HORIZON_MS).then_some(delay)
    }

    /// The scheduled one-shot fired: decide from interpolation whether the
    /// skip is (still) due.
    pub fn on_wakeup(&mut self, gates: Gates, now_ms: u64) -> Option<Action> {
        let cue = self.cue?;
        let last = self.last?;
        if !gates.all() || !last.playing {
            return None;
        }
        if self.in_cooldown(now_ms) || self.actions >= MAX_ACTIONS_PER_INSTANCE {
            return None;
        }
        if !skip_active(cue, self.duration_ms) {
            return None;
        }
        if now_ms.saturating_sub(last.fetched_at_ms) > MAX_SNAPSHOT_AGE_MS {
            return None;
        }
        let interpolated = last
            .position_ms
            .saturating_add(now_ms.saturating_sub(last.fetched_at_ms));
        let reached_fire_point = interpolated.saturating_add(ACTION_LEAD_MS) >= cue.skip_ms;
        if last.position_ms < cue.skip_ms && reached_fire_point {
            return Some(Action::SkipNext);
        }
        None
    }

    /// The engine executed `action`; record the outcome (cooldown, action
    /// count, retry bookkeeping).
    pub fn action_executed(&mut self, action: Action, outcome: Outcome, now_ms: u64) {
        self.actions = self.actions.saturating_add(1);
        self.last_action_at_ms = Some(now_ms);
        self.rebase = true;
        match action {
            Action::SeekToStart { .. } => {
                self.start_attempts = self.start_attempts.saturating_add(1);
                match outcome {
                    Outcome::Success => self.start_fired = true,
                    // A retry is allowed while attempts remain — the next
                    // poll must re-confirm the position first.
                    Outcome::Transient => {}
                    Outcome::Fatal => self.start_attempts = MAX_START_ATTEMPTS,
                }
            }
            Action::SkipNext => {
                if outcome == Outcome::Transient && !self.skip_retried {
                    self.skip_retry_pending = true;
                }
                // Our own skip IS the transition: with a pre-armed jump
                // waiting, confirm the new track at burst speed instead of
                // waiting out a full poll cycle.
                if outcome == Outcome::Success && self.prearmed.is_some() {
                    self.burst = Some(Burst {
                        polls: 0,
                        started_at_ms: now_ms,
                    });
                }
            }
        }
    }

    // -- M7: queue prediction & transition burst ------------------------------

    /// Whether the loop should fetch `/me/player/queue` now: once per
    /// instance on the track change, once more near the transition boundary,
    /// never more ([`MAX_QUEUE_FETCHES_PER_INSTANCE`]), and only while a
    /// controllable track is playing with all gates open — prediction serves
    /// automation only.
    pub fn wants_queue_fetch(&self, gates: Gates, now_ms: u64) -> bool {
        if !gates.all() || self.track_uri.is_none() {
            return false;
        }
        let Some(last) = self.last else {
            return false;
        };
        if !last.playing || self.queue_fetches >= MAX_QUEUE_FETCHES_PER_INSTANCE {
            return false;
        }
        if self.queue_fetches == 0 {
            return true;
        }
        !self.queue_fetched_near_boundary && self.near_boundary(now_ms)
    }

    /// A queue fetch succeeded; cache what (if anything) it pre-arms.
    pub fn queue_fetched(&mut self, prearmed: Option<Prearmed>, now_ms: u64) {
        self.note_queue_fetch(now_ms);
        self.prearmed = prearmed;
    }

    /// A queue fetch errored: it still counts toward the hard cap, but an
    /// earlier prediction stays — a stale hint only makes the engine confirm
    /// a transition faster, it never drives an action.
    pub fn queue_fetch_failed(&mut self, now_ms: u64) {
        self.note_queue_fetch(now_ms);
    }

    fn note_queue_fetch(&mut self, now_ms: u64) {
        self.queue_fetches = self.queue_fetches.saturating_add(1);
        if self.near_boundary(now_ms) {
            self.queue_fetched_near_boundary = true;
        }
    }

    /// URI of the pre-armed (queue-predicted) next track, if any.
    pub fn prearmed_track(&self) -> Option<&str> {
        self.prearmed.as_ref().map(|p| p.track_uri.as_str())
    }

    /// Ms until the transition-confirmation wakeup for a NATURAL track end,
    /// when a pre-armed jump exists and the end is within
    /// [`WAKEUP_HORIZON_MS`]. When our own skip will fire instead, the burst
    /// starts from that skip's execution — no wakeup needed.
    pub fn plan_transition_wakeup_ms(&self, gates: Gates, now_ms: u64) -> Option<u64> {
        self.prearmed.as_ref()?;
        let last = self.last?;
        if !gates.all() || !last.playing || self.burst.is_some() {
            return None;
        }
        if self.skip_will_fire() || self.duration_ms == 0 {
            return None;
        }
        let interpolated = last
            .position_ms
            .saturating_add(now_ms.saturating_sub(last.fetched_at_ms));
        let wake_at = self.duration_ms.saturating_sub(TRANSITION_WAKE_LEAD_MS);
        if interpolated >= wake_at {
            // The regular poll cadence already brackets the boundary.
            return None;
        }
        let delay = wake_at - interpolated;
        (delay <= WAKEUP_HORIZON_MS).then_some(delay)
    }

    /// The transition wakeup fired: start the confirmation burst — never an
    /// action, only faster polling. Returns whether a burst began.
    pub fn on_transition_wakeup(&mut self, gates: Gates, now_ms: u64) -> bool {
        if self.prearmed.is_none() || self.burst.is_some() {
            return false;
        }
        let Some(last) = self.last else {
            return false;
        };
        if !gates.all() || !last.playing {
            return false;
        }
        if now_ms.saturating_sub(last.fetched_at_ms) > MAX_SNAPSHOT_AGE_MS {
            return false;
        }
        self.burst = Some(Burst {
            polls: 0,
            started_at_ms: now_ms,
        });
        true
    }

    /// Ms until the next fast poll while a transition burst is active —
    /// strictly bounded by [`BURST_POLL_COUNT`] and [`BURST_MAX_TOTAL_MS`].
    pub fn plan_burst_delay_ms(&self, gates: Gates, now_ms: u64) -> Option<u64> {
        let burst = self.burst?;
        let last = self.last?;
        if !gates.all() || !last.playing {
            return None;
        }
        if burst.polls >= BURST_POLL_COUNT
            || now_ms.saturating_sub(burst.started_at_ms) >= BURST_MAX_TOTAL_MS
        {
            return None;
        }
        Some(BURST_POLL_SPACING_MS)
    }

    /// A 429 is being honored: while its `Retry-After` is pending, no burst
    /// may run (the prediction itself may stay — it is only a hint).
    pub fn note_rate_limited(&mut self) {
        self.burst = None;
    }

    /// A UI-initiated seek (M8 edit-mode preview or exit-restore) was just
    /// executed: absorb the next observation instead of classifying it as a
    /// manual seek — UI seeks are never automation actions and must never
    /// trigger the manual-seek suppression.
    pub fn note_external_seek(&mut self) {
        self.rebase = true;
    }

    /// Flag whether the CURRENT track is armed for whole-song auto-skip (M10).
    /// Called by the loop each poll from the applied-suggestion lookup, BEFORE
    /// [`Automation::on_poll`]. Orthogonal to a preset: an auto-skip track may
    /// have no cue at all.
    pub fn set_auto_skip(&mut self, on: bool) {
        self.auto_skip_flag = on;
    }

    /// Drain the user-behavior events classified since the last call (M9).
    /// The poll loop calls this after every [`Automation::on_poll`] and
    /// persists them off the hot path.
    pub fn take_events(&mut self) -> Vec<InsightEvent> {
        std::mem::take(&mut self.events)
    }

    /// A track change was just observed: record a `skip_next` event for the
    /// track that ended IFF the user (not Cued) skipped it before its natural
    /// end. Reads the still-current previous-track state, so it MUST run
    /// before [`Automation::begin_instance`] resets it.
    fn classify_ended_track(&mut self) {
        // `rebase` set means our own SkipNext caused this transition — never
        // a user event.
        if self.rebase {
            return;
        }
        let (Some(uri), Some(last)) = (self.track_uri.clone(), self.last) else {
            return;
        };
        // A natural end (the song simply finished) is not a skip.
        if self.duration_ms == 0
            || last.position_ms.saturating_add(NATURAL_END_WINDOW_MS) >= self.duration_ms
        {
            return;
        }
        self.events.push(InsightEvent {
            track_uri: uri,
            kind: InsightKind::SkipNext,
            from_ms: last.position_ms,
            to_ms: None,
            duration_ms: self.duration_ms,
        });
    }

    /// Record a manual seek: `from_ms` is the interpolated position the user
    /// left, `to_ms` (in `obs`) is where they landed.
    fn push_seek_event(&mut self, kind: InsightKind, obs: &Obs, from_ms: u64) {
        self.events.push(InsightEvent {
            track_uri: obs.track_uri.clone(),
            kind,
            from_ms,
            to_ms: Some(obs.position_ms),
            duration_ms: obs.duration_ms,
        });
    }

    /// Whether our own skip is expected to end this instance (otherwise the
    /// transition boundary is the natural track end).
    fn skip_will_fire(&self) -> bool {
        self.cue.is_some_and(|c| skip_active(c, self.duration_ms))
            && self.actions < MAX_ACTIONS_PER_INSTANCE
    }

    /// True within [`PREDICT_HORIZON_MS`] of the next transition boundary.
    fn near_boundary(&self, now_ms: u64) -> bool {
        let Some(last) = self.last else {
            return false;
        };
        let boundary = if self.skip_will_fire() {
            // skip_will_fire guarantees the cue exists.
            self.cue.map(|c| c.skip_ms).unwrap_or(self.duration_ms)
        } else {
            self.duration_ms
        };
        let interpolated = last
            .position_ms
            .saturating_add(now_ms.saturating_sub(last.fetched_at_ms));
        boundary.saturating_sub(interpolated) < PREDICT_HORIZON_MS
    }

    fn start_pending(&self, cue: CuePoints) -> bool {
        self.eligible_for_start
            && !self.start_fired
            && !self.start_suppressed
            && self.start_attempts < MAX_START_ATTEMPTS
            && cue.start_ms > 0
            && cue.start_ms < self.duration_ms
    }

    fn in_cooldown(&self, now_ms: u64) -> bool {
        self.last_action_at_ms
            .is_some_and(|at| now_ms.saturating_sub(at) < ACTION_COOLDOWN_MS)
    }

    /// A new playback instance begins with this observation. The global
    /// cooldown (`last_action_at_ms`) intentionally survives.
    fn begin_instance(&mut self, obs: &Obs) {
        self.track_uri = Some(obs.track_uri.clone());
        self.eligible_for_start = obs.position_ms <= START_NEAR_MS;
        self.start_fired = false;
        self.start_suppressed = false;
        self.start_attempts = 0;
        self.skip_retry_pending = false;
        self.skip_retried = false;
        self.actions = 0;
        self.rebase = false;
        // The auto-skip FLAG is a track property re-set by the loop each poll,
        // so it is not reset here; only its per-instance suppression is.
        self.auto_skip_suppressed = false;
        // The prediction targeted exactly this transition: whether it matched
        // or not, it is spent — as is the burst that was confirming it.
        self.queue_fetches = 0;
        self.queue_fetched_near_boundary = false;
        self.prearmed = None;
        self.burst = None;
        self.last = None;
    }

    /// Nothing controllable is loaded: drop all per-track state.
    fn clear_playback(&mut self) {
        let last_action_at_ms = self.last_action_at_ms;
        *self = Automation {
            last_action_at_ms,
            ..Automation::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = "spotify:track:a";
    const DURATION: u64 = 200_000;

    /// Default test preset: start 30 s, skip 3:00 on a 3:20 track.
    fn cue() -> CuePoints {
        CuePoints {
            start_ms: 30_000,
            skip_ms: 180_000,
        }
    }

    fn gates() -> Gates {
        Gates {
            automation_on: true,
            premium: true,
            device_ok: true,
            edit_hold: false,
            insights_on: true,
        }
    }

    fn obs(uri: &str, position_ms: u64, at_ms: u64) -> Obs {
        Obs {
            track_uri: uri.into(),
            controllable: true,
            playing: true,
            position_ms,
            duration_ms: DURATION,
            fetched_at_ms: at_ms,
        }
    }

    fn paused(uri: &str, position_ms: u64, at_ms: u64) -> Obs {
        Obs {
            playing: false,
            ..obs(uri, position_ms, at_ms)
        }
    }

    /// Shorthand: poll with the default cue and open gates.
    fn poll(a: &mut Automation, o: &Obs, at_ms: u64) -> Option<Action> {
        a.on_poll(Some(o), Some(cue()), gates(), at_ms)
    }

    const SEEK: Action = Action::SeekToStart { start_ms: 30_000 };

    // -- start jump -----------------------------------------------------------

    #[test]
    fn start_fires_once_for_a_track_started_from_the_beginning() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 500, 0), 0), Some(SEEK));
        a.action_executed(SEEK, Outcome::Success, 0);
        // Next obs reflects our own seek — absorbed, no action.
        assert_eq!(poll(&mut a, &obs(TRACK, 30_400, 1_000), 1_000), None);
        // Well past the cooldown, position in the active zone: still nothing.
        assert_eq!(poll(&mut a, &obs(TRACK, 33_400, 4_000), 4_000), None);
    }

    #[test]
    fn start_does_not_fire_when_the_track_is_first_seen_mid_intro() {
        // App connected (or track was scrubbed) while the intro was already
        // underway: position < start_ms but NOT near the track start.
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 10_000, 0), 0), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 11_000, 1_000), 1_000), None);
    }

    #[test]
    fn start_at_zero_is_a_no_op() {
        let mut a = Automation::default();
        let zero_start = CuePoints {
            start_ms: 0,
            skip_ms: 180_000,
        };
        let action = a.on_poll(Some(&obs(TRACK, 500, 0)), Some(zero_start), gates(), 0);
        assert_eq!(action, None);
    }

    #[test]
    fn a_manual_seek_back_into_the_intro_does_not_refire_start() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 400, 0), 0), Some(SEEK));
        a.action_executed(SEEK, Outcome::Success, 0);
        assert_eq!(poll(&mut a, &obs(TRACK, 30_300, 1_000), 1_000), None);
        // The user deliberately seeks back into the intro (beyond the
        // restart window): user intent, no correction — ever.
        assert_eq!(poll(&mut a, &obs(TRACK, 10_000, 3_000), 3_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 11_000, 4_000), 4_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 15_000, 8_000), 8_000), None);
    }

    #[test]
    fn natural_skip_crossing_still_fires_after_a_manual_seek_into_the_intro() {
        // ...continuing the scenario above: the outro boundary still applies
        // when playback NATURALLY crosses it later.
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 400, 0), 0), Some(SEEK));
        a.action_executed(SEEK, Outcome::Success, 0);
        assert_eq!(poll(&mut a, &obs(TRACK, 30_300, 1_000), 1_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 10_000, 3_000), 3_000), None); // manual
                                                                           // Manual seek forward into the active zone near the outro.
        assert_eq!(poll(&mut a, &obs(TRACK, 178_000, 5_000), 5_000), None); // manual
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 6_000), 6_000), None);
        // Natural crossing of skip_ms → skip fires.
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_050, 7_050), 7_050),
            Some(Action::SkipNext)
        );
    }

    // -- skip crossing --------------------------------------------------------

    #[test]
    fn a_natural_crossing_of_skip_fires_the_skip() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 178_500, 0), 0), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 1_000), 1_000), None);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_400, 1_900), 1_900),
            Some(Action::SkipNext)
        );
    }

    #[test]
    fn a_manual_jump_past_skip_does_not_fire() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 60_000, 0), 0), None);
        // Manual seek deep into the outro: respected.
        assert_eq!(poll(&mut a, &obs(TRACK, 185_000, 1_000), 1_000), None);
        // Natural playback within the outro afterwards: still nothing
        // (skip_ms was never crossed FROM the active zone).
        assert_eq!(poll(&mut a, &obs(TRACK, 186_000, 2_000), 2_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 188_000, 4_000), 4_000), None);
    }

    #[test]
    fn skip_at_track_end_never_fires() {
        // skip_ms == duration means "play to the end" — the boundary case
        // must produce zero actions even when the position reaches duration.
        let mut a = Automation::default();
        let cue_end = CuePoints {
            start_ms: 30_000,
            skip_ms: DURATION,
        };
        let mut at =
            |pos: u64, t: u64| a.on_poll(Some(&obs(TRACK, pos, t)), Some(cue_end), gates(), t);
        assert_eq!(at(198_500, 0), None);
        assert_eq!(at(199_500, 1_000), None);
        assert_eq!(at(DURATION, 1_500), None);
    }

    // -- playback instances ---------------------------------------------------

    #[test]
    fn repeat_one_restart_is_a_new_instance_and_start_fires_again() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 199_000, 0), 0), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 199_900, 900), 900), None);
        // Position wrapped to near zero: repeat-one restarted the track.
        assert_eq!(poll(&mut a, &obs(TRACK, 300, 1_900), 1_900), Some(SEEK));
    }

    #[test]
    fn repeat_one_after_our_own_skip_is_a_new_instance() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Success, 800);
        // Repeat-one: the SAME track restarts right after our skip. Must be
        // recognized as a new instance even though a rebase is pending —
        // though the cooldown delays the start jump…
        assert_eq!(poll(&mut a, &obs(TRACK, 400, 1_200), 1_200), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 1_400, 2_200), 2_200), None);
        // …until it elapses.
        assert_eq!(poll(&mut a, &obs(TRACK, 2_100, 2_900), 2_900), Some(SEEK));
    }

    #[test]
    fn a_new_track_is_a_new_instance_with_the_cooldown_still_applying() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_200, 700), 700),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Success, 700);
        let b = "spotify:track:b";
        // New track begins near zero → start jump due, but the cooldown from
        // the skip suppresses it first.
        assert_eq!(poll(&mut a, &obs(b, 300, 1_000), 1_000), None);
        assert_eq!(poll(&mut a, &obs(b, 1_300, 2_000), 2_000), None);
        assert_eq!(poll(&mut a, &obs(b, 2_300, 3_000), 3_000), Some(SEEK));
    }

    // -- gates / no-action conditions ------------------------------------------

    #[test]
    fn paused_produces_no_action() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &paused(TRACK, 400, 0), 0), None);
        assert_eq!(poll(&mut a, &paused(TRACK, 400, 1_000), 1_000), None);
        // Resume → the pending start jump fires.
        assert_eq!(poll(&mut a, &obs(TRACK, 700, 2_000), 2_000), Some(SEEK));
    }

    #[test]
    fn closed_gates_produce_no_action() {
        for gates in [
            Gates {
                automation_on: false,
                ..gates()
            },
            Gates {
                premium: false,
                ..gates()
            },
            Gates {
                device_ok: false,
                ..gates()
            },
            Gates {
                edit_hold: true,
                ..gates()
            },
        ] {
            let mut a = Automation::default();
            assert_eq!(
                a.on_poll(Some(&obs(TRACK, 400, 0)), Some(cue()), gates, 0),
                None
            );
        }
    }

    #[test]
    fn no_preset_produces_no_action() {
        let mut a = Automation::default();
        assert_eq!(a.on_poll(Some(&obs(TRACK, 400, 0)), None, gates(), 0), None);
    }

    #[test]
    fn non_controllable_items_produce_no_action() {
        let mut a = Automation::default();
        let mut episode = obs("spotify:episode:e", 400, 0);
        episode.controllable = false;
        assert_eq!(a.on_poll(Some(&episode), Some(cue()), gates(), 0), None);
        assert_eq!(a.on_poll(None, Some(cue()), gates(), 0), None);
    }

    #[test]
    fn stale_observations_produce_no_action() {
        let mut a = Automation::default();
        // Observation sampled >2 poll cycles before "now": never act on it.
        let action = a.on_poll(
            Some(&obs(TRACK, 400, 0)),
            Some(cue()),
            gates(),
            MAX_SNAPSHOT_AGE_MS + 1,
        );
        assert_eq!(action, None);
    }

    // -- cooldown & cap ---------------------------------------------------------

    #[test]
    fn the_action_cap_bounds_a_whole_worst_case_journey() {
        // Everything fails transiently; the user fights back. No matter
        // what, the instance never exceeds MAX_ACTIONS_PER_INSTANCE.
        let short = CuePoints {
            start_ms: 30_000,
            skip_ms: 60_000,
        };
        let mut a = Automation::default();
        let mut at = |a: &mut Automation, pos: u64, t: u64| {
            a.on_poll(Some(&obs(TRACK, pos, t)), Some(short), gates(), t)
        };

        let seek = Action::SeekToStart { start_ms: 30_000 };
        assert_eq!(at(&mut a, 400, 0), Some(seek)); // action 1
        a.action_executed(seek, Outcome::Transient, 0);
        assert_eq!(at(&mut a, 1_400, 1_000), None); // cooldown
        assert_eq!(at(&mut a, 2_900, 2_500), Some(seek)); // action 2 (single retry)
        a.action_executed(seek, Outcome::Transient, 2_500);
        assert_eq!(at(&mut a, 5_400, 5_000), None); // both start attempts used
        assert_eq!(at(&mut a, 59_400, 59_000), None);
        assert_eq!(at(&mut a, 60_900, 60_500), Some(Action::SkipNext)); // action 3
        a.action_executed(Action::SkipNext, Outcome::Transient, 60_500);
        assert_eq!(at(&mut a, 63_400, 63_000), Some(Action::SkipNext)); // action 4 (single retry)
        a.action_executed(Action::SkipNext, Outcome::Transient, 63_000);
        // Manual seek back, then play through the skip point again:
        assert_eq!(at(&mut a, 20_000, 66_000), None); // manual
        assert_eq!(at(&mut a, 24_000, 70_000), None);
        assert_eq!(at(&mut a, 59_000, 105_000), None);
        // A real natural crossing — but the cap has been reached.
        assert_eq!(at(&mut a, 60_500, 106_500), None);
    }

    #[test]
    fn the_cooldown_suppresses_actions_after_any_action() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 400, 0), 0), Some(SEEK));
        a.action_executed(SEEK, Outcome::Success, 0);
        // The forced re-poll observes our own seek.
        assert_eq!(poll(&mut a, &obs(TRACK, 30_100, 300), 300), None);
        // The user restarts the track inside the cooldown: new instance,
        // start due — but suppressed until the cooldown elapses.
        assert_eq!(poll(&mut a, &obs(TRACK, 100, 1_000), 1_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 2_100, 3_000), 3_000), Some(SEEK));
    }

    // -- transient retries -------------------------------------------------------

    #[test]
    fn a_transient_skip_failure_is_retried_exactly_once() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Transient, 800);
        // Cooldown first…
        assert_eq!(poll(&mut a, &obs(TRACK, 182_000, 2_500), 2_500), None);
        // …then the single retry (re-poll confirmed we are still past skip).
        assert_eq!(
            poll(&mut a, &obs(TRACK, 182_800, 3_300), 3_300),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Transient, 3_300);
        // No third attempt, ever.
        assert_eq!(poll(&mut a, &obs(TRACK, 185_500, 6_000), 6_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 187_500, 8_000), 8_000), None);
    }

    #[test]
    fn a_manual_seek_cancels_a_pending_skip_retry() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Transient, 800);
        // The user seeks back into the song: their intent wins, retry dropped.
        assert_eq!(poll(&mut a, &obs(TRACK, 100_000, 3_000), 3_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 101_000, 4_000), 4_000), None);
    }

    #[test]
    fn a_transient_start_failure_is_retried_exactly_once() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 400, 0), 0), Some(SEEK));
        a.action_executed(SEEK, Outcome::Transient, 0);
        assert_eq!(poll(&mut a, &obs(TRACK, 1_400, 1_000), 1_000), None); // cooldown
        assert_eq!(poll(&mut a, &obs(TRACK, 2_900, 2_500), 2_500), Some(SEEK));
        a.action_executed(SEEK, Outcome::Transient, 2_500);
        assert_eq!(poll(&mut a, &obs(TRACK, 5_400, 5_000), 5_000), None);
        assert_eq!(poll(&mut a, &obs(TRACK, 6_400, 6_000), 6_000), None);
    }

    // -- preset changes mid-play ---------------------------------------------------

    #[test]
    fn a_preset_saved_while_the_track_plays_takes_effect() {
        let mut a = Automation::default();
        // No preset yet: nothing happens.
        assert_eq!(a.on_poll(Some(&obs(TRACK, 500, 0)), None, gates(), 0), None);
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 1_500, 1_000)), None, gates(), 1_000),
            None
        );
        // The user saves a preset (start ahead of the position): it applies
        // on the very next poll, no restart needed.
        assert_eq!(poll(&mut a, &obs(TRACK, 2_500, 2_000), 2_000), Some(SEEK));
    }

    // -- boundary one-shot (plan_wakeup / on_wakeup) ---------------------------------

    #[test]
    fn a_boundary_wakeup_is_planned_with_the_lead_time() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 0), 0), None);
        // fire position = skip - lead = 179_700; interpolated now = 179_000.
        assert_eq!(a.plan_wakeup_ms(gates(), 0), Some(700));
    }

    #[test]
    fn no_wakeup_is_planned_far_from_the_boundary_or_while_paused() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 150_000, 0), 0), None);
        assert_eq!(a.plan_wakeup_ms(gates(), 0), None); // > horizon
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &paused(TRACK, 179_000, 0), 0), None);
        assert_eq!(a.plan_wakeup_ms(gates(), 0), None); // paused
    }

    #[test]
    fn no_wakeup_is_planned_for_skip_at_track_end() {
        let mut a = Automation::default();
        let cue_end = CuePoints {
            start_ms: 0,
            skip_ms: DURATION,
        };
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_000, 0)), Some(cue_end), gates(), 0),
            None
        );
        assert_eq!(a.plan_wakeup_ms(gates(), 0), None);
    }

    #[test]
    fn the_wakeup_fires_the_skip_from_interpolation() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 0), 0), None);
        let delay = a.plan_wakeup_ms(gates(), 0).expect("wakeup planned");
        assert_eq!(a.on_wakeup(gates(), delay), Some(Action::SkipNext));
    }

    #[test]
    fn the_wakeup_declines_on_stale_data() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 0), 0), None);
        // The engine overslept (system suspend): data too old, do nothing.
        assert_eq!(a.on_wakeup(gates(), MAX_SNAPSHOT_AGE_MS + 1), None);
    }

    #[test]
    fn the_wakeup_declines_when_gates_closed_since_planning() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 0), 0), None);
        let delay = a.plan_wakeup_ms(gates(), 0).expect("wakeup planned");
        let off = Gates {
            automation_on: false,
            ..gates()
        };
        assert_eq!(a.on_wakeup(off, delay), None);
    }

    #[test]
    fn the_wakeup_declines_during_the_cooldown() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 0), 0), None);
        a.action_executed(SEEK, Outcome::Success, 100);
        assert_eq!(a.on_wakeup(gates(), 700), None);
    }

    // -- M7: queue prediction (fetch budget) -----------------------------------

    const TRACK_B: &str = "spotify:track:b";

    fn prearm_b() -> Prearmed {
        Prearmed {
            track_uri: TRACK_B.into(),
            start_ms: 20_000,
        }
    }

    /// Preset of the predicted track B: start 20 s, skip 100 s.
    fn cue_b() -> CuePoints {
        CuePoints {
            start_ms: 20_000,
            skip_ms: 100_000,
        }
    }

    #[test]
    fn queue_is_fetched_once_per_instance_plus_once_near_the_boundary() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 40_000, 0), 0), None);
        assert!(a.wants_queue_fetch(gates(), 0)); // fetch #1: new instance
        a.queue_fetched(Some(prearm_b()), 0);
        assert!(!a.wants_queue_fetch(gates(), 0));
        // Mid-track: still cached, no refetch.
        assert_eq!(poll(&mut a, &obs(TRACK, 60_000, 20_000), 20_000), None);
        assert!(!a.wants_queue_fetch(gates(), 20_000));
        // Less than PREDICT_HORIZON_MS to the skip boundary (180 s): refetch.
        assert_eq!(poll(&mut a, &obs(TRACK, 166_000, 126_000), 126_000), None);
        assert!(a.wants_queue_fetch(gates(), 126_000)); // fetch #2
        a.queue_fetched(Some(prearm_b()), 126_000);
        // Hard cap: never a third fetch for this instance.
        assert_eq!(poll(&mut a, &obs(TRACK, 170_000, 130_000), 130_000), None);
        assert!(!a.wants_queue_fetch(gates(), 130_000));
    }

    #[test]
    fn a_track_change_resets_the_fetch_budget_and_discards_the_prediction() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 40_000, 0), 0), None);
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(a.prearmed_track(), Some(TRACK_B));
        // A different track appears (shuffle surprise): prediction discarded.
        assert_eq!(
            poll(&mut a, &obs("spotify:track:c", 40_000, 1_000), 1_000),
            None
        );
        assert_eq!(a.prearmed_track(), None);
        assert!(a.wants_queue_fetch(gates(), 1_000));
    }

    #[test]
    fn closed_gates_paused_or_no_playback_mean_zero_queue_fetches() {
        let off = Gates {
            automation_on: false,
            ..gates()
        };
        let free = Gates {
            premium: false,
            ..gates()
        };
        for g in [off, free] {
            let mut a = Automation::default();
            assert_eq!(
                a.on_poll(Some(&obs(TRACK, 40_000, 0)), Some(cue()), g, 0),
                None
            );
            assert!(!a.wants_queue_fetch(g, 0));
        }
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &paused(TRACK, 40_000, 0), 0), None);
        assert!(!a.wants_queue_fetch(gates(), 0)); // paused: no transition coming
        let a = Automation::default();
        assert!(!a.wants_queue_fetch(gates(), 0)); // nothing playing at all
    }

    #[test]
    fn a_failed_queue_fetch_counts_toward_the_cap_and_keeps_the_prior_hint() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 40_000, 0), 0), None);
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(poll(&mut a, &obs(TRACK, 166_000, 126_000), 126_000), None);
        assert!(a.wants_queue_fetch(gates(), 126_000));
        a.queue_fetch_failed(126_000); // the near-boundary refetch errored
        assert_eq!(a.prearmed_track(), Some(TRACK_B)); // hint kept
        assert!(!a.wants_queue_fetch(gates(), 126_500)); // attempt still counted
    }

    // -- M7: transition burst ---------------------------------------------------

    #[test]
    fn our_own_successful_skip_with_a_prediction_starts_the_burst() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Success, 800);
        assert_eq!(
            a.plan_burst_delay_ms(gates(), 800),
            Some(BURST_POLL_SPACING_MS)
        );
    }

    #[test]
    fn no_burst_without_a_prediction_or_after_a_failed_skip() {
        // No prediction: the skip alone must not start a burst.
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Success, 800);
        assert_eq!(a.plan_burst_delay_ms(gates(), 800), None);
        // Prediction, but the skip failed: no transition happened.
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Transient, 800);
        assert_eq!(a.plan_burst_delay_ms(gates(), 800), None);
    }

    /// Shorthand: a burst armed via our own skip at t=800 (prediction cached).
    fn burst_after_own_skip() -> Automation {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        a.action_executed(Action::SkipNext, Outcome::Success, 800);
        a
    }

    #[test]
    fn the_burst_is_bounded_by_its_poll_count() {
        let mut a = burst_after_own_skip();
        // Spotify is slow to switch: every burst poll still shows track A.
        assert_eq!(poll(&mut a, &obs(TRACK, 180_500, 1_100), 1_100), None);
        assert_eq!(
            a.plan_burst_delay_ms(gates(), 1_100),
            Some(BURST_POLL_SPACING_MS)
        );
        assert_eq!(poll(&mut a, &obs(TRACK, 180_800, 1_400), 1_400), None);
        assert_eq!(
            a.plan_burst_delay_ms(gates(), 1_400),
            Some(BURST_POLL_SPACING_MS)
        );
        assert_eq!(poll(&mut a, &obs(TRACK, 181_100, 1_700), 1_700), None);
        // BURST_POLL_COUNT polls done: the burst is over.
        assert_eq!(a.plan_burst_delay_ms(gates(), 1_700), None);
    }

    #[test]
    fn the_burst_is_bounded_by_total_duration() {
        let a = burst_after_own_skip();
        assert_eq!(
            a.plan_burst_delay_ms(gates(), 800 + BURST_MAX_TOTAL_MS),
            None
        );
    }

    #[test]
    fn a_rate_limit_cancels_the_burst() {
        let mut a = burst_after_own_skip();
        a.note_rate_limited();
        assert_eq!(a.plan_burst_delay_ms(gates(), 800), None);
    }

    #[test]
    fn closed_gates_stop_the_burst_polling() {
        let a = burst_after_own_skip();
        let off = Gates {
            automation_on: false,
            ..gates()
        };
        assert_eq!(a.plan_burst_delay_ms(off, 800), None);
    }

    // -- M7: natural-end transition wakeup --------------------------------------

    #[test]
    fn a_transition_wakeup_is_planned_before_a_natural_end_when_prearmed() {
        // Current track has NO preset at all — the prediction alone matters.
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_200, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(
            a.plan_transition_wakeup_ms(gates(), 0),
            Some(DURATION - TRANSITION_WAKE_LEAD_MS - 199_200)
        );
    }

    #[test]
    fn no_transition_wakeup_when_our_own_skip_will_fire_instead() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_000, 0), 0), None);
        a.queue_fetched(Some(prearm_b()), 0);
        // The skip boundary one-shot owns this transition…
        assert_eq!(a.plan_transition_wakeup_ms(gates(), 0), None);
        // …the natural-end wakeup would only double up on it.
        assert!(a.plan_wakeup_ms(gates(), 0).is_some());
    }

    #[test]
    fn no_transition_wakeup_without_a_prediction_or_far_from_the_end() {
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_200, 0)), None, gates(), 0),
            None
        );
        assert_eq!(a.plan_transition_wakeup_ms(gates(), 0), None); // nothing prearmed
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 150_000, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(a.plan_transition_wakeup_ms(gates(), 0), None); // beyond the horizon
    }

    #[test]
    fn the_transition_wakeup_starts_the_burst_only_when_fresh_and_playing() {
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_200, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        let delay = a.plan_transition_wakeup_ms(gates(), 0).expect("planned");
        assert!(a.on_transition_wakeup(gates(), delay));
        assert_eq!(
            a.plan_burst_delay_ms(gates(), delay),
            Some(BURST_POLL_SPACING_MS)
        );
        // Stale observation (system slept through the boundary): declined.
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_200, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        assert!(!a.on_transition_wakeup(gates(), MAX_SNAPSHOT_AGE_MS + 1));
        // Gates closed since planning: declined.
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_200, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        let off = Gates {
            automation_on: false,
            ..gates()
        };
        assert!(!a.on_transition_wakeup(off, 500));
    }

    // -- M7: confirmation semantics ---------------------------------------------

    #[test]
    fn a_confirmed_prediction_fires_the_start_jump_on_the_burst_poll() {
        // Natural end (no preceding action, so no cooldown in the way).
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_300, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        let delay = a.plan_transition_wakeup_ms(gates(), 0).expect("planned");
        assert!(a.on_transition_wakeup(gates(), delay));
        // Burst poll #1 still sees the old track at its very end.
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_900, 600)), None, gates(), 600),
            None
        );
        // Burst poll #2 confirms the predicted track just after its start:
        // the (normal M4) start jump fires immediately.
        assert_eq!(
            a.on_poll(Some(&obs(TRACK_B, 300, 900)), Some(cue_b()), gates(), 900),
            Some(Action::SeekToStart { start_ms: 20_000 })
        );
        assert_eq!(a.prearmed_track(), None); // prediction consumed
    }

    #[test]
    fn a_mismatched_prediction_is_discarded_and_normal_rules_apply() {
        let track_c = "spotify:track:c";
        // Surprise track WITHOUT a preset: nothing fires.
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_300, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        assert!(a.on_transition_wakeup(gates(), 500));
        assert_eq!(
            a.on_poll(Some(&obs(track_c, 300, 900)), None, gates(), 900),
            None
        );
        assert_eq!(a.prearmed_track(), None); // discarded cleanly
        assert_eq!(a.plan_burst_delay_ms(gates(), 900), None); // burst over
                                                               // Surprise track WITH a preset: the normal start jump still applies
                                                               // (confirmed observation, not the queue), exactly as without M7.
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 199_300, 0)), None, gates(), 0),
            None
        );
        a.queue_fetched(Some(prearm_b()), 0);
        assert!(a.on_transition_wakeup(gates(), 500));
        assert_eq!(
            a.on_poll(Some(&obs(track_c, 300, 900)), Some(cue_b()), gates(), 900),
            Some(Action::SeekToStart { start_ms: 20_000 })
        );
    }

    // -- M8: edit-mode gate -------------------------------------------------------

    /// Open gates except for the edit hold (the user is editing this track).
    fn editing() -> Gates {
        Gates {
            edit_hold: true,
            ..gates()
        }
    }

    #[test]
    fn no_action_fires_while_edit_mode_holds_the_track() {
        let mut a = Automation::default();
        // A due start jump is gated…
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 400, 0)), Some(cue()), editing(), 0),
            None
        );
        // …and so is an observed natural crossing of the skip point.
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 179_500, 1_000)),
                Some(cue()),
                editing(),
                1_000
            ),
            None
        );
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 180_400, 1_900)),
                Some(cue()),
                editing(),
                1_900
            ),
            None
        );
    }

    #[test]
    fn edit_mode_blocks_wakeups_bursts_and_queue_fetches() {
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 179_000, 0)), Some(cue()), editing(), 0),
            None
        );
        assert_eq!(a.plan_wakeup_ms(editing(), 0), None);
        assert_eq!(a.on_wakeup(editing(), 700), None);
        assert!(!a.wants_queue_fetch(editing(), 0));
        a.queue_fetched(Some(prearm_b()), 0);
        assert_eq!(a.plan_transition_wakeup_ms(editing(), 0), None);
    }

    #[test]
    fn automation_resumes_after_edit_mode_ends() {
        let mut a = Automation::default();
        // Editing near the boundary: nothing may fire.
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 178_000, 0)), Some(cue()), editing(), 0),
            None
        );
        // Edit ends; playback continues and naturally crosses the skip point.
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 179_000, 1_000)),
                Some(cue()),
                gates(),
                1_000
            ),
            None
        );
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 180_200, 2_200)),
                Some(cue()),
                gates(),
                2_200
            ),
            Some(Action::SkipNext)
        );
    }

    #[test]
    fn a_pending_start_jump_survives_edit_mode() {
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 400, 0)), Some(cue()), editing(), 0),
            None
        );
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 1_400, 1_000)),
                Some(cue()),
                editing(),
                1_000
            ),
            None
        );
        // Edit ends inside the intro: the start jump is still due and fires.
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 2_400, 2_000)), Some(cue()), gates(), 2_000),
            Some(SEEK)
        );
    }

    #[test]
    fn a_ui_seek_is_absorbed_and_never_suppresses_the_start_jump() {
        let mut a = Automation::default();
        // Track starts from the beginning while its preset is being edited.
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 400, 0)), Some(cue()), editing(), 0),
            None
        );
        // Preview: the UI seeks into the intro (before start_ms). Absorbed —
        // not a manual seek, so no suppression.
        a.note_external_seek();
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 27_000, 1_000)),
                Some(cue()),
                editing(),
                1_000
            ),
            None
        );
        // Exit restore: the UI seeks back to the remembered position.
        a.note_external_seek();
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 12_000, 2_000)),
                Some(cue()),
                editing(),
                2_000
            ),
            None
        );
        // Edit over, gates open: the start jump is still due and fires.
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 13_000, 3_000)),
                Some(cue()),
                gates(),
                3_000
            ),
            Some(SEEK)
        );
    }

    #[test]
    fn a_manual_seek_during_edit_mode_does_not_suppress_the_start_jump() {
        // Even a real user seek in the Spotify client while editing must not
        // permanently suppress the start jump — edit mode is a sandbox.
        let mut a = Automation::default();
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 400, 0)), Some(cue()), editing(), 0),
            None
        );
        // Manual jump into the intro (no absorb flag set).
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 12_000, 1_000)),
                Some(cue()),
                editing(),
                1_000
            ),
            None
        );
        // Edit ends: the start jump is still due.
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK, 13_000, 2_000)),
                Some(cue()),
                gates(),
                2_000
            ),
            Some(SEEK)
        );
    }

    #[test]
    fn the_pre_armed_jump_respects_the_cooldown_after_our_own_skip() {
        let mut a = burst_after_own_skip();
        // Burst polls confirm the predicted track inside its intro — but the
        // skip's cooldown (M4, unchanged) holds the seek back…
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK_B, 400, 1_100)),
                Some(cue_b()),
                gates(),
                1_100
            ),
            None
        );
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK_B, 700, 1_400)),
                Some(cue_b()),
                gates(),
                1_400
            ),
            None
        );
        // …until it elapses (skip was at t=800): then the seek fires, and it
        // counts toward THIS instance's action cap like any other action.
        assert_eq!(
            a.on_poll(
                Some(&obs(TRACK_B, 2_100, 2_900)),
                Some(cue_b()),
                gates(),
                2_900
            ),
            Some(Action::SeekToStart { start_ms: 20_000 })
        );
    }

    // -- M9: listening-insights classification --------------------------------

    /// Poll with NO preset (isolates classification from any start/skip
    /// action) and drain whatever events it produced.
    fn events_of(a: &mut Automation, o: &Obs, at_ms: u64) -> Vec<InsightEvent> {
        a.on_poll(Some(o), None, gates(), at_ms);
        a.take_events()
    }

    #[test]
    fn a_manual_forward_seek_is_recorded_with_positions() {
        let mut a = Automation::default();
        assert!(events_of(&mut a, &obs(TRACK, 30_000, 0), 0).is_empty()); // baseline
        let evs = events_of(&mut a, &obs(TRACK, 120_000, 1_000), 1_000);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, InsightKind::SeekForward);
        assert_eq!(evs[0].track_uri, TRACK);
        assert_eq!(evs[0].from_ms, 31_000); // interpolated: 30_000 + 1_000 elapsed
        assert_eq!(evs[0].to_ms, Some(120_000));
        assert_eq!(evs[0].duration_ms, DURATION);
    }

    #[test]
    fn a_manual_backward_seek_is_recorded() {
        let mut a = Automation::default();
        assert!(events_of(&mut a, &obs(TRACK, 120_000, 0), 0).is_empty());
        // Back to 40_000 — well above the restart window, so a seek not a restart.
        let evs = events_of(&mut a, &obs(TRACK, 40_000, 1_000), 1_000);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, InsightKind::SeekBack);
        assert_eq!(evs[0].from_ms, 121_000); // 120_000 + 1_000
        assert_eq!(evs[0].to_ms, Some(40_000));
    }

    #[test]
    fn a_user_skip_before_the_end_records_skip_next() {
        let mut a = Automation::default();
        assert!(events_of(&mut a, &obs(TRACK, 90_000, 0), 0).is_empty());
        assert!(events_of(&mut a, &obs(TRACK, 91_000, 1_000), 1_000).is_empty());
        // A different track appears with no Cued action between: user hit next.
        let evs = events_of(&mut a, &obs(TRACK_B, 500, 2_000), 2_000);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, InsightKind::SkipNext);
        assert_eq!(evs[0].track_uri, TRACK);
        assert_eq!(evs[0].from_ms, 91_000); // last observed position of the ended track
        assert_eq!(evs[0].to_ms, None);
        assert_eq!(evs[0].duration_ms, DURATION);
    }

    #[test]
    fn a_cued_skip_records_no_event() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 179_500, 0), 0), None);
        let _ = a.take_events();
        assert_eq!(
            poll(&mut a, &obs(TRACK, 180_300, 800), 800),
            Some(Action::SkipNext)
        );
        let _ = a.take_events();
        a.action_executed(Action::SkipNext, Outcome::Success, 800);
        // The new track appears right after OUR skip: never a user skip_next.
        poll(&mut a, &obs(TRACK_B, 500, 1_800), 1_800);
        assert!(a.take_events().is_empty());
    }

    #[test]
    fn a_natural_end_records_no_event() {
        let mut a = Automation::default();
        // Last seen within NATURAL_END_WINDOW_MS of the end: the song finished.
        assert!(events_of(&mut a, &obs(TRACK, DURATION - 1_000, 0), 0).is_empty());
        let evs = events_of(&mut a, &obs(TRACK_B, 500, 1_000), 1_000);
        assert!(evs.is_empty());
    }

    #[test]
    fn a_cued_start_jump_is_not_recorded_as_a_seek() {
        let mut a = Automation::default();
        assert_eq!(poll(&mut a, &obs(TRACK, 500, 0), 0), Some(SEEK));
        assert!(a.take_events().is_empty());
        a.action_executed(SEEK, Outcome::Success, 0);
        // The forced re-poll observes our own seek — absorbed, not a user seek.
        poll(&mut a, &obs(TRACK, 30_400, 1_000), 1_000);
        assert!(a.take_events().is_empty());
    }

    #[test]
    fn a_ui_seek_preview_or_restore_is_not_recorded() {
        let mut a = Automation::default();
        assert!(events_of(&mut a, &obs(TRACK, 30_000, 0), 0).is_empty());
        a.note_external_seek();
        // A jump big enough to look manual, but it was our own UI seek.
        let evs = events_of(&mut a, &obs(TRACK, 120_000, 1_000), 1_000);
        assert!(evs.is_empty());
    }

    #[test]
    fn a_repeat_one_restart_is_not_a_seek_event() {
        let mut a = Automation::default();
        assert!(events_of(&mut a, &obs(TRACK, 199_000, 0), 0).is_empty());
        // Position wraps near zero: a restart (new instance), not a backward seek.
        let evs = events_of(&mut a, &obs(TRACK, 300, 1_000), 1_000);
        assert!(evs.is_empty());
    }

    #[test]
    fn non_controllable_items_record_no_events() {
        let mut a = Automation::default();
        let mut ep = obs("spotify:episode:e", 60_000, 0);
        ep.controllable = false;
        a.on_poll(Some(&ep), None, gates(), 0);
        assert!(a.take_events().is_empty());
    }

    #[test]
    fn nothing_is_recorded_while_insights_are_off() {
        let off = Gates {
            insights_on: false,
            ..gates()
        };
        let mut a = Automation::default();
        a.on_poll(Some(&obs(TRACK, 30_000, 0)), None, off, 0);
        let _ = a.take_events();
        // A clear manual seek — gated off at the engine, not just the UI.
        a.on_poll(Some(&obs(TRACK, 120_000, 1_000)), None, off, 1_000);
        assert!(a.take_events().is_empty());
        // A skip to another track — also nothing.
        a.on_poll(Some(&obs(TRACK_B, 500, 2_000)), None, off, 2_000);
        assert!(a.take_events().is_empty());
    }

    #[test]
    fn nothing_is_recorded_while_editing_the_playing_track() {
        let mut a = Automation::default();
        a.on_poll(Some(&obs(TRACK, 30_000, 0)), None, editing(), 0);
        let _ = a.take_events();
        a.on_poll(Some(&obs(TRACK, 120_000, 1_000)), None, editing(), 1_000);
        assert!(a.take_events().is_empty());
    }

    // -- M10: whole-song auto-skip --------------------------------------------

    #[test]
    fn auto_skip_fires_immediately_when_a_flagged_track_starts() {
        // A flagged track with NO preset: it must still be skipped on sight.
        let mut a = Automation::default();
        a.set_auto_skip(true);
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 0, 0)), None, gates(), 0),
            Some(Action::SkipNext),
            "an armed track is skipped even without a preset"
        );
    }

    #[test]
    fn auto_skip_respects_every_m4_gate() {
        // Master toggle off.
        let mut a = Automation::default();
        a.set_auto_skip(true);
        let off = Gates {
            automation_on: false,
            ..gates()
        };
        assert_eq!(a.on_poll(Some(&obs(TRACK, 0, 0)), None, off, 0), None);

        // Not premium.
        let mut a = Automation::default();
        a.set_auto_skip(true);
        let no_prem = Gates {
            premium: false,
            ..gates()
        };
        assert_eq!(a.on_poll(Some(&obs(TRACK, 0, 0)), None, no_prem, 0), None);

        // Paused: never acts.
        let mut a = Automation::default();
        a.set_auto_skip(true);
        assert_eq!(
            a.on_poll(Some(&paused(TRACK, 0, 0)), None, gates(), 0),
            None
        );

        // Edit mode gates it too (edit_hold closes Gates::all).
        let mut a = Automation::default();
        a.set_auto_skip(true);
        assert_eq!(a.on_poll(Some(&obs(TRACK, 0, 0)), None, editing(), 0), None);
    }

    #[test]
    fn a_manual_seek_into_an_auto_skip_track_stops_the_fight() {
        let mut a = Automation::default();
        a.set_auto_skip(true);
        // First poll near the start would skip — but simulate the very first
        // observation establishing position, then a deliberate forward seek.
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 0, 0)), None, gates(), 0),
            Some(Action::SkipNext)
        );
        // Our own skip re-baselines; pretend it failed and the user instead
        // seeked deep into the song. Re-arm a fresh instance to test cleanly.
        let mut a = Automation::default();
        a.set_auto_skip(true);
        // Establish a baseline WITHOUT triggering (cooldown after a prior
        // action is not present on a fresh instance, so use a manual jump on
        // the second observation before any skip could be re-evaluated).
        // Poll 1 (t=0): would skip — consume it, then mark our own action.
        let first = a.on_poll(Some(&obs(TRACK, 0, 0)), None, gates(), 0);
        assert_eq!(first, Some(Action::SkipNext));
        a.action_executed(Action::SkipNext, Outcome::Transient, 0);
        // The action re-baselines (rebase) — the next observation is absorbed.
        a.set_auto_skip(true);
        a.on_poll(Some(&obs(TRACK, 100, 1_000)), None, gates(), 1_000);
        // Now the user manually seeks forward to 90 s: intent to listen.
        a.set_auto_skip(true);
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 90_000, 2_500)), None, gates(), 2_500),
            None,
            "a manual seek is never fought"
        );
        // And it stays suppressed for the rest of this instance.
        a.set_auto_skip(true);
        assert_eq!(
            a.on_poll(Some(&obs(TRACK, 95_000, 3_600)), None, gates(), 3_600),
            None,
            "auto-skip stays suppressed after a deliberate seek"
        );
    }

    #[test]
    fn a_new_instance_re_arms_auto_skip_after_suppression() {
        let mut a = Automation::default();
        a.set_auto_skip(true);
        a.on_poll(Some(&obs(TRACK, 0, 0)), None, gates(), 0);
        a.action_executed(Action::SkipNext, Outcome::Success, 0);
        // Suppress via a manual seek on the same instance.
        a.set_auto_skip(true);
        a.on_poll(Some(&obs(TRACK, 100, 1_000)), None, gates(), 1_000); // absorbed (rebase)
        a.set_auto_skip(true);
        a.on_poll(Some(&obs(TRACK, 90_000, 2_500)), None, gates(), 2_500); // manual → suppress
                                                                           // A different track starts (new instance) — past the cooldown.
        a.set_auto_skip(true);
        assert_eq!(
            a.on_poll(Some(&obs(TRACK_B, 0, 5_000)), None, gates(), 5_000),
            Some(Action::SkipNext),
            "the next flagged track is armed again"
        );
    }

    #[test]
    fn clearing_the_flag_disarms_auto_skip() {
        let mut a = Automation::default();
        a.set_auto_skip(false);
        assert_eq!(a.on_poll(Some(&obs(TRACK, 0, 0)), None, gates(), 0), None);
    }
}
