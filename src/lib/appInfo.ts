import { getVersion } from "@tauri-apps/api/app";

import { call, unitSchema } from "./auth";

/** Open the Ko-fi support page in the system browser (Settings footer). */
export async function openSupportPage(): Promise<void> {
  await call("open_support_page", unitSchema);
}

/**
 * App version from tauri.conf.json for the about line. Returns null when the
 * IPC call fails (the footer then simply omits the version) — logged, not
 * surfaced: a missing version number is not actionable for the user.
 */
export async function appVersion(): Promise<string | null> {
  try {
    return await getVersion();
  } catch (err) {
    console.warn("cued: could not read the app version", err);
    return null;
  }
}
