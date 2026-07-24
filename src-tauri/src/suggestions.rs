//! Listening-insights analysis engine (M10).
//!
//! A PURE module: recorded skip/seek events go in, calm suggestions come out.
//! No I/O, no network, no UI, no display metadata — the store (`presets.rs`)
//! persists the results and the loop/commands wire them to surfaces. Every
//! threshold is a named constant here, because "one premature suggestion costs
//! more trust than ten missed ones" — the bar is deliberately high.
//!
//! ## What "a play" means
//! M9 records only genuine skips/seeks, never completions, so Cued cannot see
//! a play in which the user did nothing. A *play* here is therefore a temporal
//! cluster of a track's events (see [`group_plays`]): "the plays in which you
//! did something". Denominators are counts of such observed plays. This is an
//! honest floor, not a lie — a suggestion only ever claims what the data shows.

use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Thresholds (all named, all here)
// ---------------------------------------------------------------------------

/// A `skip_next` fired before this position is a SONG REJECTION ("not this
/// song right now"), not a skip-point signal: excluded from skip-point
/// clustering, counted toward whole-song auto-skip.
pub const REJECTION_WINDOW_MS: u64 = 15_000;

/// A `seek_forward` that leaves a position before this is an "intro skip" —
/// a START-POINT signal. Later seeks are mid-track SKIP-POINT signals.
pub const EARLY_SEEK_WINDOW_MS: u64 = 15_000;

/// Events (or plays) whose positions fall within this radius of each other
/// belong to the same cluster.
pub const CLUSTER_RADIUS_MS: u64 = 5_000;

/// Recency half-life: an event this old counts for half of a brand-new one.
/// Taste drifts, so recent behavior weighs more.
pub const DECAY_HALF_LIFE_MS: u64 = 90 * 24 * 60 * 60 * 1_000;

/// Two of a track's events more than this far apart in wall-clock time belong
/// to different plays. Sized to bracket one listening of a long track (plus a
/// short pause) while still separating distinct listening occasions. Merging
/// two quick back-to-back replays is possible and deliberately conservative
/// (fewer plays raises the bar, never lowers it).
pub const PLAY_SESSION_GAP_MS: u64 = 8 * 60_000;

/// A skip-point / start-point suggestion needs at least this many observed
/// plays behind it.
pub const MIN_PLAYS: u32 = 5;

/// …and the pattern must hold in at least this (recency-weighted) share of
/// them.
pub const MIN_MATCH_RATIO: f64 = 0.70;

/// Whole-song auto-skip is higher-stakes, so it demands more evidence: more
/// plays…
pub const AUTO_SKIP_MIN_PLAYS: u32 = 10;

/// …and a stricter share of them ending in rejection.
pub const AUTO_SKIP_MIN_RATIO: f64 = 0.90;

/// A proactive card shown and ignored this many times retires (drops to the
/// Library section only).
pub const IGNORE_RETIRE_COUNT: u32 = 3;

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// The kind of a recorded event (mirrors the `type` column, minus display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    SeekForward,
    SeekBack,
    SkipNext,
}

/// One recorded listening event, as the analysis engine consumes it. Free of
/// any display metadata — just the behavioral facts.
#[derive(Debug, Clone, Copy)]
pub struct Event {
    pub kind: EventKind,
    /// Interpolated position the user left.
    pub from_ms: u64,
    /// Where a seek landed; `None` for `skip_next`.
    pub to_ms: Option<u64>,
    /// Unix ms the event was recorded.
    pub created_at: u64,
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

/// The three suggestion kinds Cued can raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionType {
    /// Skip the rest of the track from a consistent point.
    SkipPoint,
    /// Start the track from a consistent point (past the intro).
    StartPoint,
    /// Skip the whole track whenever it comes on.
    AutoSkip,
}

