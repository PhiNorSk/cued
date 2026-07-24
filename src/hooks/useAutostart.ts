import { useCallback, useEffect, useState } from "react";
import { AuthError } from "../lib/auth";
import { getAutostartEnabled, setAutostartEnabled } from "../lib/autostart";

export interface UseAutostartResult {
  /** Whether the login item is registered (optimistic while a change is in flight). */
  enabled: boolean;
  /** False until the actual OS state has been read once. */
  ready: boolean;
  /** Display-ready message when registering/removing the login item failed. */
  error: string | null;
  setEnabled: (enabled: boolean) => void;
}

const messageOf = (err: unknown, fallback: string): string =>
  err instanceof AuthError ? err.message : fallback;

/**
 * Settings state for "Start at login" (M14). The settings panel mounts
 * fresh on every open, so the mount-time load reads the ACTUAL login-item
 * state from the OS each time — removing the entry in System Settings shows
 * up on the next open. Until that load finishes the toggle shows the
 * default: off.
 */
export function useAutostart(): UseAutostartResult {
  const [enabled, setEnabledState] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getAutostartEnabled()
      .then((on) => {
        if (cancelled) return;
        setEnabledState(on);
        setReady(true);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        // Not surfaced: the toggle stays at the safe default (off) and any
        // real problem shows up inline when the user flips it.
        console.warn("cued: could not read the login-item state", err);
        setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setEnabled = useCallback((next: boolean) => {
    // Optimistic: the switch feels instant; roll back if the OS call fails.
    setEnabledState(next);
    setError(null);
    setAutostartEnabled(next).catch((err: unknown) => {
      setEnabledState(!next);
      setError(messageOf(err, "Could not update the login item."));
    });
  }, []);

  return { enabled, ready, error, setEnabled };
}
