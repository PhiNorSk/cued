import { z } from "zod";
import { call, unitSchema } from "./auth";

/** A stored preset incl. the metadata snapshot taken at save time. */
export const presetSchema = z.object({
  trackUri: z.string(),
  title: z.string(),
  artists: z.array(z.string()),
  coverUrl: z.string().nullable(),
  durationMs: z.number().nonnegative(),
  startMs: z.number().nonnegative(),
  skipMs: z.number().nonnegative(),
  createdAt: z.number(),
  updatedAt: z.number(),
});
export type Preset = z.infer<typeof presetSchema>;

/** What the frontend sends to create/update a preset (no timestamps). */
export type PresetInput = Omit<Preset, "createdAt" | "updatedAt">;

/** Startup health of the preset database. */
export const presetDbHealthSchema = z.object({
  /** True when a corrupt database was set aside and recreated this run. */
  recovered: z.boolean(),
  /** Human-readable reason when the store could not be opened at all. */
  failed: z.string().nullable(),
});
export type PresetDbHealth = z.infer<typeof presetDbHealthSchema>;

/** Validate (in Rust, authoritative) and upsert a preset; returns the row. */
export function savePreset(preset: PresetInput): Promise<Preset> {
  return call("save_preset", presetSchema, { preset });
}

/** Read the preset for one track URI (null when none exists). */
export function getPreset(trackUri: string): Promise<Preset | null> {
  return call("get_preset", presetSchema.nullable(), { trackUri });
}

/** All stored presets, newest first. */
export function listPresets(): Promise<Preset[]> {
  return call("list_presets", z.array(presetSchema));
}

/** Delete the preset for one track URI (idempotent). */
export async function deletePreset(trackUri: string): Promise<void> {
  await call("delete_preset", unitSchema, { trackUri });
}

/** Startup health of the preset database, for the one-time UI notice. */
export function getPresetDbHealth(): Promise<PresetDbHealth> {
  return call("get_preset_db_health", presetDbHealthSchema);
}