impl SuggestionType {
    /// Stable string persisted in the `type` column.
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionType::SkipPoint => "skip_point",
            SuggestionType::StartPoint => "start_point",
            SuggestionType::AutoSkip => "auto_skip",
        }
    }

    /// Parse the persisted string back to a type.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "skip_point" => Some(SuggestionType::SkipPoint),
            "start_point" => Some(SuggestionType::StartPoint),
            "auto_skip" => Some(SuggestionType::AutoSkip),
            _ => None,
        }
    }
}

/// A calm, evidence-backed suggestion. `value_*` carry the proposed times;
/// `plays_*` are the honest evidence shown in the card copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Suggestion {
    pub suggestion_type: SuggestionType,
    /// Skip point: region start (the point applied). Start point: the target.
    /// Auto-skip: `None`.
    pub value_start_ms: Option<u64>,
    /// Skip point: region end (for the "1:12–1:48" display). Otherwise `None`.
    pub value_end_ms: Option<u64>,
    /// Observed plays considered.
    pub plays_total: u32,
    /// …of which the pattern held in.
    pub plays_matching: u32,
}

// ---------------------------------------------------------------------------
// Lifecycle state machine (pure)
// ---------------------------------------------------------------------------

/// Lifecycle status of a stored suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Live: eligible for the proactive card and the Library section.
    Active,
    /// The user accepted it (preset saved, or auto-skip armed).
    Applied,
    /// The user said no — never resurface this type for this track, ever.
    Dismissed,
    /// Shown and ignored too often: no proactive card, Library only.
    Retired,
}

impl Status {
    /// Stable string persisted in the `status` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Applied => "applied",
            Status::Dismissed => "dismissed",
            Status::Retired => "retired",
        }
    }

    /// Parse the persisted status string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Status::Active),
            "applied" => Some(Status::Applied),
            "dismissed" => Some(Status::Dismissed),
            "retired" => Some(Status::Retired),
            _ => None,
        }
    }
}

/// The status a freshly analyzed suggestion should take, given any existing
/// row. Re-analysis MAY refresh values but NEVER resurrects a dismissed
/// suggestion and never overrides an applied/retired one — only a brand-new or
/// already-active suggestion is (re)published as active.
pub fn status_after_analysis(existing: Option<Status>) -> Status {
    match existing {
        None | Some(Status::Active) => Status::Active,
        Some(other) => other,
    }
}

/// Whether a fresh analysis result should overwrite the stored row's values.
/// A dismissed suggestion is frozen — its values are never touched again.
pub fn analysis_may_update(existing: Option<Status>) -> bool {
    !matches!(existing, Some(Status::Dismissed))
}

