import { useState } from "react";
import { suggestionsCopy } from "../lib/copy";
import { friendlyAuthMessage } from "../lib/errorCopy";
import { formatTimeMs } from "../lib/playback";
import type { Preset, PresetInput } from "../lib/presets";
import type { AcceptResult, Suggestion, SuggestionType } from "../lib/suggestions";

interface SuggestionCardProps {
  card: Suggestion;
  onAccept: (type: SuggestionType) => Promise<AcceptResult | null>;
  onDismiss: (type: SuggestionType) => Promise<void> | void;
  onUndo: (
    type: SuggestionType,
    previous: PresetInput | null,
  ) => Promise<void> | void;
  /** Open M8 edit mode prefilled with the just-applied preset. */
  onAdjust: (saved: Preset | null) => void;
}

function toInput(preset: Preset): PresetInput {
  return {
    trackUri: preset.trackUri,
    title: preset.title,
    artists: preset.artists,
    coverUrl: preset.coverUrl,
    durationMs: preset.durationMs,
    startMs: preset.startMs,
    skipMs: preset.skipMs,
  };
}

/** The ✦ glyph: a desaturated accent mark at ~65% opacity, no motion. */
function Glyph() {
  return (
    <span aria-hidden className="text-sm leading-none text-accent/65">
      ✦
    </span>
  );
}

/**
 * The single proactive suggestion card in Now Playing (M10). Calm by design:
 * a hairline-bordered card that fades in once (200 ms, no pulse), states the
 * observed fact, and offers a quiet choice. Accepting a skip/start suggestion
 * applies it instantly and morphs to a confirmation with Adjust + Undo;
 * auto-skip is the higher-stakes two-button choice. Remounted per suggestion
 * by its key, so its local phase always starts fresh.
 */
export function SuggestionCard({
  card,
  onAccept,
  onDismiss,
  onUndo,
  onAdjust,
}: SuggestionCardProps) {
  const [phase, setPhase] = useState<"offer" | "applied">("offer");
  const [saved, setSaved] = useState<Preset | null>(null);
  const [previous, setPrevious] = useState<Preset | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const region =
    card.valueStartMs !== null && card.valueEndMs !== null
      ? `${formatTimeMs(card.valueStartMs)}–${formatTimeMs(card.valueEndMs)}`
      : card.valueStartMs !== null
        ? formatTimeMs(card.valueStartMs)
        : "";
  const target = card.valueStartMs !== null ? formatTimeMs(card.valueStartMs) : "";

  const runAccept = () => {
    setBusy(true);
    setError(null);
    onAccept(card.type)
      .then((result) => {
        if (result) {
          setSaved(result.saved);
          setPrevious(result.previous);
          setPhase("applied");
        }
      })
      .catch((err: unknown) => {
        setError(friendlyAuthMessage(err));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  const runUndo = () => {
    setBusy(true);
    setError(null);
    Promise.resolve(onUndo(card.type, previous ? toInput(previous) : null))
      .then(() => {
        setPhase("offer");
        setSaved(null);
        setPrevious(null);
      })
      .catch((err: unknown) => {
        setError(friendlyAuthMessage(err));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  const runDismiss = () => {
    setBusy(true);
    setError(null);
    Promise.resolve(onDismiss(card.type)).catch((err: unknown) => {
      setError(friendlyAuthMessage(err));
      setBusy(false);
    });
  };

  return (
    <div className="anim-fade-in mt-4 rounded-xl border border-hairline bg-surface-2/40 px-4 py-3">
      {phase === "offer" ? (
        <div className="flex flex-col gap-2.5">
          <p className="flex items-start gap-2 text-sm leading-relaxed text-text">
            <span className="mt-0.5">
              <Glyph />
            </span>
            <span>{factText(card, region, target)}</span>
          </p>
          <div className="flex items-center gap-2">
            <PrimaryButton
              label={acceptLabel(card.type)}
              onClick={runAccept}
              disabled={busy}
            />
            <QuietButton
              label={dismissLabel(card.type)}
              onClick={runDismiss}
              disabled={busy}
            />
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-2.5">
          <p className="flex items-center gap-2 text-sm text-accent-hi">
            <Glyph />
            <span>{appliedText(card, region, target)}</span>
          </p>
          <div className="flex items-center gap-2">
            {card.type !== "auto_skip" && (
              <QuietButton
                label={suggestionsCopy.adjust}
                onClick={() => {
                  onAdjust(saved);
                }}
                disabled={busy}
              />
            )}
            <QuietButton
              label={suggestionsCopy.undo}
              onClick={runUndo}
              disabled={busy}
            />
          </div>
        </div>
      )}
      {error !== null && (
        <p role="alert" className="mt-2 text-xs text-amber">
          {error}
        </p>
      )}
    </div>
  );
}

function factText(card: Suggestion, region: string, target: string): string {
  switch (card.type) {
    case "skip_point":
      return suggestionsCopy.skipPointFact(region, card.playsMatching, card.playsTotal);
    case "start_point":
      return suggestionsCopy.startPointFact(target, card.playsMatching, card.playsTotal);
    case "auto_skip":
      return suggestionsCopy.autoSkipFact(card.playsMatching, card.playsTotal);
  }
}

function appliedText(card: Suggestion, region: string, target: string): string {
  switch (card.type) {
    case "skip_point":
      return suggestionsCopy.skipPointSet(region);
    case "start_point":
      return suggestionsCopy.startPointSet(target);
    case "auto_skip":
      return suggestionsCopy.autoSkipSet;
  }
}

function acceptLabel(type: SuggestionType): string {
  switch (type) {
    case "skip_point":
      return suggestionsCopy.setSkipPoint;
    case "start_point":
      return suggestionsCopy.setStartPoint;
    case "auto_skip":
      return suggestionsCopy.autoSkipIt;
  }
}

function dismissLabel(type: SuggestionType): string {
  return type === "auto_skip"
    ? suggestionsCopy.keepPlaying
    : suggestionsCopy.noThanks;
}

function PrimaryButton({
  label,
  onClick,
  disabled,
}: {
  label: string;
  onClick: () => void;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="rounded-full bg-accent px-4 py-1.5 text-xs font-semibold text-ground transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
    >
      {label}
    </button>
  );
}

function QuietButton({
  label,
  onClick,
  disabled,
}: {
  label: string;
  onClick: () => void;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="rounded-full px-3 py-1.5 text-xs font-medium text-text-mut transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
    >
      {label}
    </button>
  );
}
