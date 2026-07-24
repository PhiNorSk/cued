import { useState } from "react";
import { useHeatmap } from "../hooks/useHeatmap";
import { usePlayback } from "../hooks/usePlayback";
import { usePreset, type UsePresetResult } from "../hooks/usePreset";
import { useSuggestions, type UseSuggestionsResult } from "../hooks/useSuggestions";
import {
  formatTimeMs,
  interpolatePositionMs,
  type PlaybackState,
  type TrackInfo,
} from "../lib/playback";
import { canPreviewAgain, MIN_GAP_MS } from "../lib/presetLogic";
import { PresetTimeline, type HandleKind } from "./PresetTimeline";
import { SuggestionCard } from "./SuggestionCard";

interface NowPlayingProps {
  /** Master-toggle state of the auto-skip engine (status line only). */
  automationOn: boolean;
  /** Preview seeks need Premium; without it the control is hidden. */
  isPremium: boolean;
  /** Fired when the engine reports that re-authentication is required. */
  onAuthLost: () => void;
}

/** Bar heights/timings chosen to look organic; static bars under reduced motion. */
const EQ_BARS = [
  { height: "60%", duration: "0.9s", delay: "0s" },
  { height: "100%", duration: "1.15s", delay: "0.2s" },
  { height: "75%", duration: "1.3s", delay: "0.1s" },
] as const;

/** Subtle animated EQ bars next to the "Now playing" eyebrow. */
function EqBars() {
  return (
    <span aria-hidden className="inline-flex h-2.5 items-end gap-[2px]">
      {EQ_BARS.map((bar, index) => (
        <span
          key={index}
          className="eq-bar w-[2px] rounded-full bg-accent-hi"
          style={{
            height: bar.height,
            animationDuration: bar.duration,
            animationDelay: bar.delay,
          }}
        />
      ))}
    </span>
  );
}

/** Presets only make sense for real tracks that are long enough. */
function isPresetable(track: TrackInfo | null): track is TrackInfo {
  return (
    track !== null &&
    track.kind === "track" &&
    !track.isLocal &&
    track.durationMs >= MIN_GAP_MS
  );
}

/**
 * Now Playing card: cover, title, artists — and for regular tracks the
 * two-state preset timeline (calm VIEW with markers; full EDIT mode with
 * handles, audible preview and save/cancel). Episodes and local files get
 * a quiet note instead.
 */
export function NowPlaying({
  automationOn,
  isPremium,
  onAuthLost,
}: NowPlayingProps) {
  const { view, nowMs } = usePlayback(onAuthLost);
  const track = view.kind === "track" ? view.state.track : null;
  const presetable = isPresetable(track) ? track : null;
  const preset = usePreset(presetable);
  const suggestions = useSuggestions(presetable?.uri ?? null);

  return (
    <div className="flex min-h-36 w-[32rem] items-center rounded-xl border border-hairline bg-surface p-5">
      {view.kind === "track" ? (
        <TrackCard
          state={view.state}
          positionMs={
            view.frozen
              ? view.state.positionMs
              : interpolatePositionMs(view.state, nowMs)
          }
          nowMs={nowMs}
          preset={preset}
          suggestions={suggestions}
          automationOn={automationOn}
          isPremium={isPremium}
        />
      ) : (
        <p className="w-full text-center text-sm text-text-mut">
          {view.kind === "empty"
            ? "Nothing playing — start a song in Spotify"
            : "…"}
        </p>
      )}
    </div>
  );
}