/// Fold one "shown but ignored" occurrence into the lifecycle: only an active
/// suggestion counts (an applied/dismissed/retired card is not being shown
/// proactively), and it retires once ignored [`IGNORE_RETIRE_COUNT`] times.
pub fn status_after_ignore(status: Status, shown_count: u32) -> (Status, u32) {
    if status != Status::Active {
        return (status, shown_count);
    }
    let next = shown_count.saturating_add(1);
    if next >= IGNORE_RETIRE_COUNT {
        (Status::Retired, next)
    } else {
        (Status::Active, next)
    }
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

/// Analyze one track's full event history into the suggestions it currently
/// supports (zero, one, or several — one per type at most). `preset` is the
/// track's stored (start_ms, skip_ms) if any, used for the
/// already-covered suppression rule; `now_ms` anchors the recency decay.
pub fn analyze(
    events: &[Event],
    preset: Option<(u64, u64)>,
    duration_ms: u64,
    now_ms: u64,
) -> Vec<Suggestion> {
    if duration_ms == 0 || events.is_empty() {
        return Vec::new();
    }
    let plays = group_plays(events);
    let mut out = Vec::new();
    if let Some(s) = auto_skip_suggestion(&plays, now_ms) {
        out.push(s);
    }
    if let Some(s) = skip_point_suggestion(&plays, preset, now_ms) {
        out.push(s);
    }
    if let Some(s) = start_point_suggestion(&plays, preset, now_ms) {
        out.push(s);
    }
    out
}

/// A reconstructed play: the events of one listening occasion and its most
/// recent timestamp (the recency anchor).
#[derive(Debug, Clone)]
struct Play {
    events: Vec<Event>,
    last_at: u64,
}

impl Play {
    fn weight(&self, now_ms: u64) -> f64 {
        decay_weight(self.last_at, now_ms)
    }

    /// The user bailed out of this play early (a whole-song rejection).
    fn is_rejection(&self) -> bool {
        self.events
            .iter()
            .any(|e| e.kind == EventKind::SkipNext && e.from_ms < REJECTION_WINDOW_MS)
    }

    /// Mid-track positions the user skipped FROM in this play (skip-point
    /// signals): a late `skip_next`, or a mid-track `seek_forward`.
    fn skip_points(&self) -> impl Iterator<Item = u64> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::SkipNext if e.from_ms >= REJECTION_WINDOW_MS => Some(e.from_ms),
            EventKind::SeekForward if e.from_ms >= EARLY_SEEK_WINDOW_MS => Some(e.from_ms),
            _ => None,
        })
    }

    /// Intro-skip destinations in this play (start-point signals): where an
    /// EARLY `seek_forward` landed.
    fn start_targets(&self) -> impl Iterator<Item = u64> + '_ {
        self.events.iter().filter_map(|e| match e.kind {
            EventKind::SeekForward if e.from_ms < EARLY_SEEK_WINDOW_MS => e.to_ms,
            _ => None,
        })
    }
}

/// Group a track's events into plays by wall-clock gap. Events are assumed to
/// share one track; the returned plays are in chronological order.
fn group_plays(events: &[Event]) -> Vec<Play> {
    let mut sorted: Vec<Event> = events.to_vec();
    sorted.sort_by_key(|e| e.created_at);
    let mut plays: Vec<Play> = Vec::new();
    for ev in sorted {
        match plays.last_mut() {
            Some(play) if ev.created_at.saturating_sub(play.last_at) <= PLAY_SESSION_GAP_MS => {
                play.last_at = ev.created_at;
                play.events.push(ev);
            }
            _ => plays.push(Play {
                last_at: ev.created_at,
                events: vec![ev],
            }),
        }
    }
    plays
}

/// Exponential recency decay in [0, 1]; 1.0 for "now", 0.5 at one half-life.
fn decay_weight(at_ms: u64, now_ms: u64) -> f64 {
    let age = now_ms.saturating_sub(at_ms) as f64;
    0.5_f64.powf(age / DECAY_HALF_LIFE_MS as f64)
}

/// Round a millisecond value to the nearest whole second (suggested times are
/// always second-granular).
fn round_to_second(ms: f64) -> u64 {
    ((ms / 1000.0).round() as u64) * 1000
}

/// Whole-song auto-skip: the user rejects this track in an overwhelming,
/// recency-weighted majority of plays.
fn auto_skip_suggestion(plays: &[Play], now_ms: u64) -> Option<Suggestion> {
    if (plays.len() as u32) < AUTO_SKIP_MIN_PLAYS {
        return None;
    }
    let mut total_w = 0.0;
    let mut reject_w = 0.0;
    let mut reject_n: u32 = 0;
    for play in plays {
        let w = play.weight(now_ms);
        total_w += w;
        if play.is_rejection() {
            reject_w += w;
            reject_n += 1;
        }
    }
    if total_w <= 0.0 || reject_w / total_w < AUTO_SKIP_MIN_RATIO {
        return None;
    }
    Some(Suggestion {
        suggestion_type: SuggestionType::AutoSkip,
        value_start_ms: None,
        value_end_ms: None,
        plays_total: plays.len() as u32,
        plays_matching: reject_n,
    })
}

