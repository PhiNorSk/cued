import { z } from "zod";
import { call, unitSchema } from "./auth";
import { presetSchema, type PresetInput } from "./presets";

/**
 * Suggestions IPC wrappers + pure surface logic (M10). The analysis lives in
 * Rust (`suggestions.rs`); this module fetches results, applies/reverses them,
 * and holds the pure rules for WHICH suggestion a surface shows. Kept pure and
 * unit-tested so the "strongest one only" and toggle-gating rules never drift.
 */

export const suggestionTypeSchema = z.enum([
  "skip_point",
  "start_point",
  "auto_skip",
]);
export type SuggestionType = z.infer<typeof suggestionTypeSchema>;

export const suggestionStatusSchema = z.enum([
  "active",
  "applied",
  "dismissed",
  "retired",
]);
export type SuggestionStatus = z.infer<typeof suggestionStatusSchema>;

/** One stored suggestion joined with its track's display metadata. */
export const suggestionSchema = z.object({
  trackUri: z.string(),
  type: suggestionTypeSchema,
  status: suggestionStatusSchema,
  shownCount: z.number().nonnegative(),
  valueStartMs: z.number().nonnegative().nullable(),
  valueEndMs: z.number().nonnegative().nullable(),
  playsTotal: z.number().nonnegative(),
  playsMatching: z.number().nonnegative(),
  updatedAt: z.number(),
  title: z.string(),
  artists: z.array(z.string()),
  coverUrl: z.string().nullable(),
  durationMs: z.number().nonnegative(),
});
export type Suggestion = z.infer<typeof suggestionSchema>;

/** The three per-type suggestion toggles (persisted in config.json). */
export const suggestionTogglesSchema = z.object({
  skipPoints: z.boolean(),
  startPoints: z.boolean(),
  autoSkip: z.boolean(),
});
export type SuggestionToggles = z.infer<typeof suggestionTogglesSchema>;

/** What accepting a skip/start suggestion did (for a full undo). */
export const acceptResultSchema = z.object({
  saved: presetSchema.nullable(),
  previous: presetSchema.nullable(),
});
export type AcceptResult = z.infer<typeof acceptResultSchema>;

// -- IPC --------------------------------------------------------------------

/** Run the opportunistic analysis pass (debounced by the caller). */
export async function analyzeSuggestions(): Promise<void> {
  await call("analyze_suggestions", unitSchema);
}

/** Non-dismissed suggestions for one track (Now Playing card input). */
export function getTrackSuggestions(trackUri: string): Promise<Suggestion[]> {
  return call("get_track_suggestions", z.array(suggestionSchema), { trackUri });
}

/** Every non-dismissed suggestion (the Library section). */
export function listSuggestions(): Promise<Suggestion[]> {
  return call("list_suggestions", z.array(suggestionSchema));
}

/** Accept a suggestion; skip/start apply instantly as a preset. */
export function acceptSuggestion(
  trackUri: string,
  suggestionType: SuggestionType,
): Promise<AcceptResult> {
  return call("accept_suggestion", acceptResultSchema, {
    trackUri,
    suggestionType,
  });
}

/** Undo an accepted suggestion; `previous` is the preset to restore (or null). */
export async function undoSuggestion(
  trackUri: string,
  suggestionType: SuggestionType,
  previous: PresetInput | null,
): Promise<void> {
  await call("undo_suggestion", unitSchema, {
    trackUri,
    suggestionType,
    previous,
  });
}

/** "No thanks" / ×: never surface this type for this track again. */
export async function dismissSuggestion(
  trackUri: string,
  suggestionType: SuggestionType,
): Promise<void> {
  await call("dismiss_suggestion", unitSchema, { trackUri, suggestionType });
}

/** Record that a shown proactive card was ignored (track moved on). */
export async function ignoreSuggestion(
  trackUri: string,
  suggestionType: SuggestionType,
): Promise<void> {
  await call("ignore_suggestion", unitSchema, { trackUri, suggestionType });
}

/** Reverse (or re-arm) an applied auto-skip from the Library. */
export async function setAutoSkipApplied(
  trackUri: string,
  applied: boolean,
): Promise<void> {
  await call("set_auto_skip_applied", unitSchema, { trackUri, applied });
}

/** Read the per-type suggestion toggles. */
export function getSuggestionToggles(): Promise<SuggestionToggles> {
  return call("get_suggestion_toggles", suggestionTogglesSchema);
}

/** Persist the per-type suggestion toggles. */
export async function setSuggestionToggles(
  toggles: SuggestionToggles,
): Promise<void> {
  await call("set_suggestion_toggles", unitSchema, { toggles });
}

// -- pure surface logic -----------------------------------------------------

/** Whether a suggestion type is switched on in the per-type toggles. */
export function isTypeEnabled(
  type: SuggestionType,
  toggles: SuggestionToggles,
): boolean {
  switch (type) {
    case "skip_point":
      return toggles.skipPoints;
    case "start_point":
      return toggles.startPoints;
    case "auto_skip":
      return toggles.autoSkip;
  }
}

/** Recency-agnostic evidence strength: matching share of considered plays. */
export function suggestionRatio(s: Suggestion): number {
  return s.playsTotal > 0 ? s.playsMatching / s.playsTotal : 0;
}

/** Tie-break priority when two suggestions are equally strong. */
const TYPE_PRIORITY: Record<SuggestionType, number> = {
  auto_skip: 3,
  skip_point: 2,
  start_point: 1,
};

/**
 * The single proactive card to surface for a track, or null. Only ACTIVE
 * suggestions of an enabled type are candidates, and only when insights are
 * on at all; the strongest (by evidence ratio, then plays, then type) wins —
 * never more than one card, ever.
 */
export function pickCardSuggestion(
  suggestions: Suggestion[],
  toggles: SuggestionToggles,
  insightsOn: boolean,
): Suggestion | null {
  if (!insightsOn) {
    return null;
  }
  const candidates = suggestions.filter(
    (s) => s.status === "active" && isTypeEnabled(s.type, toggles),
  );
  let best: Suggestion | null = null;
  for (const s of candidates) {
    if (best === null || strongerThan(s, best)) {
      best = s;
    }
  }
  return best;
}

function strongerThan(a: Suggestion, b: Suggestion): boolean {
  const ra = suggestionRatio(a);
  const rb = suggestionRatio(b);
  if (ra !== rb) return ra > rb;
  if (a.playsTotal !== b.playsTotal) return a.playsTotal > b.playsTotal;
  return TYPE_PRIORITY[a.type] > TYPE_PRIORITY[b.type];
}

/**
 * Which suggestions the Library section lists. Active/retired suggestions
 * appear only when insights are on and their type is enabled; an APPLIED
 * auto-skip ALWAYS appears (so the user can always find and reverse it —
 * music must never disappear with no way back). Applied skip/start
 * suggestions are omitted: they are ordinary presets now.
 */
export function librarySuggestions(
  suggestions: Suggestion[],
  toggles: SuggestionToggles,
  insightsOn: boolean,
): Suggestion[] {
  return suggestions.filter((s) => {
    if (s.type === "auto_skip" && s.status === "applied") {
      return true;
    }
    if (s.status !== "active" && s.status !== "retired") {
      return false;
    }
    return insightsOn && isTypeEnabled(s.type, toggles);
  });
}
