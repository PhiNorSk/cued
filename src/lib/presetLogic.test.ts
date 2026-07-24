import { describe, expect, it } from "vitest";
import { formatTimeMs } from "./playback";
import {
  canPreviewAgain,
  clampSkipMs,
  clampStartMs,
  FLAG_REPEL_BELOW_PCT,
  isNeutralPreset,
  isValidPreset,
  MIN_GAP_MS,
  parseTimeMs,
  presetMatchesQuery,
  PREVIEW_COOLDOWN_MS,
  PREVIEW_PREROLL_MS,
  previewTargetMs,
  restoreTargetMs,
  saveActionFor,
  flagsRepel,
  snapMsToSecond,
  timelineMsFromPointer,
} from "./presetLogic";

const DURATION = 200_000;

describe("isValidPreset", () => {
  it("accepts a typical preset", () => {
    expect(isValidPreset(5_000, 180_000, DURATION)).toBe(true);
  });

  it("accepts the full-track preset (0 to duration)", () => {
    expect(isValidPreset(0, DURATION, DURATION)).toBe(true);
  });

  it("accepts a gap of exactly 10 s", () => {
    expect(isValidPreset(0, MIN_GAP_MS, DURATION)).toBe(true);
  });

  it("rejects a gap 1 ms below the minimum", () => {
    expect(isValidPreset(0, MIN_GAP_MS - 1, DURATION)).toBe(false);
  });

  it("rejects start == skip", () => {
    expect(isValidPreset(50_000, 50_000, DURATION)).toBe(false);
  });

  it("rejects start > skip", () => {
    expect(isValidPreset(60_000, 50_000, DURATION)).toBe(false);
  });

  it("rejects a negative start", () => {
    expect(isValidPreset(-1, 50_000, DURATION)).toBe(false);
  });

  it("rejects skip beyond the duration", () => {
    expect(isValidPreset(0, DURATION + 1, DURATION)).toBe(false);
  });

  it("rejects non-integer values", () => {
    expect(isValidPreset(0.5, 50_000, DURATION)).toBe(false);
    expect(isValidPreset(0, 50_000.5, DURATION)).toBe(false);
  });

  it("rejects everything on a track shorter than the minimum gap", () => {
    expect(isValidPreset(0, 9_000, 9_000)).toBe(false);
  });
});

describe("clampStartMs", () => {
  it("keeps an in-range value unchanged", () => {
    expect(clampStartMs(5_000, 180_000)).toBe(5_000);
  });

  it("stops at skip - 10 s when dragged into the gap", () => {
    expect(clampStartMs(175_000, 180_000)).toBe(180_000 - MIN_GAP_MS);
  });

  it("stops at 0 when dragged below the track start", () => {
    expect(clampStartMs(-4_000, 180_000)).toBe(0);
  });

  it("collapses to 0 when skip leaves no room at all", () => {
    expect(clampStartMs(3_000, MIN_GAP_MS)).toBe(0);
  });
});

describe("clampSkipMs", () => {
  it("keeps an in-range value unchanged", () => {
    expect(clampSkipMs(180_000, 5_000, DURATION)).toBe(180_000);
  });

  it("stops at start + 10 s when dragged into the gap", () => {
    expect(clampSkipMs(7_000, 5_000, DURATION)).toBe(5_000 + MIN_GAP_MS);
  });

  it("stops at the duration when dragged past the end", () => {
    expect(clampSkipMs(DURATION + 9_000, 5_000, DURATION)).toBe(DURATION);
  });

  it("collapses to the duration when start leaves no room", () => {
    expect(clampSkipMs(DURATION - 2_000, DURATION - 5_000, DURATION)).toBe(
      DURATION,
    );
  });
});

describe("snapMsToSecond", () => {
  it("rounds to the nearest whole second", () => {
    expect(snapMsToSecond(1_499)).toBe(1_000);
    expect(snapMsToSecond(1_500)).toBe(2_000);
  });

  it("keeps whole seconds untouched", () => {
    expect(snapMsToSecond(3_000)).toBe(3_000);
  });
});

describe("timelineMsFromPointer", () => {
  it("maps the left edge to 0 and the right edge to the duration", () => {
    expect(timelineMsFromPointer(100, 100, 400, DURATION)).toBe(0);
    expect(timelineMsFromPointer(500, 100, 400, DURATION)).toBe(DURATION);
  });

  it("maps the middle proportionally", () => {
    expect(timelineMsFromPointer(300, 100, 400, DURATION)).toBe(DURATION / 2);
  });

  it("clamps pointers outside the bar", () => {
    expect(timelineMsFromPointer(0, 100, 400, DURATION)).toBe(0);
    expect(timelineMsFromPointer(900, 100, 400, DURATION)).toBe(DURATION);
  });

  it("returns 0 for a degenerate zero-width bar", () => {
    expect(timelineMsFromPointer(300, 100, 0, DURATION)).toBe(0);
  });
});

