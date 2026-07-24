import { useCallback, useEffect, useRef, useState } from "react";
import { friendlyAuthMessage } from "../lib/errorCopy";
import type { TrackInfo } from "../lib/playback";
import { setEditMode, uiSeek } from "../lib/player";
import {
  canPreviewAgain,
  clampSkipMs,
  clampStartMs,
  isValidPreset,
  previewTargetMs,
  restoreTargetMs,
  saveActionFor,
  type SaveAction,
} from "../lib/presetLogic";
import {
  deletePreset,
  getPreset,
  savePreset,
  type Preset,
} from "../lib/presets";

/** Where the preset editor currently stands for the loaded track. */
export type PresetPhase =
  | "loading"
  | "ready"
  | "saving"
  | "saved"
  | "removed"
  | "error";

/** The two editable points, as currently shown by the handles. */
export interface PresetDraft {
  startMs: number;
  skipMs: number;
}

/** One edit-mode session: which track, and where to seek back on exit. */
interface EditSession {
  uri: string;
  /** Playback position remembered on entry; null = nothing to restore. */
  returnPositionMs: number | null;
}

export interface UsePresetResult {
  /** Handle positions to render; null while the stored preset is loading. */
  draft: PresetDraft | null;
  /** The persisted preset for this track, if any. */
  stored: Preset | null;
  /** True when the draft differs from what save would be a no-op against. */
  dirty: boolean;
  /** True while edit mode is open (engine automation gated for this track). */
  editing: boolean;
  /** What pressing "Save preset" would do with the current draft. */
  saveAction: SaveAction;
  /** Wall-clock ms of the last preview seek (preview cooldown input). */
  lastPreviewAtMs: number | null;
  phase: PresetPhase;
  /** Display-ready message when phase is "error". */
  error: string | null;
  /** Move the START handle (clamped against the skip point). */
  setStart: (ms: number) => void;
  /** Move the SKIP handle (clamped against start and duration). */
  setSkip: (ms: number) => void;
  /** Enter edit mode, remembering `positionMs` to seek back to on exit. */
  enterEdit: (positionMs: number | null) => void;
  /** Discard the draft entirely, restore the position, leave edit mode. */
  cancelEdit: () => void;
  /** Persist (or, for a neutral draft, delete) the preset and leave edit mode. */
  save: () => void;
  /** Audition `pointMs` by ear: seek to the pre-roll before it (debounced). */
  preview: (pointMs: number) => void;
  /** Adopt a preset just saved elsewhere (M10 accept), so Adjust prefills it. */
  adoptSaved: (preset: Preset) => void;
}

function defaultDraft(track: TrackInfo): PresetDraft {
  return { startMs: 0, skipMs: track.durationMs };
}

function draftOf(preset: Preset): PresetDraft {
  return { startMs: preset.startMs, skipMs: preset.skipMs };
}

/**
 * Preset editing state for the currently playing track: loads the stored
 * preset when the track changes, owns the edit-mode session (engine gate,
 * preview seeks, exit-restore), tracks the dragged draft, and saves or —
 * for a draft moved back to neutral — deletes it. Pass null when presets
 * don't apply (episode, local file) — everything stays inert then.
 */
