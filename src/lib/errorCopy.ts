import { AuthError } from "./auth";

/** Fallback when an error carries no display-ready information at all. */
export const GENERIC_ERROR = "Something went wrong. Please try again.";

/**
 * The ONE place error codes become user-facing copy (M6 polish rule).
 * Every entry is plain words: what happened + what to do — no codes, no
 * jargon. Covers all codes from src-tauri/src/error.rs plus the
 * frontend-only codes thrown in src/lib/auth.ts.
 */
export const AUTH_ERROR_COPY: Record<string, string> = {
  port_in_use:
    "Another program on this computer is using the connection Cued needs for the Spotify login. Close other apps (or restart the computer) and try again.",
  callback_timeout:
    "We didn't hear back from Spotify. Try again and finish the login in the browser window that opens.",
  state_mismatch:
    "The answer from Spotify didn't belong to this login attempt, so Cued discarded it to be safe. Try again.",
  access_denied:
    "The Spotify page was closed with “Cancel”. If that wasn't intended, try again and choose “Agree”.",
  spotify_auth:
    "Spotify didn't accept the login. On the Spotify dashboard, check that your app has the Web API checked and the redirect address matches exactly, then try again.",
  no_client_id:
    "No Client ID is saved yet. Go back one step and paste it first.",
  invalid_client_id:
    "That Client ID wasn't accepted. Compare it with the one shown under your app's name on the Spotify dashboard.",
  keychain:
    "Cued couldn't store the login securely on this computer. Unlock your keychain and try again.",
  network:
    "Couldn't reach Spotify. Check your internet connection and try again.",
  api: "Spotify had trouble answering. Wait a moment and try again.",
  malformed_response:
    "Spotify sent an answer Cued couldn't read. Try again in a moment.",
  rate_limited:
    "Spotify asked Cued to slow down for a moment. Wait a little, then try again.",
  config:
    "Cued couldn't read or write its settings on this computer. Restart the app and try again.",
  login_in_progress:
    "A Spotify login is already open. Finish it in your browser, or wait a moment and try again.",
  bad_callback:
    "The browser sent back something Cued didn't expect. Try again.",
  timeout: "The app didn't respond in time. Try again.",
  bad_ipc_shape:
    "Something went wrong inside the app. Restart Cued and try again.",
};

/**
 * Turn any thrown value into display-ready copy: mapped friendly text for
 * known auth codes, the backend's own display-ready message for other
 * AuthErrors (e.g. preset validation), the generic line for everything else.
 */
export function friendlyAuthMessage(err: unknown): string {
  if (err instanceof AuthError) {
    return AUTH_ERROR_COPY[err.code] ?? err.message;
  }
  return GENERIC_ERROR;
}
