//! Skip-density heatmap (M11).
//!
//! A PURE module: a track's recorded skip/seek-away positions go in, a
//! fixed-width, peak-normalized density curve comes out — the input the UI
//! draws as the "you usually skip this part" overlay. No I/O, no display
//! metadata; the store (`presets.rs`) reads the events, this turns them into a
//! curve. It shares the analysis engine's event vocabulary and the SAME 15 s
//! song-rejection rule (a rejection is "not this song right now", never a
//! "skip this part" signal — see [`is_skip_away`]).

use crate::suggestions::{Event, EventKind, REJECTION_WINDOW_MS};

/// Horizontal segments the track is bucketed into. Matches the UI's 100-point
/// curve and is deliberately independent of track length, so the overlay reads
/// the same for a 2-minute and a 6-minute song.
pub const HEATMAP_BUCKETS: usize = 100;

/// A curve needs at least this many eligible skip-away events before it is
/// shown. Below it the band collapses — two data points are noise, not a
/// pattern, and a heatmap that whispers from nothing is a lie.
pub const HEATMAP_MIN_EVENTS: u32 = 8;

/// A normalized skip-density curve for one track.
#[derive(Debug, Clone, PartialEq)]
pub struct Heatmap {
    /// [`HEATMAP_BUCKETS`] values in `[0, 1]`, peak-normalized (the densest
    /// bucket is exactly `1.0`). Index 0 is the track start, the last index
    /// the track end.
    pub buckets: Vec<f64>,
    /// How many eligible events fed the curve — the honest evidence count.
    pub event_count: u32,
}

/// The position the user skipped AWAY FROM, if this event is a skip-density
/// signal. A `skip_next` before [`REJECTION_WINDOW_MS`] is a whole-song
/// rejection (excluded, same rule as the suggestion engine); a `seek_back` is
/// a replay, not a skip (excluded). Everything else contributes its `from_ms`.
fn is_skip_away(e: &Event) -> Option<u64> {
    match e.kind {
        EventKind::SkipNext if e.from_ms >= REJECTION_WINDOW_MS => Some(e.from_ms),
        EventKind::SeekForward => Some(e.from_ms),
        _ => None,
    }
}

/// Map a position to its bucket index in `[0, HEATMAP_BUCKETS - 1]`. The exact
/// track end lands in the LAST bucket (never out of range), and any position
/// at or past the duration is clamped there.
fn bucket_index(pos_ms: u64, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    // u128 keeps `pos_ms * HEATMAP_BUCKETS` from overflowing on long tracks.
    let idx = (pos_ms as u128 * HEATMAP_BUCKETS as u128) / duration_ms as u128;
    (idx as usize).min(HEATMAP_BUCKETS - 1)
}

