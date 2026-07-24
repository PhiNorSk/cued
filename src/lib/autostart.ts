import { z } from "zod";
import { call, unitSchema } from "./auth";

/**
 * Launch-at-login IPC wrappers (M14). The state lives in the OS (login
 * item), not in config.json — reading it is always a live query, so the
 * settings toggle reflects reality even after changes made outside Cued.
 */

/** Whether Cued is currently registered as an OS login item. */
export function getAutostartEnabled(): Promise<boolean> {
  return call("get_autostart_enabled", z.boolean());
}

/**
 * Register or remove the OS login item. Opt-in only: reached exclusively
 * from the explicit settings toggle, never on Cued's own initiative.
 */
export async function setAutostartEnabled(enabled: boolean): Promise<void> {
  await call("set_autostart_enabled", unitSchema, { enabled });
}
