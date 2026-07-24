import { z } from "zod";
import { call, unitSchema } from "./auth";

/**
 * Listening-insights IPC wrappers (M9). Collection only — Cued records where
 * the user skips/seeks, stored locally; this module exposes the settings
 * surface (toggle, count, delete-all). No suggestions or visualization yet.
 */

/** Whether insight collection is enabled (persisted in config.json). */
export function getInsightsEnabled(): Promise<boolean> {
  return call("get_insights_enabled", z.boolean());
}

/** Persist and apply the insights master toggle. */
export async function setInsightsEnabled(enabled: boolean): Promise<void> {
  await call("set_insights_enabled", unitSchema, { enabled });
}

/** How many events have been collected so far. */
export function getInsightsCount(): Promise<number> {
  return call("get_insights_count", z.number().nonnegative());
}

/** Erase ALL collected insights data (both tables). Presets are untouched. */
export async function deleteAllInsights(): Promise<void> {
  await call("delete_all_insights", unitSchema);
}

/**
 * Human-readable count for the settings surface. Pure so it is unit-tested
 * without any IPC: pluralizes and gives an inviting empty state.
 */
export function insightsCountLabel(count: number): string {
  if (count <= 0) {
    return "No events collected yet";
  }
  if (count === 1) {
    return "1 event collected";
  }
  return `${count.toLocaleString()} events collected`;
}
