import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";
import { call, unitSchema } from "./auth";

/** Event name the backend pushes master-toggle changes under (tray sync). */
const AUTOMATION_ENABLED_EVENT = "automation://enabled";

/** Whether the auto-seek/skip engine is enabled (persisted in config.json). */
export function getAutomationEnabled(): Promise<boolean> {
  return call("get_automation_enabled", z.boolean());
}

/** Persist and apply the automation master toggle. */
export async function setAutomationEnabled(enabled: boolean): Promise<void> {
  await call("set_automation_enabled", unitSchema, { enabled });
}

/**
 * Subscribe to master-toggle changes made outside this UI (the tray menu).
 * Malformed payloads are logged and dropped — they never reach the callback.
 * Resolves to the unlisten function; call it in the effect cleanup.
 */
export function onAutomationEnabled(
  onChange: (enabled: boolean) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(AUTOMATION_ENABLED_EVENT, (event) => {
    const parsed = z.boolean().safeParse(event.payload);
    if (!parsed.success) {
      console.warn(
        "cued: dropped a malformed automation event",
        parsed.error.issues,
      );
      return;
    }
    onChange(parsed.data);
  });
}
