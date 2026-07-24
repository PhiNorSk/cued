/**
 * Pure preset logic: validation rules, handle clamping and m:ss parsing.
 * The same validation rules live authoritatively in Rust (presets.rs) —
 * keep both in sync.
 */

/** Minimum distance between the start and skip points. */
export const MIN_GAP_MS = 10_000;

/** Keyboard step for a focused handle (arrow keys). */
export const HANDLE_STEP_MS = 1_000;

/** Keyboard step with shift held. */
export const HANDLE_STEP_LARGE_MS = 5_000;

/** True when the preset satisfies 0 <= start < skip <= duration, gap >= 10 s. */
export function isValidPreset(
  startMs: number,
  skipMs: number,
  durationMs: number,
): boolean {
  return (
    Number.isInteger(startMs) &&
    Number.isInteger(skipMs) &&
    Number.isInteger(durationMs) &&
    startMs >= 0 &&
    skipMs <= durationMs &&
    skipMs - startMs >= MIN_GAP_MS
  );
}

/** Clamp a desired START value to [0, skip - MIN_GAP_MS]. */
export function clampStartMs(desiredMs: number, skipMs: number): number {
  return Math.min(Math.max(desiredMs, 0), Math.max(skipMs - MIN_GAP_MS, 0));
}

/** Clamp a desired SKIP value to [start + MIN_GAP_MS, duration]. */
export function clampSkipMs(
  desiredMs: number,
  startMs: number,
  durationMs: number,
): number {
  return Math.max(
    Math.min(desiredMs, durationMs),
    Math.min(startMs + MIN_GAP_MS, durationMs),
  );
}

/** Pre-roll before a previewed point so the ear catches the transition. */
export const PREVIEW_PREROLL_MS = 3_000;

/**
 * Minimum time between preview seeks — mirrors the engine's 2 s action
 * cooldown (ACTION_COOLDOWN_MS in automation.rs) so edit-mode previews
 * follow the same rate discipline as automation actions.
 */
export const PREVIEW_COOLDOWN_MS = 2_000;

/** Where a preview of `pointMs` seeks to: the pre-roll earlier, floored at 0. */
export function previewTargetMs(pointMs: number): number {
  return Math.max(0, pointMs - PREVIEW_PREROLL_MS);
}

/** True once the preview cooldown has elapsed (null = never previewed). */
export function canPreviewAgain(
  lastPreviewAtMs: number | null,
  nowMs: number,
): boolean {
  return (
    lastPreviewAtMs === null || nowMs - lastPreviewAtMs >= PREVIEW_COOLDOWN_MS
  );
}

/**
 * True when the draft equals the neutral state (the whole track plays).
 * A neutral draft is "no preset", never a row — the same rule is enforced
 * authoritatively in Rust (presets.rs rejects neutral writes).
 */
export function isNeutralPreset(
  startMs: number,
  skipMs: number,
  durationMs: number,
): boolean {
  return startMs === 0 && skipMs === durationMs;
}

/** What pressing "Save preset" should do with the current draft. */
export type SaveAction = "save" | "delete" | "nothing";

/**
 * Decide the save action: a neutral draft saves nothing — unless a preset
 * is stored, in which case saving it back to neutral deletes that preset.
 */
export function saveActionFor(
  startMs: number,
  skipMs: number,
  durationMs: number,
  hasStored: boolean,
): SaveAction {
  if (!isNeutralPreset(startMs, skipMs, durationMs)) {
    return "save";
  }
  return hasStored ? "delete" : "nothing";
}

/**
 * Where playback should seek back to when leaving edit mode — only when the
 * remembered track is still the one playing; null means "do nothing".
 */
export function restoreTargetMs(
  rememberedUri: string,
  rememberedPositionMs: number,
  currentUri: string | null,
): number | null {
  return currentUri === rememberedUri ? rememberedPositionMs : null;
}

/**
 * Below this distance (in % of the timeline) the two flag chips would
 * overlap, so they anchor away from each other instead of centering.
 */
export const FLAG_REPEL_BELOW_PCT = 16;

/** True when the start/skip flag chips are close enough to collide. */
export function flagsRepel(startPct: number, skipPct: number): boolean {
  return skipPct - startPct < FLAG_REPEL_BELOW_PCT;
}

/** Round to the nearest whole second (drag granularity). */
export function snapMsToSecond(ms: number): number {
  return Math.round(ms / 1000) * 1000;
}

/** Map a pointer x-coordinate on the timeline to a position in ms. */
export function timelineMsFromPointer(
  clientX: number,
  rectLeft: number,
  rectWidth: number,
  durationMs: number,
): number {
  if (rectWidth <= 0) {
    return 0;
  }
  const unit = Math.min(1, Math.max(0, (clientX - rectLeft) / rectWidth));
  return Math.round(unit * durationMs);
}

/** Parse strict `m:ss` text (e.g. "3:05") to ms; null on any bad input. */
export function parseTimeMs(text: string): number | null {
  const match = /^(\d+):([0-5]\d)$/.exec(text.trim());
  if (match === null) {
    return null;
  }
  return Number(match[1]) * 60_000 + Number(match[2]) * 1000;
}

/** Case-insensitive Library search over title and artists; "" matches all. */
export function presetMatchesQuery(
  title: string,
  artists: string[],
  query: string,
): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") {
    return true;
  }
  return (
    title.toLowerCase().includes(needle) ||
    artists.some((artist) => artist.toLowerCase().includes(needle))
  );
}
