import { describe, expect, it } from "vitest";
import { fakeWaveHeights, heatGradient, heatStopColor } from "./heatmap";

/** Parse the alpha out of an "rgba(r, g, b, a)" string. */
function alphaOf(c: string): number {
  return Number(c.match(/rgba\(\d+, \d+, \d+, ([\d.]+)\)/)![1]);
}

describe("heatStopColor", () => {
  it("is fully transparent at zero density (grey waveform shows through)", () => {
    expect(alphaOf(heatStopColor(0))).toBe(0);
  });

  it("is dark red and clearly opaque at full density", () => {
    expect(heatStopColor(1)).toBe("rgba(168, 63, 48, 0.700)");
  });

  it("clamps out-of-range densities", () => {
    expect(heatStopColor(-5)).toBe(heatStopColor(0));
    expect(heatStopColor(5)).toBe(heatStopColor(1));
  });

  it("fades in monotonically (alpha grows with density)", () => {
    let prev = -1;
    for (const d of [0, 0.25, 0.5, 0.75, 1]) {
      const a = alphaOf(heatStopColor(d));
      expect(a).toBeGreaterThanOrEqual(prev);
      prev = a;
    }
  });

  it("warms from amber toward dark red (more red, less green)", () => {
    const rgb = (c: string) =>
      c.match(/rgba\((\d+), (\d+), (\d+),/)!.slice(1, 4).map(Number);
    const [r0, g0] = rgb(heatStopColor(0));
    const [r1, g1] = rgb(heatStopColor(1));
    expect(r1).toBeLessThan(r0); // amber 201 → red 168
    expect(g1).toBeLessThan(g0); // amber 145 → red 63
  });
});

describe("heatGradient", () => {
  it("returns none when there is no usable curve", () => {
    expect(heatGradient(null)).toBe("none");
    expect(heatGradient(undefined)).toBe("none");
    expect(heatGradient([])).toBe("none");
    expect(heatGradient([0.5])).toBe("none");
  });

  it("is a horizontal gradient with one stop per bucket", () => {
    const d = heatGradient([0, 0.5, 1]);
    expect(d.startsWith("linear-gradient(90deg, ")).toBe(true);
    expect(d).toContain("rgba(");
    expect((d.match(/%/g) ?? []).length).toBe(3);
  });

  it("places stops from 0% to 100% in non-decreasing order", () => {
    const positions = (heatGradient([0.1, 0.2, 0.9, 0.3, 0.7]).match(
      /([\d.]+)%/g,
    ) ?? []).map((s) => Number(s.replace("%", "")));
    expect(positions[0]).toBe(0);
    expect(positions[positions.length - 1]).toBe(100);
    for (let i = 1; i < positions.length; i++) {
      expect(positions[i]).toBeGreaterThanOrEqual(positions[i - 1]);
    }
  });
});

describe("fakeWaveHeights", () => {
  it("is deterministic — identical for every track", () => {
    expect(fakeWaveHeights(64)).toEqual(fakeWaveHeights(64));
  });

  it("returns the requested count", () => {
    expect(fakeWaveHeights(64)).toHaveLength(64);
    expect(fakeWaveHeights(0)).toHaveLength(0);
  });

  it("keeps every height within the audible band [0.28, 1]", () => {
    for (const h of fakeWaveHeights(200)) {
      expect(h).toBeGreaterThanOrEqual(0.28);
      expect(h).toBeLessThanOrEqual(1);
    }
  });

  it("actually varies (it is a wave, not a flat line)", () => {
    const hs = fakeWaveHeights(64);
    expect(new Set(hs.map((h) => h.toFixed(3))).size).toBeGreaterThan(8);
  });

  it("is left-right symmetric (clearly decorative, not real audio)", () => {
    for (const count of [48, 49, 64]) {
      const hs = fakeWaveHeights(count);
      for (let i = 0; i < hs.length; i++) {
        expect(hs[i]).toBeCloseTo(hs[hs.length - 1 - i], 9);
      }
    }
  });
});
