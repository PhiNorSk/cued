import { describe, expect, it } from "vitest";
import {
  EMPTY_STATE_DEBOUNCE_MS,
  formatTimeMs,
  interpolatePositionMs,
  playbackStateSchema,
  resolveNowPlayingView,
  type PlaybackState,
} from "./playback";

const TRACK = {
  uri: "spotify:track:abc123",
  title: "Song",
  artists: ["Artist A", "Artist B"],
  coverUrl: null,
  durationMs: 200_000,
  isLocal: false,
  kind: "track" as const,
};

function state(over: Partial<PlaybackState> = {}): PlaybackState {
  return {
    status: "playing",
    reconnecting: false,
    track: TRACK,
    positionMs: 10_000,
    fetchedAtMs: 1_000_000,
    automationSuspended: null,
    ...over,
  };
}

describe("interpolatePositionMs", () => {
  it("advances by wall-clock time while playing", () => {
    expect(interpolatePositionMs(state(), 1_002_500)).toBe(12_500);
  });

  it("clamps at the track duration", () => {
    expect(interpolatePositionMs(state({ positionMs: 199_500 }), 1_060_000)).toBe(
      200_000,
    );
  });

  it("freezes while paused", () => {
    expect(interpolatePositionMs(state({ status: "paused" }), 1_005_000)).toBe(
      10_000,
    );
  });

  it("freezes while reconnecting", () => {
    expect(interpolatePositionMs(state({ reconnecting: true }), 1_005_000)).toBe(
      10_000,
    );
  });

  it("never moves backwards on clock skew", () => {
    expect(interpolatePositionMs(state(), 999_000)).toBe(10_000);
  });

  it("passes through a missing position", () => {
    expect(interpolatePositionMs(state({ positionMs: null }), 1_001_000)).toBeNull();
  });
});

describe("resolveNowPlayingView", () => {
  const idle = () => state({ status: "idle" as const, track: null, positionMs: null });

  it("is pending before the first event", () => {
    expect(resolveNowPlayingView(null, null, null, 0).kind).toBe("pending");
  });

  it("shows the live track when one is present", () => {
    expect(resolveNowPlayingView(state(), state(), null, 1_000_000)).toEqual({
      kind: "track",
      state: state(),
      frozen: false,
    });
  });

  it("keeps the last track frozen during a brief idle gap", () => {
    const v = resolveNowPlayingView(
      idle(),
      state(),
      1_000_000,
      1_000_000 + EMPTY_STATE_DEBOUNCE_MS - 1,
    );
    expect(v).toEqual({ kind: "track", state: state(), frozen: true });
  });

  it("shows the empty state once the gap outlasts the debounce", () => {
    const v = resolveNowPlayingView(
      idle(),
      state(),
      1_000_000,
      1_000_000 + EMPTY_STATE_DEBOUNCE_MS,
    );
    expect(v.kind).toBe("empty");
  });

  it("shows the empty state immediately when no track was ever seen", () => {
    expect(resolveNowPlayingView(idle(), null, 1_000_000, 1_000_100).kind).toBe(
      "empty",
    );
  });

  it("never shows a track after auth is lost", () => {
    const lost = state({ status: "authLost" as const, track: null, positionMs: null });
    expect(resolveNowPlayingView(lost, state(), 1_000_000, 1_000_100).kind).toBe(
      "empty",
    );
  });
});

describe("formatTimeMs", () => {
  it("formats zero", () => expect(formatTimeMs(0)).toBe("0:00"));
  it("floors sub-second remainders", () => expect(formatTimeMs(59_400)).toBe("0:59"));
  it("rolls over minutes", () => expect(formatTimeMs(61_000)).toBe("1:01"));
  it("pads seconds", () => expect(formatTimeMs(725_000)).toBe("12:05"));
  it("clamps negatives to zero", () => expect(formatTimeMs(-5)).toBe("0:00"));
});

describe("playbackStateSchema", () => {
  it("parses a full playing snapshot", () => {
    expect(playbackStateSchema.safeParse(state()).success).toBe(true);
  });

  it("parses an authLost snapshot without a track", () => {
    const lost = {
      status: "authLost",
      reconnecting: false,
      track: null,
      positionMs: null,
      fetchedAtMs: 123,
      automationSuspended: null,
    };
    expect(playbackStateSchema.safeParse(lost).success).toBe(true);
  });

  it("parses an automation suspension reason", () => {
    const suspended = state({ automationSuspended: "restrictedDevice" });
    expect(playbackStateSchema.safeParse(suspended).success).toBe(true);
  });

  it("rejects an unknown suspension reason", () => {
    const bad = state({ automationSuspended: "solarFlare" as never });
    expect(playbackStateSchema.safeParse(bad).success).toBe(false);
  });

  it("rejects an unknown status", () => {
    expect(
      playbackStateSchema.safeParse(state({ status: "buffering" as never })).success,
    ).toBe(false);
  });

  it("rejects a snapshot missing its sample timestamp", () => {
    const incomplete: Record<string, unknown> = { ...state() };
    delete incomplete.fetchedAtMs;
    expect(playbackStateSchema.safeParse(incomplete).success).toBe(false);
  });
});
