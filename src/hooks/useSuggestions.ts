import { useCallback, useEffect, useRef, useState } from "react";
import { getInsightsEnabled } from "../lib/insights";
import {
  acceptSuggestion,
  analyzeSuggestions,
  dismissSuggestion,
  getSuggestionToggles,
  getTrackSuggestions,
  ignoreSuggestion,
  pickCardSuggestion,
  undoSuggestion,
  type AcceptResult,
  type Suggestion,
  type SuggestionToggles,
  type SuggestionType,
} from "../lib/suggestions";
import type { PresetInput } from "../lib/presets";

const ALL_ON: SuggestionToggles = {
  skipPoints: true,
  startPoints: true,
  autoSkip: true,
};

export interface UseSuggestionsResult {
  /** The single proactive card to surface, or null. */
  card: Suggestion | null;
  /** Accept a suggestion; returns the result so the caller can offer Undo. */
  accept: (type: SuggestionType) => Promise<AcceptResult | null>;
  /** Undo an accepted suggestion, restoring `previous` (or removing it). */
  undo: (type: SuggestionType, previous: PresetInput | null) => Promise<void>;
  /** "No thanks" — never surface this type for this track again. */
  dismiss: (type: SuggestionType) => Promise<void>;
}

/**
 * Suggestion state for the currently playing track (M10). On every track
 * change it runs the (debounced, bounded) analysis pass, then loads this
 * track's suggestions and the gating settings, and exposes the ONE proactive
 * card to show. Leaving a track whose card was shown-but-not-acted-on reports
 * it ignored (three ignores retire it — handled in the engine). Accept/dismiss
 * update optimistically so the card responds instantly.
 */
export function useSuggestions(trackUri: string | null): UseSuggestionsResult {
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [toggles, setToggles] = useState<SuggestionToggles>(ALL_ON);
  const [insightsOn, setInsightsOn] = useState(true);

  // Load the gating settings once; cheap and rarely changes mid-session.
  useEffect(() => {
    let cancelled = false;
    void Promise.allSettled([getSuggestionToggles(), getInsightsEnabled()]).then(
      ([t, on]) => {
        if (cancelled) return;
        if (t.status === "fulfilled") setToggles(t.value);
        if (on.status === "fulfilled") setInsightsOn(on.value);
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  // Per track: analyze, then load this track's suggestions.
  useEffect(() => {
    setSuggestions([]);
    if (trackUri === null) {
      return;
    }
    let cancelled = false;
    void (async () => {
      // Analysis is opportunistic and best-effort — a failure just means no
      // fresh suggestion this time, never a broken card.
      await analyzeSuggestions().catch(() => undefined);
      const list = await getTrackSuggestions(trackUri).catch(
        () => [] as Suggestion[],
      );
      if (!cancelled) setSuggestions(list);
    })();
    return () => {
      cancelled = true;
    };
  }, [trackUri]);

  // Only ever consider suggestions for the track on screen right now. This
  // also makes the transient render after a track change (old suggestions,
  // new URI) resolve to no card, which keeps the "ignored on track change"
  // bookkeeping below correct.
  const forTrack = suggestions.filter((s) => s.trackUri === trackUri);
  const card = pickCardSuggestion(forTrack, toggles, insightsOn);
  const cardType = card?.type ?? null;

  // Remember the card currently on screen and whether the user has acted on it.
  const shownRef = useRef<{ uri: string; type: SuggestionType } | null>(null);
  const actedRef = useRef(false);
  useEffect(() => {
    if (cardType !== null && trackUri !== null) {
      shownRef.current = { uri: trackUri, type: cardType };
    }
  }, [cardType, trackUri]);

  // Count "shown but ignored" only on a genuine TRACK CHANGE (not on a tab
  // switch / unmount, and not on a plain refetch): the song moved on while the
  // card sat there untouched. Three of these retire it (handled in the engine).
  const prevUriRef = useRef<string | null>(trackUri);
  useEffect(() => {
    const prev = prevUriRef.current;
    prevUriRef.current = trackUri;
    if (
      prev !== null &&
      prev !== trackUri &&
      shownRef.current !== null &&
      shownRef.current.uri === prev &&
      !actedRef.current
    ) {
      void ignoreSuggestion(shownRef.current.uri, shownRef.current.type).catch(
        () => undefined,
      );
    }
    actedRef.current = false;
    shownRef.current = null;
  }, [trackUri]);

  const accept = useCallback(
    async (type: SuggestionType): Promise<AcceptResult | null> => {
      if (trackUri === null) return null;
      // Mark acted (so leaving the track never counts it as ignored) but keep
      // the suggestion in local state: the card stays mounted and shows its
      // own applied → Undo morph. The backend row is already `applied`; a
      // later refetch (next track) reflects that.
      actedRef.current = true;
      return acceptSuggestion(trackUri, type);
    },
    [trackUri],
  );

  const undo = useCallback(
    async (type: SuggestionType, previous: PresetInput | null): Promise<void> => {
      if (trackUri === null) return;
      await undoSuggestion(trackUri, type, previous);
    },
    [trackUri],
  );

  const dismiss = useCallback(
    async (type: SuggestionType): Promise<void> => {
      if (trackUri === null) return;
      actedRef.current = true;
      await dismissSuggestion(trackUri, type);
      setSuggestions((prev) => prev.filter((s) => s.type !== type));
    },
    [trackUri],
  );

  return { card, accept, undo, dismiss };
}