export function usePreset(track: TrackInfo | null): UsePresetResult {
  const [draft, setDraft] = useState<PresetDraft | null>(null);
  const [stored, setStored] = useState<Preset | null>(null);
  const [phase, setPhase] = useState<PresetPhase>("loading");
  const [error, setError] = useState<string | null>(null);
  const [session, setSession] = useState<EditSession | null>(null);
  const [lastPreviewAtMs, setLastPreviewAtMs] = useState<number | null>(null);

  // Guards async results against track changes mid-flight.
  const uriRef = useRef<string | null>(null);
  uriRef.current = track?.uri ?? null;

  const uri = track?.uri ?? null;
  const durationMs = track?.durationMs ?? 0;

  useEffect(() => {
    setDraft(null);
    setStored(null);
    setError(null);
    if (uri === null) {
      return;
    }
    setPhase("loading");
    let cancelled = false;
    getPreset(uri)
      .then((preset) => {
        if (cancelled) return;
        setStored(preset);
        setDraft(preset ? draftOf(preset) : { startMs: 0, skipMs: durationMs });
        setPhase("ready");
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        // Still editable: the draft starts from defaults, saving may succeed.
        setDraft({ startMs: 0, skipMs: durationMs });
        setPhase("error");
        setError(friendlyAuthMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [uri, durationMs]);

  // The track changed (or stopped) under an open editor: leave edit mode
  // silently — no seek, the remembered position belongs to the old track.
  useEffect(() => {
    if (session !== null && session.uri !== uri) {
      setSession(null);
      setLastPreviewAtMs(null);
      setEditMode(null).catch((err: unknown) => {
        console.warn("cued: could not release edit mode in the engine", err);
      });
    }
  }, [session, uri]);

  // Unmount (tab switch, disconnect) while editing: release the engine gate.
  const editingRef = useRef(false);
  editingRef.current = session !== null;
  useEffect(() => {
    return () => {
      if (editingRef.current) {
        setEditMode(null).catch((err: unknown) => {
          console.warn("cued: could not release edit mode in the engine", err);
        });
      }
    };
  }, []);

  const setStart = useCallback((ms: number) => {
    setDraft((d) => (d ? { ...d, startMs: clampStartMs(ms, d.skipMs) } : d));
    setPhase((p) => (p === "saved" || p === "removed" || p === "error" ? "ready" : p));
    setError(null);
  }, []);

  const setSkip = useCallback(
    (ms: number) => {
      setDraft((d) =>
        d ? { ...d, skipMs: clampSkipMs(ms, d.startMs, durationMs) } : d,
      );
      setPhase((p) => (p === "saved" || p === "removed" || p === "error" ? "ready" : p));
      setError(null);
    },
    [durationMs],
  );

  const enterEdit = useCallback(
    (positionMs: number | null) => {
      if (!track) return;
      // Edit mode only opens once the engine gate is armed — automation must
      // never fire for this track while the handles are out.
      setEditMode(track.uri)
        .then(() => {
          if (uriRef.current !== track.uri) {
            // The track changed during the round-trip: release again.
            return setEditMode(null);
          }
          setSession({ uri: track.uri, returnPositionMs: positionMs });
          setPhase((p) => (p === "loading" || p === "saving" ? p : "ready"));
          setError(null);
          return undefined;
        })
        .catch((err: unknown) => {
          setPhase("error");
          setError(friendlyAuthMessage(err));
        });
    },
    [track],
  );

  /** Leave edit mode: seek back (same track only), then release the gate. */
  const finishEdit = useCallback(() => {
    if (session === null) return;
    setSession(null);
    setLastPreviewAtMs(null);
    const target =
      session.returnPositionMs === null
        ? null
        : restoreTargetMs(session.uri, session.returnPositionMs, uriRef.current);
    void (async () => {
      try {
        if (target !== null) {
          await uiSeek(target);
        }
      } catch (err) {
        console.warn("cued: could not restore the playback position", err);
      }
      try {
        await setEditMode(null);
      } catch (err) {
        console.warn("cued: could not release edit mode in the engine", err);
      }
    })();
  }, [session]);

  const cancelEdit = useCallback(() => {
    setDraft(stored ? draftOf(stored) : track ? defaultDraft(track) : null);
    setPhase("ready");
    setError(null);
    finishEdit();
  }, [stored, track, finishEdit]);

  const saveAction: SaveAction =
    track && draft
      ? saveActionFor(draft.startMs, draft.skipMs, track.durationMs, stored !== null)
      : "nothing";

  const save = useCallback(() => {
    if (!track || !draft) {
      return;
    }
    const action = saveActionFor(
      draft.startMs,
      draft.skipMs,
      track.durationMs,
      stored !== null,
    );
    if (action === "nothing") {
      return;
    }
    if (
      action === "save" &&
      !isValidPreset(draft.startMs, draft.skipMs, track.durationMs)
    ) {
      return;
    }
    setPhase("saving");
    setError(null);
    const done =
      action === "delete"
        ? deletePreset(track.uri).then(() => {
            if (uriRef.current !== track.uri) return;
            setStored(null);
            setDraft(defaultDraft(track));
            setPhase("removed");
            finishEdit();
          })
        : savePreset({
            trackUri: track.uri,
            title: track.title,
            artists: track.artists,
            coverUrl: track.coverUrl,
            durationMs: track.durationMs,
            startMs: draft.startMs,
            skipMs: draft.skipMs,
          }).then((saved) => {
            if (uriRef.current !== saved.trackUri) return;
            setStored(saved);
            setDraft(draftOf(saved));
            setPhase("saved");
            finishEdit();
          });
    done.catch((err: unknown) => {
      if (uriRef.current !== track.uri) return;
      // Stay in edit mode so the user can adjust, retry or cancel.
      setPhase("error");
      setError(friendlyAuthMessage(err));
    });
  }, [track, draft, stored, finishEdit]);

  const preview = useCallback(
    (pointMs: number) => {
      const now = Date.now();
      if (session === null || !canPreviewAgain(lastPreviewAtMs, now)) {
        return;
      }
      setLastPreviewAtMs(now);
      uiSeek(previewTargetMs(pointMs)).catch((err: unknown) => {
        setPhase("error");
        setError(friendlyAuthMessage(err));
      });
    },
    [session, lastPreviewAtMs],
  );

  // A suggestion was accepted for this track (M10): a preset now exists.
  // Adopt it so the timeline shows it and Adjust prefills from it.
  const adoptSaved = useCallback(
    (preset: Preset) => {
      if (uriRef.current !== preset.trackUri) return;
      setStored(preset);
      setDraft(draftOf(preset));
      setPhase((p) => (p === "loading" || p === "saving" ? p : "ready"));
    },
    [],
  );

  const baseline = stored ? draftOf(stored) : track ? defaultDraft(track) : null;
  const dirty =
    draft !== null &&
    baseline !== null &&
    (draft.startMs !== baseline.startMs || draft.skipMs !== baseline.skipMs);

  return {
    draft,
    stored,
    dirty,
    editing: session !== null,
    saveAction,
    lastPreviewAtMs,
    phase,
    error,
    setStart,
    setSkip,
    enterEdit,
    cancelEdit,
    save,
    preview,
    adoptSaved,
  };
}