/// Skip point: among plays the user did NOT reject early (they engaged with
/// the track), a consistent mid-track exit cluster.
fn skip_point_suggestion(
    plays: &[Play],
    preset: Option<(u64, u64)>,
    now_ms: u64,
) -> Option<Suggestion> {
    let engaged: Vec<&Play> = plays.iter().filter(|p| !p.is_rejection()).collect();
    if (engaged.len() as u32) < MIN_PLAYS {
        return None;
    }
    // Every skip point across engaged plays, weighted by its play's recency.
    let points: Vec<(u64, f64)> = engaged
        .iter()
        .flat_map(|p| {
            let w = p.weight(now_ms);
            p.skip_points().map(move |pos| (pos, w))
        })
        .collect();
    let cluster = dominant_cluster(&points)?;

    // A play matches if any of its skip points sits inside the cluster.
    let mut total_w = 0.0;
    let mut match_w = 0.0;
    let mut match_n: u32 = 0;
    for play in &engaged {
        let w = play.weight(now_ms);
        total_w += w;
        if play.skip_points().any(|pos| cluster.contains(pos)) {
            match_w += w;
            match_n += 1;
        }
    }
    if total_w <= 0.0 || match_w / total_w < MIN_MATCH_RATIO {
        return None;
    }

    let region_start = round_to_second(cluster.min as f64);
    let region_end = round_to_second(cluster.max as f64);
    // Already covered: a stored preset that skips at or before the region.
    if let Some((_, skip_ms)) = preset {
        if skip_ms <= region_end {
            return None;
        }
    }
    Some(Suggestion {
        suggestion_type: SuggestionType::SkipPoint,
        value_start_ms: Some(region_start),
        value_end_ms: Some(region_end),
        plays_total: engaged.len() as u32,
        plays_matching: match_n,
    })
}

/// Start point: a consistent intro-skip landing spot across plays.
fn start_point_suggestion(
    plays: &[Play],
    preset: Option<(u64, u64)>,
    now_ms: u64,
) -> Option<Suggestion> {
    if (plays.len() as u32) < MIN_PLAYS {
        return None;
    }
    let targets: Vec<(u64, f64)> = plays
        .iter()
        .flat_map(|p| {
            let w = p.weight(now_ms);
            p.start_targets().map(move |t| (t, w))
        })
        .collect();
    let cluster = dominant_cluster(&targets)?;

    let mut total_w = 0.0;
    let mut match_w = 0.0;
    let mut match_n: u32 = 0;
    for play in plays {
        let w = play.weight(now_ms);
        total_w += w;
        if play.start_targets().any(|t| cluster.contains(t)) {
            match_w += w;
            match_n += 1;
        }
    }
    if total_w <= 0.0 || match_w / total_w < MIN_MATCH_RATIO {
        return None;
    }

    let target = round_to_second(cluster.weighted_median);
    // Already covered: a stored preset that already starts at/after the target.
    if let Some((start_ms, _)) = preset {
        if start_ms >= target {
            return None;
        }
    }
    Some(Suggestion {
        suggestion_type: SuggestionType::StartPoint,
        value_start_ms: Some(target),
        value_end_ms: None,
        plays_total: plays.len() as u32,
        plays_matching: match_n,
    })
}

/// The winning cluster of weighted positions.
struct Cluster {
    /// Weighted mean of the members (the recency-pulled centre).
    center: f64,
    /// Weighted median of the members (robust central value).
    weighted_median: f64,
    min: u64,
    max: u64,
}

impl Cluster {
    fn contains(&self, pos: u64) -> bool {
        (pos as f64 - self.center).abs() <= CLUSTER_RADIUS_MS as f64
    }
}

