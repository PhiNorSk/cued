import { z } from "zod";

/**
 * How long the UI keeps showing the last known track after playback reports
 * "nothing playing", so brief gaps between polls (e.g. between songs) never
 * flash the empty state.
 */
export const EMPTY_STATE_DEBOUNCE_MS = 3000;

/** How often the UI re-renders to advance the interpolated progress bar. */
export const UI_TICK_MS = 250;

/** Track (or podcast episode) currently loaded in the user's Spotify player. */
export const trackInfoSchema = z.object({
  /** Stable identity (Spotify URI) — used to detect track changes. */
  uri: z.string(),
  title: z.string(),
  artists: z.array(z.string()),
  coverUrl: z.string().nullable(),
  durationMs: z.number().nonnegative(),
  isLocal: z.boolean(),
  kind: z.enum(["track", "episode"]),
});
export type TrackInfo = z.infer<typeof trackInfoSchema>;

/** Why the auto-skip engine cannot act right now (engine-reported). */
export const automationSuspensionSchema = z.enum([
  "noPremium",
  "restrictedDevice",
  "rateLimited",
]);
export type AutomationSuspension = z.infer<typeof automationSuspensionSchema>;

/** Payload of the `playback://state` event pushed by the Rust poll engine. */
export const playbackStateSchema = z.object({
  status: z.enum(["playing", "paused", "idle", "suspended", "authLost"]),
  /** True while the engine is backing off after transient network errors. */
  reconnecting: z.boolean(),
  track: trackInfoSchema.nullable(),
  positionMs: z.number().nullable(),
  /** Unix ms at which `positionMs` was sampled — interpolation baseline. */
  fetchedAtMs: z.number(),
  /** Why automation cannot act right now (null = it can). */
  automationSuspended: automationSuspensionSchema.nullable(),
});
export type PlaybackState = z.infer<typeof playbackStateSchema>;

/** What the Now Playing card should render right now. */
export type NowPlayingView =
  | { kind: "pending" }
  | { kind: "empty" }
  | { kind: "track"; state: PlaybackState; frozen: boolean };

/**
 * Current playback position, extrapolated from the last engine snapshot.
 * Advances with wall-clock time only while actually playing (never while
 * paused or reconnecting) and clamps to the track duration.
 */
export function interpolatePositionMs(
  state: PlaybackState,
  nowMs: number,
): number | null {
  if (state.positionMs === null) {
    return null;
  }
  const durationMs = state.track?.durationMs ?? Number.POSITIVE_INFINITY;
  if (state.status !== "playing" || state.reconnecting) {
    return Math.min(state.positionMs, durationMs);
  }
  const elapsedMs = Math.max(0, nowMs - state.fetchedAtMs);
  return Math.min(durationMs, state.positionMs + elapsedMs);
}

/**
 * Decide what the Now Playing card shows: the live track, the last known
 * track (frozen, while an idle gap is still within the debounce window),
 * the calm empty state, or a pending placeholder before the first event.
 */
export function resolveNowPlayingView(
  current: PlaybackState | null,
  lastWithTrack: PlaybackState | null,
  idleSinceMs: number | null,
  nowMs: number,
): NowPlayingView {
  if (current === null) {
    return { kind: "pending" };
  }
  if (current.status === "authLost") {
    return { kind: "empty" };
  }
  if (current.track !== null) {
    return { kind: "track", state: current, frozen: false };
  }
  const withinDebounce =
    lastWithTrack !== null &&
    idleSinceMs !== null &&
    nowMs - idleSinceMs < EMPTY_STATE_DEBOUNCE_MS;
  if (withinDebounce) {
    return { kind: "track", state: lastWithTrack, frozen: true };
  }
  return { kind: "empty" };
}

/** Format a millisecond position as `m:ss` for the progress time labels. */
export function formatTimeMs(ms: number): string {
  const totalSecs = Math.max(0, Math.floor(ms / 1000));
  const mins = Math.floor(totalSecs / 60);
  const secs = totalSecs % 60;
  return `${mins}:${String(secs).padStart(2, "0")}`;
}
