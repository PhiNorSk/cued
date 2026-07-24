import { z } from "zod";
import { call } from "./auth";

/**
 * Skip-density heatmap (M11): the IPC wrapper for a track's normalized curve
 * plus the PURE coloring/geometry that renders it. Bucketing + normalization +
 * the rejection-exclusion rule all live in Rust (`heatmap.rs`); this module
 * turns the curve into a soft CSS heat gradient (decoupled from the waveform,
 * so it stays accurate at any bar thickness) and generates the FAKE (constant,
 * symmetric) waveform shape. Kept pure and unit-tested so the visual never
 * drifts from the data.
 */

/** Clamp helper — values arrive normalized but defend the boundary anyway. */
function clamp01(v: number): number {
  return Math.min(1, Math.max(0, v));
}

/** Low-density warm end — mirrors the --amber token (#c9915b). */
const AMBER_RGB = [201, 145, 91] as const;
/** Peak end — mirrors the --heat token (#a83f30) in index.css. */
const HEAT_RGB = [168, 63, 48] as const;
/** Deckkraft of the heat wash at peak density (kept calm — a whisper). */
const HEAT_MAX_ALPHA = 0.7;

/**
 * One stop color of the heat overlay for a normalized density: fully
 * transparent at 0 (the grey waveform shows through untouched), warming from
 * amber toward dark red and fading IN as density rises. A smoothstep on the
 * alpha keeps faint density barely visible so only real hotspots darken.
 */
export function heatStopColor(density: number): string {
  const d = clamp01(density);
  const eased = d * d * (3 - 2 * d); // smoothstep — calm low end
  const mix = (a: number, b: number) => Math.round(a + (b - a) * d);
  const r = mix(AMBER_RGB[0], HEAT_RGB[0]);
  const g = mix(AMBER_RGB[1], HEAT_RGB[1]);
  const b = mix(AMBER_RGB[2], HEAT_RGB[2]);
  const a = (eased * HEAT_MAX_ALPHA).toFixed(3);
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/**
 * A CSS horizontal `linear-gradient` for the heat overlay: one soft stop per
 * bucket, so it uses the FULL curve resolution and fades smoothly (no hard
 * per-bar edges), independent of how thick the waveform bars are. Returns
 * "none" when there is no usable curve.
 */
export function heatGradient(buckets: number[] | null | undefined): string {
  if (!buckets || buckets.length < 2) {
    return "none";
  }
  const n = buckets.length;
  const stops = buckets.map((v, i) => {
    const pos = ((i / (n - 1)) * 100).toFixed(2);
    return `${heatStopColor(v)} ${pos}%`;
  });
  return `linear-gradient(90deg, ${stops.join(", ")})`;
}

/**
 * The FAKE waveform envelope: `count` bar heights in [~0.32, ~0.92]. It is a
 * REGULAR, left-right SYMMETRIC pattern — a steady fine oscillation under a
 * gentle centre-taller envelope, driven purely by the distance from the middle
 * so `heights[i] === heights[count-1-i]`. Deterministic and IDENTICAL for every
 * track by design: it reads as decorative on sight and encodes no audio, so it
 * can never masquerade as a real waveform (the honesty rule). No randomness.
 */
export function fakeWaveHeights(count: number): number[] {
  if (count <= 0) {
    return [];
  }
  const out: number[] = [];
  const mid = (count - 1) / 2;
  for (let i = 0; i < count; i++) {
    const d = Math.abs(i - mid); // symmetric distance from centre → palindrome
    const fine = 0.5 + 0.5 * Math.cos(d * 1.15); // steady, even bar rhythm
    const envelope = 0.75 + 0.25 * Math.cos((d / (mid || 1)) * Math.PI); // taller centre
    out.push(0.32 + 0.6 * clamp01(fine * envelope));
  }
  return out;
}

const heatmapSchema = z.object({
  buckets: z.array(z.number()),
  eventCount: z.number().nonnegative(),
});
export type Heatmap = z.infer<typeof heatmapSchema>;

/** A track's skip-density curve, or null when there is too little data. */
export function getTrackHeatmap(trackUri: string): Promise<Heatmap | null> {
  return call("get_track_heatmap", heatmapSchema.nullable(), { trackUri });
}