/// Find the densest cluster of weighted positions: the point whose
/// ±[`CLUSTER_RADIUS_MS`] neighborhood carries the most total weight wins;
/// the cluster's members are the positions inside that neighborhood.
fn dominant_cluster(points: &[(u64, f64)]) -> Option<Cluster> {
    if points.is_empty() {
        return None;
    }
    // Each candidate centre is an observed position; pick the one whose window
    // gathers the most weight (ties → the earlier position, so we suggest the
    // earliest well-supported exit).
    let mut best_idx = 0;
    let mut best_weight = f64::NEG_INFINITY;
    for (i, &(center, _)) in points.iter().enumerate() {
        let weight: f64 = points
            .iter()
            .filter(|&&(pos, _)| pos.abs_diff(center) <= CLUSTER_RADIUS_MS)
            .map(|&(_, w)| w)
            .sum();
        if weight > best_weight {
            best_weight = weight;
            best_idx = i;
        }
    }
    let center_pos = points[best_idx].0;
    let members: Vec<(u64, f64)> = points
        .iter()
        .copied()
        .filter(|&(pos, _)| pos.abs_diff(center_pos) <= CLUSTER_RADIUS_MS)
        .collect();

    let total_w: f64 = members.iter().map(|&(_, w)| w).sum();
    if total_w <= 0.0 {
        return None;
    }
    let center = members.iter().map(|&(pos, w)| pos as f64 * w).sum::<f64>() / total_w;
    let min = members.iter().map(|&(pos, _)| pos).min().unwrap_or(0);
    let max = members.iter().map(|&(pos, _)| pos).max().unwrap_or(0);
    Some(Cluster {
        center,
        weighted_median: weighted_median(&members),
        min,
        max,
    })
}

/// Weighted median of positions: the smallest position whose cumulative weight
/// reaches half of the total.
fn weighted_median(members: &[(u64, f64)]) -> f64 {
    let mut sorted = members.to_vec();
    sorted.sort_by_key(|&(pos, _)| pos);
    let total: f64 = sorted.iter().map(|&(_, w)| w).sum();
    let mut cumulative = 0.0;
    for &(pos, w) in &sorted {
        cumulative += w;
        if cumulative >= total / 2.0 {
            return pos as f64;
        }
    }
    sorted.last().map(|&(pos, _)| pos as f64).unwrap_or(0.0)
}

