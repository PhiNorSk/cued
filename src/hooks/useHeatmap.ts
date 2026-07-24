import { useEffect, useState } from "react";
import { getInsightsEnabled } from "../lib/insights";
import { getTrackHeatmap } from "../lib/heatmap";

/**
 * The skip-density curve for the currently playing track (M11), or null when
 * there is nothing to draw — too little data, insights switched off, or no
 * track. Fetched at most ONCE per track change (the effect is keyed on the
 * URI) and off the polling hot path; the Rust side does the bucketing. The
 * caller collapses the reserved band when this is null, so the space never
 * shows an empty gap.
 */
export function useHeatmap(trackUri: string | null): number[] | null {
  const [buckets, setBuckets] = useState<number[] | null>(null);

  useEffect(() => {
    setBuckets(null);
    if (trackUri === null) {
      return;
    }
    let cancelled = false;
    void (async () => {
      // Both must hold: insights on AND enough data. Best-effort — any failure
      // just means no whisper this time, never a broken timeline.
      const on = await getInsightsEnabled().catch((err: unknown) => {
        console.warn("cued: could not read the insights setting", err);
        return false;
      });
      if (cancelled || !on) {
        return;
      }
      const hm = await getTrackHeatmap(trackUri).catch((err: unknown) => {
        console.warn("cued: could not load the skip heatmap", err);
        return null;
      });
      if (!cancelled) {
        setBuckets(hm?.buckets ?? null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [trackUri]);

  return buckets;
}
