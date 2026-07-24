import { describe, expect, it } from "vitest";
import { insightsCountLabel } from "./insights";

describe("insightsCountLabel", () => {
  it("gives an inviting empty state at zero", () => {
    expect(insightsCountLabel(0)).toBe("No events collected yet");
  });

  it("treats a negative count as empty (defensive)", () => {
    expect(insightsCountLabel(-1)).toBe("No events collected yet");
  });

  it("uses the singular for exactly one event", () => {
    expect(insightsCountLabel(1)).toBe("1 event collected");
  });

  it("pluralizes and groups larger counts", () => {
    expect(insightsCountLabel(2)).toBe("2 events collected");
    expect(insightsCountLabel(1234)).toBe(`${(1234).toLocaleString()} events collected`);
  });
});
