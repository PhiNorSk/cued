import { useCallback, useEffect, useState } from "react";
import { AuthError } from "../lib/auth";
import {
  deleteAllInsights,
  getInsightsCount,
  getInsightsEnabled,
  setInsightsEnabled,
} from "../lib/insights";

export interface UseInsightsResult {
  /** Whether collection is on (optimistic while a save is in flight). */
  enabled: boolean;
  /** Collected-event count; null until first loaded. */
  count: number | null;
  /** False until the persisted toggle + count have loaded once. */
  ready: boolean;
  /** Display-ready message when a toggle save or delete failed. */
  error: string | null;
  setEnabled: (enabled: boolean) => void;
  /** Re-read the count (call whenever the surface (re)opens). */
  refreshCount: () => void;
  /** Erase all collected data; resets the count to 0 on success. */
  deleteAll: () => void;
}

const messageOf = (err: unknown, fallback: string): string =>
  err instanceof AuthError ? err.message : fallback;

/**
 * Settings state for listening insights (M9): the persisted on/off toggle
 * (saved optimistically, rolled back on failure), the live collected count,
 * and the destructive delete-all. Load happens on mount, so mounting the
 * settings surface fresh each open gives an up-to-date count.
 */
export function useInsights(): UseInsightsResult {
  const [enabled, setEnabledState] = useState(true);
  const [count, setCount] = useState<number | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshCount = useCallback(() => {
    getInsightsCount()
      .then(setCount)
      .catch((err: unknown) => {
        console.warn("cued: could not read the insights count", err);
      });
  }, []);

  useEffect(() => {
    let cancelled = false;
    Promise.allSettled([getInsightsEnabled(), getInsightsCount()])
      .then(([on, n]) => {
        if (cancelled) return;
        if (on.status === "fulfilled") setEnabledState(on.value);
        if (n.status === "fulfilled") setCount(n.value);
        setReady(true);
      })
      .catch(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setEnabled = useCallback((next: boolean) => {
    // Optimistic: the switch feels instant; roll back if saving fails.
    setEnabledState(next);
    setError(null);
    setInsightsEnabled(next).catch((err: unknown) => {
      setEnabledState(!next);
      setError(messageOf(err, "Could not save the setting."));
    });
  }, []);

  const deleteAll = useCallback(() => {
    setError(null);
    deleteAllInsights()
      .then(() => setCount(0))
      .catch((err: unknown) => {
        setError(messageOf(err, "Could not delete the data."));
      });
  }, []);

  return { enabled, count, ready, error, setEnabled, refreshCount, deleteAll };
}
