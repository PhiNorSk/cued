import { useEffect, useState } from "react";
import { useInsights } from "../hooks/useInsights";
import { openSupportPage } from "../lib/appInfo";
import { cn } from "../lib/cn";
import { appCopy, settingsCopy } from "../lib/copy";
import { insightsCountLabel } from "../lib/insights";
import {
  getSuggestionToggles,
  setSuggestionToggles,
  type SuggestionToggles,
} from "../lib/suggestions";

interface SettingsPanelProps {
  /** App version for the about line (moved here from the footer). */
  version: string | null;
  onClose: () => void;
}

/**
 * Settings overlay behind the header gear (M9). Currently the listening-
 * insights controls (on/off, collected count, delete-all with inline
 * confirm) plus the app version line. Mounted only while open, so the count
 * is freshly loaded each time it appears.
 */
const TOGGLES_ON: SuggestionToggles = {
  skipPoints: true,
  startPoints: true,
  autoSkip: true,
};

export function SettingsPanel({ version, onClose }: SettingsPanelProps) {
  const insights = useInsights();
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [supportError, setSupportError] = useState<string | null>(null);
  const [toggles, setToggles] = useState<SuggestionToggles>(TOGGLES_ON);
  const [togglesReady, setTogglesReady] = useState(false);

  // Escape closes the panel from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  // Load the per-type suggestion toggles once the panel opens.
  useEffect(() => {
    let cancelled = false;
    getSuggestionToggles()
      .then((t) => {
        if (cancelled) return;
        setToggles(t);
        setTogglesReady(true);
      })
      .catch(() => {
        if (!cancelled) setTogglesReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Optimistic: flip instantly, roll back if the save fails.
  const updateToggle = (patch: Partial<SuggestionToggles>) => {
    const next = { ...toggles, ...patch };
    const prev = toggles;
    setToggles(next);
    setSuggestionToggles(next).catch(() => {
      setToggles(prev);
    });
  };

  const countLabel =
    insights.count === null ? "Counting…" : insightsCountLabel(insights.count);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={settingsCopy.title}
      className="fixed inset-0 z-50 flex items-center justify-center p-6"
    >
      <button
        type="button"
        aria-label={settingsCopy.close}
        tabIndex={-1}
        onClick={onClose}
        className="absolute inset-0 bg-ground/70"
      />
      <div className="anim-rise-in relative w-full max-w-md rounded-2xl border border-hairline bg-surface p-6 shadow-2xl">
        <div className="flex items-center justify-between">
          <h2 className="text-base font-semibold tracking-tight text-text">
            {settingsCopy.title}
          </h2>
          <button
            type="button"
            aria-label={settingsCopy.close}
            onClick={onClose}
            className="rounded-full p-1 text-text-mut transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
          >
            <svg
              viewBox="0 0 20 20"
              className="h-5 w-5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
              aria-hidden
            >
              <path d="M5 5l10 10M15 5L5 15" />
            </svg>
          </button>
        </div>

        <section className="mt-5">
          <div className="flex items-center justify-between gap-4">
            <span
              id="insights-label"
              className="text-sm font-medium text-text"
            >
              {settingsCopy.insightsHeading}
            </span>
            <button
              type="button"
              role="switch"
              aria-checked={insights.enabled}
              aria-labelledby="insights-label"
              disabled={!insights.ready}
              onClick={() => {
                insights.setEnabled(!insights.enabled);
              }}
              className={cn(
                "relative h-5 w-9 shrink-0 rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:opacity-50",
                insights.enabled ? "bg-accent" : "bg-surface-2",
              )}
            >
              <span
                aria-hidden
                className={cn(
                  "absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-text transition-transform",
                  insights.enabled && "translate-x-4",
                )}
              />
            </button>
          </div>
          <p className="mt-2 text-xs leading-relaxed text-text-mut">
            {settingsCopy.insightsBlurb}
          </p>

          <div className="mt-4 flex items-center justify-between gap-4">
            <span
              aria-live="polite"
              className="text-sm tabular-nums text-text-mut"
            >
              {countLabel}
            </span>
            {!confirmingDelete ? (
              <button
                type="button"
                onClick={() => {
                  setConfirmingDelete(true);
                }}
                className="rounded-full px-3 py-1 text-xs font-medium text-amber transition-colors hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
              >
                {settingsCopy.deleteAction}
              </button>
            ) : (
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setConfirmingDelete(false);
                  }}
                  className="rounded-full px-3 py-1 text-xs font-medium text-text-mut transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
                >
                  {settingsCopy.deleteCancel}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    insights.deleteAll();
                    setConfirmingDelete(false);
                  }}
                  className="rounded-full bg-amber px-3 py-1 text-xs font-semibold text-ground transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
                >
                  {settingsCopy.deleteConfirm}
                </button>
              </div>
            )}
          </div>
          {confirmingDelete && (
            <p role="alert" className="mt-2 text-xs text-amber">
              {settingsCopy.deleteConfirmPrompt}
            </p>
          )}
          {insights.error && (
            <p role="alert" className="mt-2 text-xs text-amber">
              {insights.error}
            </p>
          )}
        </section>

        <section className="mt-6 border-t border-hairline pt-5">
          <h3 className="text-sm font-medium text-text">
            {settingsCopy.suggestionsHeading}
          </h3>
          <div className="mt-3 flex flex-col gap-3">
            <ToggleRow
              label={settingsCopy.suggestSkipPoints}
              checked={toggles.skipPoints}
              disabled={!togglesReady}
              onChange={(v) => updateToggle({ skipPoints: v })}
            />
            <ToggleRow
              label={settingsCopy.suggestStartPoints}
              checked={toggles.startPoints}
              disabled={!togglesReady}
              onChange={(v) => updateToggle({ startPoints: v })}
            />
            <ToggleRow
              label={settingsCopy.suggestAutoSkip}
              checked={toggles.autoSkip}
              disabled={!togglesReady}
              onChange={(v) => updateToggle({ autoSkip: v })}
            />
          </div>
          <p className="mt-3 text-xs leading-relaxed text-text-mut">
            {settingsCopy.provenance}
          </p>
        </section>

        <div className="mt-6 flex items-center justify-center gap-2 border-t border-hairline pt-3 text-[11px] text-text-mut">
          <button
            type="button"
            onClick={() => {
              setSupportError(null);
              openSupportPage().catch(() => {
                setSupportError(settingsCopy.supportFailed);
              });
            }}
            className="rounded transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
          >
            {settingsCopy.supportLabel}
          </button>
          {version !== null && (
            <>
              <span aria-hidden>·</span>
              <span>{appCopy.aboutLine(version)}</span>
            </>
          )}
        </div>
        {supportError && (
          <p role="alert" className="mt-2 text-center text-xs text-amber">
            {supportError}
          </p>
        )}
      </div>
    </div>
  );
}

/** A labelled on/off switch row, matching the insights toggle styling. */
function ToggleRow({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-sm text-text">{label}</span>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:opacity-50",
          checked ? "bg-accent" : "bg-surface-2",
        )}
      >
        <span
          aria-hidden
          className={cn(
            "absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-text transition-transform",
            checked && "translate-x-4",
          )}
        />
      </button>
    </div>
  );
}
