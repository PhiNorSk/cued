import { cn } from "../lib/cn";
import type { AutomationSuspension } from "../lib/playback";

const SUSPENSION_NOTICE: Record<AutomationSuspension, string> = {
  noPremium: "Automation requires Spotify Premium",
  restrictedDevice: "This device can't be controlled",
  rateLimited: "Spotify rate limit — automation paused briefly",
};

interface AutomationToggleProps {
  enabled: boolean;
  /** False while the persisted state is still loading (switch disabled). */
  ready: boolean;
  /** Why the engine cannot act right now (null = it can). */
  suspension: AutomationSuspension | null;
  /** Display-ready message when persisting the toggle failed. */
  error: string | null;
  onChange: (enabled: boolean) => void;
}

/**
 * Master switch of the auto-skip engine (header pill switch, accent when
 * on) plus a quiet note explaining why the engine cannot act, when it can't.
 */
export function AutomationToggle({
  enabled,
  ready,
  suspension,
  error,
  onChange,
}: AutomationToggleProps) {
  const notice =
    error ?? (enabled && suspension ? SUSPENSION_NOTICE[suspension] : null);
  return (
    <div className="flex min-w-0 items-center gap-3">
      {notice && (
        <p className="min-w-0 truncate text-xs text-text-mut">{notice}</p>
      )}
      <span id="automation-label" className="text-xs font-medium text-text-mut">
        Automation
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-labelledby="automation-label"
        disabled={!ready}
        onClick={() => {
          onChange(!enabled);
        }}
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:opacity-50",
          enabled ? "bg-accent" : "bg-surface-2",
        )}
      >
        <span
          aria-hidden
          className={cn(
            "absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-text transition-transform",
            enabled && "translate-x-4",
          )}
        />
      </button>
    </div>
  );
}