function TrackCard({
  state,
  positionMs,
  nowMs,
  preset,
  suggestions,
  automationOn,
  isPremium,
}: {
  state: PlaybackState;
  positionMs: number | null;
  nowMs: number;
  preset: UsePresetResult;
  suggestions: UseSuggestionsResult;
  automationOn: boolean;
  isPremium: boolean;
}) {
  const track = state.track;
  if (!track) {
    return null;
  }
  const subtitle =
    track.kind === "episode" ? "Podcast episode" : track.artists.join(", ");
  // The engine acts only while actually playing and not suspended.
  const automationActive =
    automationOn &&
    state.status === "playing" &&
    state.automationSuspended === null;

  return (
    <div className="w-full">
      <div className="flex items-center gap-5">
        {track.coverUrl ? (
          <img
            src={track.coverUrl}
            alt=""
            className="h-20 w-20 shrink-0 rounded-lg object-cover"
          />
        ) : (
          <div
            aria-hidden
            className="flex h-20 w-20 shrink-0 items-center justify-center rounded-lg bg-surface-2 text-2xl text-text-mut"
          >
            ♪
          </div>
        )}
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.2em] text-text-mut">
            {state.status !== "paused" && <EqBars />}
            {state.status === "paused" ? "Paused" : "Now playing"}
            {state.reconnecting && (
              <span className="ml-2 normal-case tracking-normal text-amber">
                reconnecting…
              </span>
            )}
          </p>
          <h2 className="mt-1 truncate text-lg font-semibold text-text">
            {track.title}
          </h2>
          <p className="truncate text-sm text-text-mut">
            {subtitle}
            {track.isLocal && <span className="text-amber"> · local file</span>}
          </p>
        </div>
      </div>

      {isPresetable(track) ? (
        <PresetEditor
          track={track}
          positionMs={positionMs}
          nowMs={nowMs}
          preset={preset}
          suggestions={suggestions}
          automationActive={automationActive}
          automationOn={automationOn}
          isPremium={isPremium}
        />
      ) : (
        <PlainProgress track={track} positionMs={positionMs} />
      )}
    </div>
  );
}

/** The two-state timeline area: calm VIEW, full EDIT with preview + save. */
function PresetEditor({
  track,
  positionMs,
  nowMs,
  preset,
  suggestions,
  automationActive,
  automationOn,
  isPremium,
}: {
  track: TrackInfo;
  positionMs: number | null;
  nowMs: number;
  preset: UsePresetResult;
  suggestions: UseSuggestionsResult;
  automationActive: boolean;
  automationOn: boolean;
  isPremium: boolean;
}) {
  const [focusedHandle, setFocusedHandle] = useState<HandleKind>("start");
  const heatmap = useHeatmap(track.uri);

  if (!preset.editing) {
    return (
      <div className="mt-2">
        <div className="mt-6">
          <PresetTimeline
            mode="view"
            durationMs={track.durationMs}
            positionMs={positionMs}
            heatmap={heatmap}
            startMs={preset.stored?.startMs ?? null}
            skipMs={preset.stored?.skipMs ?? null}
          />
        </div>
        <div className="mt-3 flex items-center gap-3">
          <button
            type="button"
            onClick={() => {
              setFocusedHandle("start");
              preset.enterEdit(positionMs);
            }}
            disabled={preset.phase === "loading"}
            className="rounded-full border border-hairline bg-surface-2 px-4 py-1.5 text-xs font-semibold text-text transition-colors hover:border-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
          >
            {preset.stored ? "Edit preset" : "Set preset"}
          </button>
          <PresetStateLine preset={preset} automationActive={automationActive} />
        </div>
        {suggestions.card !== null && (
          <SuggestionCard
            key={suggestions.card.trackUri + suggestions.card.type}
            card={suggestions.card}
            onAccept={suggestions.accept}
            onDismiss={suggestions.dismiss}
            onUndo={suggestions.undo}
            onAdjust={(saved) => {
              if (saved) preset.adoptSaved(saved);
              preset.enterEdit(positionMs);
            }}
          />
        )}
      </div>
    );
  }

  const draft = preset.draft;
  if (!draft) {
    return null; // edit mode only opens once the draft is loaded
  }
  const previewPointMs =
    focusedHandle === "start" ? draft.startMs : draft.skipMs;
  const previewReady = canPreviewAgain(preset.lastPreviewAtMs, nowMs);

  return (
    <div className="mt-2">
      <PresetTimeline
        mode="edit"
        durationMs={track.durationMs}
        positionMs={positionMs}
        heatmap={heatmap}
        startMs={draft.startMs}
        skipMs={draft.skipMs}
        onChangeStart={preset.setStart}
        onChangeSkip={preset.setSkip}
        onFocusHandle={setFocusedHandle}
      />
      <div className="mt-3 flex items-center gap-3">
        {isPremium ? (
          <button
            type="button"
            onClick={() => {
              preset.preview(previewPointMs);
            }}
            disabled={!previewReady}
            className="rounded-full border border-hairline px-3 py-1.5 text-xs font-medium text-text transition-colors hover:border-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
          >
            ▶ Listen from here
            <span className="ml-1.5 font-normal tabular-nums text-text-mut">
              {focusedHandle === "start" ? "start" : "skip"}{" "}
              {formatTimeMs(previewPointMs)}
            </span>
          </button>
        ) : (
          <p className="text-xs text-text-mut">
            Preview needs Spotify Premium.
          </p>
        )}
        <div className="ml-auto flex items-center gap-3">
          <button
            type="button"
            onClick={preset.cancelEdit}
            disabled={preset.phase === "saving"}
            className="rounded-full px-3 py-1.5 text-xs font-medium text-text-mut transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={preset.save}
            disabled={
              preset.phase === "saving" ||
              !preset.dirty ||
              preset.saveAction === "nothing"
            }
            className="rounded-full bg-accent px-4 py-1.5 text-xs font-semibold text-ground transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
          >
            {preset.phase === "saving" ? "Saving…" : "Save preset"}
          </button>
        </div>
      </div>
      <EditHintLine preset={preset} automationOn={automationOn} />
    </div>
  );
}

