/**
 * Frontend mirror of the Rust-side Client ID validation (src-tauri/src/config.rs)
 * for instant inline feedback. The Rust side re-validates at the IPC boundary —
 * this is UX, not the security check.
 *
 * Spotify Client IDs are 32 hex chars today; 20–40 alphanumeric ASCII chars are
 * accepted to survive minor format changes without letting garbage through.
 */
export function isValidClientId(clientId: string): boolean {
  return /^[A-Za-z0-9]{20,40}$/.test(clientId);
}
