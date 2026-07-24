import { describe, expect, it } from "vitest";
import {
  isTypeEnabled,
  librarySuggestions,
  pickCardSuggestion,
  suggestionRatio,
  type Suggestion,
  type SuggestionStatus,
  type SuggestionToggles,
  type SuggestionType,
} from "./suggestions";

const ALL_ON: SuggestionToggles = {
  skipPoints: true,
  startPoints: true,
  autoSkip: true,
};

function sugg(
  overrides: Partial<Suggestion> & {
    type: SuggestionType;
    status: SuggestionStatus;
  },
): Suggestion {
  return {
    trackUri: "spotify:track:x",
    shownCount: 0,
    valueStartMs: 72_000,
    valueEndMs: 74_000,
    playsTotal: 10,
    playsMatching: 8,
    updatedAt: 0,
    title: "Song",
    artists: ["Artist"],
    coverUrl: null,
    durationMs: 200_000,
    ...overrides,
  };
}

describe("suggestionRatio", () => {
  it("is matching / total, and 0 when nothing was considered", () => {
    expect(suggestionRatio(sugg({ type: "skip_point", status: "active", playsTotal: 10, playsMatching: 8 }))).toBeCloseTo(0.8);
    expect(suggestionRatio(sugg({ type: "skip_point", status: "active", playsTotal: 0, playsMatching: 0 }))).toBe(0);
  });
});

describe("isTypeEnabled", () => {
  it("maps each type to its toggle", () => {
    const off: SuggestionToggles = { skipPoints: false, startPoints: true, autoSkip: false };
    expect(isTypeEnabled("skip_point", off)).toBe(false);
    expect(isTypeEnabled("start_point", off)).toBe(true);
    expect(isTypeEnabled("auto_skip", off)).toBe(false);
  });
});

describe("pickCardSuggestion", () => {
  it("returns null when insights are off, whatever exists", () => {
    const active = [sugg({ type: "skip_point", status: "active" })];
    expect(pickCardSuggestion(active, ALL_ON, false)).toBeNull();
  });

  it("only considers ACTIVE suggestions (never applied/retired/dismissed)", () => {
    const list = [
      sugg({ type: "skip_point", status: "applied" }),
      sugg({ type: "start_point", status: "retired" }),
      sugg({ type: "auto_skip", status: "dismissed" }),
    ];
    expect(pickCardSuggestion(list, ALL_ON, true)).toBeNull();
  });

  it("picks the single strongest by evidence ratio", () => {
    const weak = sugg({ type: "skip_point", status: "active", playsTotal: 10, playsMatching: 7 });
    const strong = sugg({ type: "start_point", status: "active", playsTotal: 10, playsMatching: 9 });
    expect(pickCardSuggestion([weak, strong], ALL_ON, true)?.type).toBe("start_point");
  });

  it("breaks ratio ties by plays, then by type priority", () => {
    const fewer = sugg({ type: "skip_point", status: "active", playsTotal: 5, playsMatching: 4 });
    const more = sugg({ type: "start_point", status: "active", playsTotal: 10, playsMatching: 8 });
    // Same 0.8 ratio → more plays wins.
    expect(pickCardSuggestion([fewer, more], ALL_ON, true)?.type).toBe("start_point");

    const skip = sugg({ type: "skip_point", status: "active", playsTotal: 10, playsMatching: 8 });
    const auto = sugg({ type: "auto_skip", status: "active", playsTotal: 10, playsMatching: 8 });
    // Same ratio and plays → auto_skip has the higher priority.
    expect(pickCardSuggestion([skip, auto], ALL_ON, true)?.type).toBe("auto_skip");
  });

  it("skips a disabled type even when it is the strongest", () => {
    const off: SuggestionToggles = { skipPoints: false, startPoints: true, autoSkip: true };
    const strongDisabled = sugg({ type: "skip_point", status: "active", playsTotal: 10, playsMatching: 10 });
    const weakerEnabled = sugg({ type: "start_point", status: "active", playsTotal: 10, playsMatching: 8 });
    expect(pickCardSuggestion([strongDisabled, weakerEnabled], off, true)?.type).toBe("start_point");
  });
});

describe("librarySuggestions", () => {
  it("always lists an applied auto-skip so it stays reversible", () => {
    const list = [sugg({ type: "auto_skip", status: "applied" })];
    // Even with insights off AND the auto-skip type toggled off.
    const off: SuggestionToggles = { skipPoints: true, startPoints: true, autoSkip: false };
    expect(librarySuggestions(list, off, false)).toHaveLength(1);
  });

  it("lists active and retired suggestions of enabled types", () => {
    const list = [
      sugg({ type: "skip_point", status: "active" }),
      sugg({ type: "start_point", status: "retired" }),
    ];
    expect(librarySuggestions(list, ALL_ON, true)).toHaveLength(2);
  });

  it("hides active/retired suggestions when insights are off", () => {
    const list = [sugg({ type: "skip_point", status: "active" })];
    expect(librarySuggestions(list, ALL_ON, false)).toHaveLength(0);
  });

  it("omits applied skip/start suggestions (they are presets now)", () => {
    const list = [
      sugg({ type: "skip_point", status: "applied" }),
      sugg({ type: "start_point", status: "applied" }),
    ];
    expect(librarySuggestions(list, ALL_ON, true)).toHaveLength(0);
  });
});
