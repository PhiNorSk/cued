import { describe, expect, it } from "vitest";
import { AuthError } from "./auth";
import { AUTH_ERROR_COPY, friendlyAuthMessage, GENERIC_ERROR } from "./errorCopy";

/** Every code the Rust backend can emit (error.rs) plus the frontend's own. */
const ALL_KNOWN_CODES = [
  "port_in_use",
  "callback_timeout",
  "state_mismatch",
  "access_denied",
  "spotify_auth",
  "no_client_id",
  "invalid_client_id",
  "keychain",
  "network",
  "api",
  "malformed_response",
  "rate_limited",
  "config",
  "login_in_progress",
  "bad_callback",
  // frontend-only codes from src/lib/auth.ts
  "timeout",
  "bad_ipc_shape",
];

describe("AUTH_ERROR_COPY", () => {
  it("covers every known error code", () => {
    for (const code of ALL_KNOWN_CODES) {
      expect(AUTH_ERROR_COPY[code], `missing copy for "${code}"`).toBeTruthy();
    }
  });

  it("contains no raw codes or jargon in the copy", () => {
    for (const [code, text] of Object.entries(AUTH_ERROR_COPY)) {
      // snake_case would mean a raw code leaked into user-facing text
      // (single-word codes like "network" are legitimate English words)
      expect(text, `copy for "${code}" contains an underscore`).not.toContain(
        "_",
      );
      // "what to do" implies a full sentence, not a fragment
      expect(text.endsWith("."), `copy for "${code}" is not a sentence`).toBe(
        true,
      );
    }
  });
});

describe("friendlyAuthMessage", () => {
  it("maps a known AuthError code to its friendly copy", () => {
    const err = new AuthError("access_denied", "Access was denied in the Spotify login page.");
    expect(friendlyAuthMessage(err)).toBe(AUTH_ERROR_COPY.access_denied);
  });

  it("falls back to the backend's display-ready message for unmapped codes", () => {
    const err = new AuthError("invalid_times", "Start must come before skip.");
    expect(friendlyAuthMessage(err)).toBe("Start must come before skip.");
  });

  it("returns the generic message for anything that is not an AuthError", () => {
    expect(friendlyAuthMessage(new Error("boom"))).toBe(GENERIC_ERROR);
    expect(friendlyAuthMessage("boom")).toBe(GENERIC_ERROR);
    expect(friendlyAuthMessage(undefined)).toBe(GENERIC_ERROR);
  });
});