/** Quiet line under the edit controls: error > nothing-to-save > paused note. */
function EditHintLine({
  preset,
  automationOn,
}: {
  preset: UsePresetResult;
  automationOn: boolean;
}) {
  if (preset.phase === "error" && preset.error) {
    return (
      <p role="alert" className="mt-2 truncate text-xs text-amber">
        {preset.error}
      </p>
    );
  }
  if (preset.saveAction === "nothing") {
    return (
      <p className="mt-2 text-xs text-text-mut">
        Nothing to save — move a handle first.
      </p>
    );
  }
  if (automationOn) {
    return (
      <p className="mt-2 text-xs text-text-mut">
        Automation is paused for this song while you edit.
      </p>
    );
  }
  return null;
}

/** State line next to the VIEW-state button. */
function PresetStateLine({
  preset,
  automationActive,
}: {
  preset: UsePresetResult;
  automationActive: boolean;
}) {
  if (preset.phase === "error" && preset.error) {
    return (
      <p role="alert" className="min-w-0 truncate text-xs text-amber">
        {preset.error}
      </p>
    );
  }
  if (preset.phase === "removed") {
    return (
      <p className="anim-rise-in min-w-0 truncate text-xs text-accent-hi">
        Preset removed — song plays normally
      </p>
    );
  }
  if (automationActive && preset.stored) {
    return (
      <p className="min-w-0 truncate text-xs text-accent-hi">
        Automation active — starts at {formatTimeMs(preset.stored.startMs)},
        skips at {formatTimeMs(preset.stored.skipMs)}
      </p>
    );
  }
  if (preset.phase === "saved" && preset.stored) {
    return (
      <p className="anim-rise-in min-w-0 truncate text-xs text-accent-hi">
        Preset saved — starts at {formatTimeMs(preset.stored.startMs)}, skips
        at {formatTimeMs(preset.stored.skipMs)}
      </p>
    );
  }
  return null;
}

/** The read-only M2 progress bar, for items presets don't apply to. */
function PlainProgress({
  track,
  positionMs,
}: {
  track: TrackInfo;
  positionMs: number | null;
}) {
  const progress =
    positionMs !== null && track.durationMs > 0
      ? Math.min(100, (positionMs / track.durationMs) * 100)
      : 0;
  return (
    <div className="mt-4">
      <div className="h-1 overflow-hidden rounded-full bg-surface-2">
        <div
          className="h-full rounded-full bg-accent"
          style={{ width: `${progress}%` }}
        />
      </div>
      <TimeLabels positionMs={positionMs} durationMs={track.durationMs} />
      <p className="mt-2 text-xs text-text-mut">
        {track.durationMs < MIN_GAP_MS && track.kind === "track" && !track.isLocal
          ? "This track is too short for a preset."
          : "Presets aren't available for this type."}
      </p>
    </div>
  );
}

function TimeLabels({
  positionMs,
  durationMs,
}: {
  positionMs: number | null;
  durationMs: number;
}) {
  return (
    <div className="mt-1 flex justify-between text-[11px] tabular-nums text-text-mut">
      <span>{positionMs !== null ? formatTimeMs(positionMs) : "–:––"}</span>
      <span>{formatTimeMs(durationMs)}</span>
    </div>
  );
}