describe("parseTimeMs", () => {
  it("parses m:ss", () => {
    expect(parseTimeMs("0:00")).toBe(0);
    expect(parseTimeMs("3:05")).toBe(185_000);
    expect(parseTimeMs("12:34")).toBe(754_000);
  });

  it("parses long tracks (3+ digit minutes)", () => {
    expect(parseTimeMs("123:45")).toBe(123 * 60_000 + 45_000);
  });

  it("tolerates surrounding whitespace", () => {
    expect(parseTimeMs(" 1:30 ")).toBe(90_000);
  });

  it("rejects seconds >= 60", () => {
    expect(parseTimeMs("1:60")).toBeNull();
  });

  it("rejects single-digit seconds", () => {
    expect(parseTimeMs("1:5")).toBeNull();
  });

  it("rejects garbage, negatives and empty input", () => {
    expect(parseTimeMs("")).toBeNull();
    expect(parseTimeMs("abc")).toBeNull();
    expect(parseTimeMs("-1:30")).toBeNull();
    expect(parseTimeMs(":30")).toBeNull();
    expect(parseTimeMs("1:30.5")).toBeNull();
    expect(parseTimeMs("90")).toBeNull();
    expect(parseTimeMs("1:2:03")).toBeNull();
  });

  it("round-trips with formatTimeMs for whole seconds", () => {
    for (const ms of [0, 1_000, 59_000, 60_000, 185_000, 3_599_000]) {
      expect(parseTimeMs(formatTimeMs(ms))).toBe(ms);
    }
  });
});

describe("isNeutralPreset", () => {
  it("detects the untouched default (0 to duration)", () => {
    expect(isNeutralPreset(0, DURATION, DURATION)).toBe(true);
  });

  it("treats skip one ms before the end as a real preset", () => {
    expect(isNeutralPreset(0, DURATION - 1, DURATION)).toBe(false);
  });

  it("treats a moved start as a real preset", () => {
    expect(isNeutralPreset(1_000, DURATION, DURATION)).toBe(false);
  });
});

describe("saveActionFor", () => {
  it("blocks saving a neutral draft when nothing is stored", () => {
    expect(saveActionFor(0, DURATION, DURATION, false)).toBe("nothing");
  });

  it("deletes when an existing preset is edited back to neutral", () => {
    expect(saveActionFor(0, DURATION, DURATION, true)).toBe("delete");
  });

  it("saves any non-neutral draft", () => {
    expect(saveActionFor(30_000, 180_000, DURATION, false)).toBe("save");
    expect(saveActionFor(30_000, 180_000, DURATION, true)).toBe("save");
    // Boundary: skip one ms before the end is savable, not neutral.
    expect(saveActionFor(0, DURATION - 1, DURATION, false)).toBe("save");
  });
});

describe("previewTargetMs", () => {
  it("starts the pre-roll before the point", () => {
    expect(previewTargetMs(30_000)).toBe(30_000 - PREVIEW_PREROLL_MS);
  });

  it("clamps at 0 for points inside the first pre-roll window", () => {
    expect(previewTargetMs(0)).toBe(0);
    expect(previewTargetMs(2_000)).toBe(0);
    expect(previewTargetMs(PREVIEW_PREROLL_MS)).toBe(0);
  });
});

describe("canPreviewAgain", () => {
  it("allows the first preview", () => {
    expect(canPreviewAgain(null, 1_000)).toBe(true);
  });

  it("blocks taps inside the cooldown", () => {
    expect(canPreviewAgain(10_000, 10_000 + PREVIEW_COOLDOWN_MS - 1)).toBe(
      false,
    );
  });

  it("allows the next preview once the cooldown elapsed", () => {
    expect(canPreviewAgain(10_000, 10_000 + PREVIEW_COOLDOWN_MS)).toBe(true);
  });
});

describe("restoreTargetMs", () => {
  it("restores when the same track still plays", () => {
    expect(restoreTargetMs("spotify:track:a", 42_000, "spotify:track:a")).toBe(
      42_000,
    );
  });

  it("does nothing when the track changed or nothing plays", () => {
    expect(
      restoreTargetMs("spotify:track:a", 42_000, "spotify:track:b"),
    ).toBeNull();
    expect(restoreTargetMs("spotify:track:a", 42_000, null)).toBeNull();
  });
});

describe("flagsRepel", () => {
  it("repels chips closer than the threshold", () => {
    expect(flagsRepel(50, 50 + FLAG_REPEL_BELOW_PCT - 1)).toBe(true);
  });

  it("keeps distant chips centered", () => {
    expect(flagsRepel(10, 80)).toBe(false);
  });
});

describe("presetMatchesQuery", () => {
  const artists = ["Daft Punk", "Pharrell Williams"];

  it("matches everything on an empty or whitespace query", () => {
    expect(presetMatchesQuery("Get Lucky", artists, "")).toBe(true);
    expect(presetMatchesQuery("Get Lucky", artists, "   ")).toBe(true);
  });

  it("matches title and artists case-insensitively", () => {
    expect(presetMatchesQuery("Get Lucky", artists, "get LUCKY")).toBe(true);
    expect(presetMatchesQuery("Get Lucky", artists, "pharrell")).toBe(true);
  });

  it("rejects non-matching queries", () => {
    expect(presetMatchesQuery("Get Lucky", artists, "beatles")).toBe(false);
  });
});
