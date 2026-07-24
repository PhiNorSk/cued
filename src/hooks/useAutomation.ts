import { useCallback, useEffect, useState } from "react";
import {
  getAutomationEnabled,
  onAutomationEnabled,
  setAutomationEnabled,
} from "../lib/automation";
import { AuthError } from "../lib/auth";
import type { AutomationSuspension } from "../lib/playback";
import { onPlaybackState } from "../lib/player";

export interface UseAutomationResult {
  /** Current master-toggle state (optimistic while a save is in flight). */
  enabled: boolean;
  /** False until the persisted value has been loaded. */
  ready: boolean;
  /** Why the engine cannot act right now (null = it can). */
  suspension: AutomationSuspension | null;
  /** Display-ready message when persisting the toggle failed. */
  error: string | null;
  setEnabled: (enabled: boolean) => void;
}

/**
 * Master-toggle state of the auto-skip engine: loads the persisted value,
 * saves changes optimistically (rolling back on failure), and mirrors the
 * engine-reported suspension reason from the playback events.
 */
export function useAutomation(): UseAutomationResult {
  const [enabled, setEnabledState] = useState(true);
  const [ready, setReady] = useState(false);
  const [suspension, setSuspension] = useState<AutomationSuspension | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getAutomationEnabled()
      .then((on) => {
        if (cancelled) return;
        setEnabledState(on);
        setReady(true);
      })
      .catch((err: unknown) => {
        console.warn("cued: could not load the automation toggle", err);
        // Mirror the backend default (on) and stay usable.
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    onPlaybackState((state) => {
      setSuspension(state.automationSuspended);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err: unknown) => {
        console.warn("cued: could not subscribe to automation status", err);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Toggle changes made outside this UI (the tray menu): the backend pushes
  // the applied value, so the in-app switch always mirrors the tray.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    onAutomationEnabled((on) => {
      setEnabledState(on);
      setError(null);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err: unknown) => {
        console.warn("cued: could not subscribe to automation changes", err);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const setEnabled = useCallback((next: boolean) => {
    // Optimistic: the switch must feel instant; roll back if saving fails.
    setEnabledState(next);
    setError(null);
    setAutomationEnabled(next).catch((err: unknown) => {
      setEnabledState(!next);
      setError(
        err instanceof AuthError ? err.message : "Could not save the setting.",
      );
    });
  }, []);

  return { enabled, ready, suspension, error, setEnabled };
}