/// Bucket a track's skip-away events into a peak-normalized density curve, or
/// `None` when there is too little data (a zero-length track, or fewer than
/// [`HEATMAP_MIN_EVENTS`] eligible events) — the caller collapses the band.
pub fn compute(events: &[Event], duration_ms: u64) -> Option<Heatmap> {
    if duration_ms == 0 {
        return None;
    }
    let positions: Vec<u64> = events.iter().filter_map(is_skip_away).collect();
    let event_count = positions.len() as u32;
    if event_count < HEATMAP_MIN_EVENTS {
        return None;
    }
    let mut counts = vec![0u32; HEATMAP_BUCKETS];
    for pos in positions {
        counts[bucket_index(pos, duration_ms)] += 1;
    }
    // event_count >= HEATMAP_MIN_EVENTS > 0, so at least one bucket is non-zero.
    let peak = counts.iter().copied().max().unwrap_or(1).max(1);
    let buckets = counts.iter().map(|&c| c as f64 / peak as f64).collect();
    Some(Heatmap {
        buckets,
        event_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURATION: u64 = 200_000;

    fn skip(from_ms: u64) -> Event {
        Event {
            kind: EventKind::SkipNext,
            from_ms,
            to_ms: None,
            created_at: 0,
        }
    }

    fn seek_forward(from_ms: u64, to_ms: u64) -> Event {
        Event {
            kind: EventKind::SeekForward,
            from_ms,
            to_ms: Some(to_ms),
            created_at: 0,
        }
    }

    fn seek_back(from_ms: u64, to_ms: u64) -> Event {
        Event {
            kind: EventKind::SeekBack,
            from_ms,
            to_ms: Some(to_ms),
            created_at: 0,
        }
    }

    // -- bucketing: 100 segments + edge positions -----------------------------

    #[test]
    fn positions_map_to_one_of_a_hundred_buckets() {
        assert_eq!(bucket_index(0, DURATION), 0, "track start → first bucket");
        assert_eq!(
            bucket_index(DURATION, DURATION),
            HEATMAP_BUCKETS - 1,
            "track end → last bucket, never out of range"
        );
        assert_eq!(
            bucket_index(DURATION * 2, DURATION),
            HEATMAP_BUCKETS - 1,
            "past the end is clamped to the last bucket"
        );
        // Midpoint and a quarter land where arithmetic says.
        assert_eq!(bucket_index(DURATION / 2, DURATION), 50);
        assert_eq!(bucket_index(DURATION / 4, DURATION), 25);
        // Just inside the first bucket boundary.
        assert_eq!(bucket_index(DURATION / 100 - 1, DURATION), 0);
        assert_eq!(bucket_index(DURATION / 100, DURATION), 1);
    }

    #[test]
    fn a_zero_length_bucket_index_is_safe() {
        assert_eq!(bucket_index(1_000, 0), 0);
    }

    // -- normalization: peak is exactly 1.0 -----------------------------------

    #[test]
    fn the_densest_bucket_normalizes_to_one() {
        // Eight skips clustered at 100 s (bucket 50) plus one at 40 s
        // (bucket 20): bucket 50 is the peak → 1.0, bucket 20 is 1/8.
        let mut events: Vec<Event> = (0..8).map(|_| skip(100_000)).collect();
        events.push(skip(40_000));
        let hm = compute(&events, DURATION).expect("enough events");
        assert_eq!(hm.buckets.len(), HEATMAP_BUCKETS);
        assert_eq!(hm.event_count, 9);
        assert_eq!(hm.buckets[50], 1.0);
        assert!((hm.buckets[20] - 1.0 / 8.0).abs() < 1e-9);
        assert_eq!(hm.buckets[0], 0.0);
        // Every value stays in range.
        assert!(hm.buckets.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    // -- rejection exclusion (< 15 s skip_next) -------------------------------

    #[test]
    fn early_rejections_are_excluded_from_the_curve() {
        // Eight late skips (skip-points) + five early rejections at 5 s: only
        // the eight count, and none land in the intro bucket.
        let mut events: Vec<Event> = (0..8).map(|_| skip(90_000)).collect();
        events.extend((0..5).map(|_| skip(5_000)));
        let hm = compute(&events, DURATION).expect("enough non-rejection events");
        assert_eq!(hm.event_count, 8, "rejections do not count");
        assert_eq!(
            hm.buckets[bucket_index(5_000, DURATION)],
            0.0,
            "no density where only rejections sit"
        );
    }

    #[test]
    fn a_track_of_only_rejections_has_no_curve() {
        let events: Vec<Event> = (0..20).map(|_| skip(3_000)).collect();
        assert_eq!(compute(&events, DURATION), None);
    }

    // -- seek_back (replay) exclusion -----------------------------------------

    #[test]
    fn rewinds_are_not_skip_signals() {
        let events: Vec<Event> = (0..12).map(|_| seek_back(120_000, 60_000)).collect();
        assert_eq!(compute(&events, DURATION), None, "replays are not skips");
    }

    #[test]
    fn early_intro_seeks_do_count_as_skip_signals() {
        // Skipping the intro forward IS skipping part of the song — a
        // seek_forward is included regardless of position.
        let events: Vec<Event> = (0..8).map(|_| seek_forward(2_000, 30_000)).collect();
        let hm = compute(&events, DURATION).expect("intro skips count");
        assert_eq!(hm.event_count, 8);
        assert_eq!(hm.buckets[bucket_index(2_000, DURATION)], 1.0);
    }

    // -- minimum-event threshold ----------------------------------------------

    #[test]
    fn just_below_the_threshold_collapses_just_above_shows() {
        let below: Vec<Event> = (0..(HEATMAP_MIN_EVENTS - 1))
            .map(|_| skip(80_000))
            .collect();
        assert_eq!(compute(&below, DURATION), None);

        let at: Vec<Event> = (0..HEATMAP_MIN_EVENTS).map(|_| skip(80_000)).collect();
        assert!(compute(&at, DURATION).is_some());
    }

    #[test]
    fn a_zero_length_track_has_no_curve() {
        let events: Vec<Event> = (0..20).map(|_| skip(1_000)).collect();
        assert_eq!(compute(&events, 0), None);
    }
}