/// Current unix time in ms (for the analysis anchor at call sites that have no
/// external clock). Returns 0 on a pre-1970 clock rather than panicking.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: u64 = 24 * 60 * 60 * 1_000;
    const DURATION: u64 = 200_000;
    /// A fixed "now" far enough from epoch that ages never underflow.
    const NOW: u64 = 1_000 * DAY_MS;

    fn ev(kind: EventKind, from_ms: u64, to_ms: Option<u64>, created_at: u64) -> Event {
        Event {
            kind,
            from_ms,
            to_ms,
            created_at,
        }
    }

    /// One recent play per index, spaced a day apart so each is its own
    /// session; index 0 is the oldest.
    fn spaced_at(index: u64) -> u64 {
        NOW - (20 - index) * DAY_MS
    }

    fn of_type(out: &[Suggestion], t: SuggestionType) -> Option<Suggestion> {
        out.iter().copied().find(|s| s.suggestion_type == t)
    }

    // -- classification: the 15 s rejection boundary --------------------------

    #[test]
    fn skip_at_14s_is_a_rejection_not_a_skip_point() {
        // Ten plays, each a lone early skip at 14 s → auto-skip, never a
        // skip point (14 s < 15 s rejection window).
        let events: Vec<Event> = (0..10)
            .map(|i| ev(EventKind::SkipNext, 14_000, None, spaced_at(i)))
            .collect();
        let out = analyze(&events, None, DURATION, NOW);
        assert!(of_type(&out, SuggestionType::AutoSkip).is_some());
        assert!(of_type(&out, SuggestionType::SkipPoint).is_none());
    }

    #[test]
    fn skip_at_16s_is_a_skip_point_not_a_rejection() {
        // Ten plays, each a lone skip at 16 s → skip point, never a rejection.
        let events: Vec<Event> = (0..10)
            .map(|i| ev(EventKind::SkipNext, 16_000, None, spaced_at(i)))
            .collect();
        let out = analyze(&events, None, DURATION, NOW);
        assert!(of_type(&out, SuggestionType::AutoSkip).is_none());
        let skip = of_type(&out, SuggestionType::SkipPoint).expect("skip point");
        assert_eq!(skip.value_start_ms, Some(16_000));
    }

    // -- clustering ±5 s ------------------------------------------------------

    #[test]
    fn skips_within_the_radius_form_one_cluster() {
        // Positions 70/72/74 s (within ±5 s) across 6 plays cluster together.
        let positions = [70_000, 72_000, 74_000, 71_000, 73_000, 72_000];
        let events: Vec<Event> = positions
            .iter()
            .enumerate()
            .map(|(i, &p)| ev(EventKind::SkipNext, p, None, spaced_at(i as u64)))
            .collect();
        let skip = of_type(
            &analyze(&events, None, DURATION, NOW),
            SuggestionType::SkipPoint,
        )
        .expect("skip point");
        assert_eq!(skip.plays_total, 6);
        assert_eq!(skip.plays_matching, 6);
        assert_eq!(skip.value_start_ms, Some(70_000));
        assert_eq!(skip.value_end_ms, Some(74_000));
    }

    #[test]
    fn a_lone_far_outlier_does_not_join_the_cluster() {
        // Five tight skips at ~90 s plus one at 30 s: the outlier is not a
        // match, so 5/6 ≈ 83 % — still above 70 %.
        let mut events: Vec<Event> = (0..5)
            .map(|i| ev(EventKind::SkipNext, 90_000, None, spaced_at(i)))
            .collect();
        events.push(ev(EventKind::SkipNext, 30_000, None, spaced_at(5)));
        let skip = of_type(
            &analyze(&events, None, DURATION, NOW),
            SuggestionType::SkipPoint,
        )
        .expect("skip point");
        assert_eq!(skip.plays_total, 6);
        assert_eq!(skip.plays_matching, 5);
        assert_eq!(skip.value_start_ms, Some(90_000));
    }

    // -- time-decay ordering --------------------------------------------------

    #[test]
    fn recent_plays_pull_the_suggested_value() {
        // Old plays skip at 60 s, recent plays at 100 s. Both clusters have 5
        // plays, but recency weighting favors the newer 100 s cluster.
        let mut events = Vec::new();
        for i in 0..5 {
            // Old: ~1.5 half-lives ago.
            events.push(ev(
                EventKind::SkipNext,
                60_000,
                None,
                NOW - 135 * DAY_MS - i * DAY_MS,
            ));
        }
        for i in 0..5 {
            // Recent: within days.
            events.push(ev(EventKind::SkipNext, 100_000, None, NOW - i * DAY_MS));
        }
        let skip = of_type(
            &analyze(&events, None, DURATION, NOW),
            SuggestionType::SkipPoint,
        )
        .expect("skip point");
        assert_eq!(
            skip.value_start_ms,
            Some(100_000),
            "the recent cluster wins"
        );
    }

    // -- 70 % / 5-play threshold, just below and just above -------------------

    #[test]
    fn skip_point_fires_at_four_of_five_but_not_three_of_five() {
        // 5 engaged plays, 4 skip near 80 s, 1 skips near 120 s → 80 % ≥ 70 %.
        let mut fires: Vec<Event> = (0..4)
            .map(|i| ev(EventKind::SkipNext, 80_000, None, spaced_at(i)))
            .collect();
        fires.push(ev(EventKind::SkipNext, 120_000, None, spaced_at(4)));
        assert!(of_type(
            &analyze(&fires, None, DURATION, NOW),
            SuggestionType::SkipPoint
        )
        .is_some());

        // 3 near 80 s, 2 near 120 s → the 80 s cluster is only 60 % < 70 %.
        let mut nope: Vec<Event> = (0..3)
            .map(|i| ev(EventKind::SkipNext, 80_000, None, spaced_at(i)))
            .collect();
        nope.push(ev(EventKind::SkipNext, 120_000, None, spaced_at(3)));
        nope.push(ev(EventKind::SkipNext, 121_000, None, spaced_at(4)));
        // 80 s cluster: 3/5 = 60 %; 120 s cluster: 2/5 = 40 %. Neither ≥ 70 %.
        assert!(of_type(
            &analyze(&nope, None, DURATION, NOW),
            SuggestionType::SkipPoint
        )
        .is_none());
    }

    #[test]
    fn fewer_than_five_plays_never_fires_even_at_100_percent() {
        let events: Vec<Event> = (0..4)
            .map(|i| ev(EventKind::SkipNext, 80_000, None, spaced_at(i)))
            .collect();
        assert!(of_type(
            &analyze(&events, None, DURATION, NOW),
            SuggestionType::SkipPoint
        )
        .is_none());
    }

    // -- auto-skip 90 % / 10-play threshold -----------------------------------

    #[test]
    fn auto_skip_needs_ten_plays_and_ninety_percent() {
        // 9 rejections is not enough plays.
        let nine: Vec<Event> = (0..9)
            .map(|i| ev(EventKind::SkipNext, 5_000, None, spaced_at(i)))
            .collect();
        assert!(of_type(
            &analyze(&nine, None, DURATION, NOW),
            SuggestionType::AutoSkip
        )
        .is_none());

        // 10 rejections, all early → fires.
        let ten: Vec<Event> = (0..10)
            .map(|i| ev(EventKind::SkipNext, 5_000, None, spaced_at(i)))
            .collect();
        let auto = of_type(
            &analyze(&ten, None, DURATION, NOW),
            SuggestionType::AutoSkip,
        )
        .expect("auto-skip");
        assert_eq!(auto.plays_total, 10);
        assert_eq!(auto.plays_matching, 10);

        // 10 plays but only 8 rejections (80 %) < 90 % → no auto-skip.
        let mut mixed: Vec<Event> = (0..8)
            .map(|i| ev(EventKind::SkipNext, 5_000, None, spaced_at(i)))
            .collect();
        mixed.push(ev(EventKind::SkipNext, 90_000, None, spaced_at(8)));
        mixed.push(ev(EventKind::SkipNext, 90_000, None, spaced_at(9)));
        assert!(of_type(
            &analyze(&mixed, None, DURATION, NOW),
            SuggestionType::AutoSkip
        )
        .is_none());
    }

    // -- start point ----------------------------------------------------------

    #[test]
    fn consistent_intro_skips_suggest_a_start_point() {
        // Early seek_forward to ~30 s in 6 plays.
        let events: Vec<Event> = (0..6)
            .map(|i| ev(EventKind::SeekForward, 3_000, Some(30_000), spaced_at(i)))
            .collect();
        let start = of_type(
            &analyze(&events, None, DURATION, NOW),
            SuggestionType::StartPoint,
        )
        .expect("start point");
        assert_eq!(start.value_start_ms, Some(30_000));
        assert_eq!(start.value_end_ms, None);
    }

    // -- existing-preset suppression ------------------------------------------

    #[test]
    fn a_preset_already_covering_the_region_suppresses_the_skip_point() {
        let events: Vec<Event> = (0..6)
            .map(|i| ev(EventKind::SkipNext, 90_000, None, spaced_at(i)))
            .collect();
        // Preset already skips at 80 s (before the 90 s region) → suppressed.
        assert!(of_type(
            &analyze(&events, Some((0, 80_000)), DURATION, NOW),
            SuggestionType::SkipPoint
        )
        .is_none());
        // A preset that skips LATER (at the very end) does not cover it.
        assert!(of_type(
            &analyze(&events, Some((0, DURATION)), DURATION, NOW),
            SuggestionType::SkipPoint
        )
        .is_some());
    }

    #[test]
    fn a_preset_already_starting_late_suppresses_the_start_point() {
        let events: Vec<Event> = (0..6)
            .map(|i| ev(EventKind::SeekForward, 3_000, Some(30_000), spaced_at(i)))
            .collect();
        // Preset already starts at 40 s (past the 30 s target) → suppressed.
        assert!(of_type(
            &analyze(&events, Some((40_000, DURATION)), DURATION, NOW),
            SuggestionType::StartPoint
        )
        .is_none());
    }

    #[test]
    fn seek_back_and_late_seek_are_ignored_for_start_points() {
        // Replays (seek_back) and a late forward seek are not intro skips.
        let mut events: Vec<Event> = (0..6)
            .map(|i| ev(EventKind::SeekBack, 120_000, Some(60_000), spaced_at(i)))
            .collect();
        events.push(ev(
            EventKind::SeekForward,
            90_000,
            Some(150_000),
            spaced_at(6),
        ));
        assert!(analyze(&events, None, DURATION, NOW).is_empty());
    }

    // -- sessionization -------------------------------------------------------

    #[test]
    fn events_close_in_time_are_one_play() {
        // Two events seconds apart = one play; four such plays = 4 plays < 5.
        let mut events = Vec::new();
        for i in 0..4 {
            let base = spaced_at(i);
            events.push(ev(EventKind::SeekForward, 3_000, Some(30_000), base));
            events.push(ev(EventKind::SkipNext, 90_000, None, base + 60_000));
        }
        // 8 events but only 4 plays → below MIN_PLAYS for either type.
        assert!(analyze(&events, None, DURATION, NOW).is_empty());
    }

    // -- lifecycle state machine ----------------------------------------------

    #[test]
    fn analysis_never_resurrects_a_dismissed_suggestion() {
        assert_eq!(status_after_analysis(None), Status::Active);
        assert_eq!(status_after_analysis(Some(Status::Active)), Status::Active);
        assert_eq!(
            status_after_analysis(Some(Status::Dismissed)),
            Status::Dismissed
        );
        assert_eq!(
            status_after_analysis(Some(Status::Applied)),
            Status::Applied
        );
        assert_eq!(
            status_after_analysis(Some(Status::Retired)),
            Status::Retired
        );
        assert!(!analysis_may_update(Some(Status::Dismissed)));
        assert!(analysis_may_update(Some(Status::Active)));
        assert!(analysis_may_update(None));
    }

    #[test]
    fn three_ignores_retire_an_active_card_but_not_other_states() {
        let (s1, n1) = status_after_ignore(Status::Active, 0);
        assert_eq!((s1, n1), (Status::Active, 1));
        let (s2, n2) = status_after_ignore(s1, n1);
        assert_eq!((s2, n2), (Status::Active, 2));
        let (s3, n3) = status_after_ignore(s2, n2);
        assert_eq!((s3, n3), (Status::Retired, 3));
        // Applied / dismissed never count as "shown and ignored".
        assert_eq!(
            status_after_ignore(Status::Applied, 0),
            (Status::Applied, 0)
        );
        assert_eq!(
            status_after_ignore(Status::Dismissed, 1),
            (Status::Dismissed, 1)
        );
    }

    #[test]
    fn type_and_status_strings_round_trip() {
        for t in [
            SuggestionType::SkipPoint,
            SuggestionType::StartPoint,
            SuggestionType::AutoSkip,
        ] {
            assert_eq!(SuggestionType::from_str(t.as_str()), Some(t));
        }
        for s in [
            Status::Active,
            Status::Applied,
            Status::Dismissed,
            Status::Retired,
        ] {
            assert_eq!(Status::from_str(s.as_str()), Some(s));
        }
        assert_eq!(SuggestionType::from_str("nope"), None);
        assert_eq!(Status::from_str("nope"), None);
    }
}
