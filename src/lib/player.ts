import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { call, unitSchema } from "./auth";
import { playbackStateSchema, type PlaybackState } from "./playback";

/** Event name the Rust polling engine pushes playback snapshots under. */
const PLAYBACK_STATE_EVENT = "playback://state";

/**
 * Subscribe to playback snapshots from the Rust poll engine. Malformed
 * payloads are logged and dropped — they never reach the callback.
 * Resolves to the unlisten function; call it in the effect cleanup.
 */
export function onPlaybackState(
  onState: (state: PlaybackState) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(PLAYBACK_STATE_EVENT, (event) => {
    const parsed = playbackStateSchema.safeParse(event.payload);
    if (!parsed.success) {
      console.warn("cued: dropped a malformed playback event", parsed.error.issues);
      return;
    }
    onState(parsed.data);
  });
}

/**
 * Wake signal to the Rust engine (component mount / window focus): a
 * suspended or sleeping poll loop polls again right away.
 */
export async function playerWake(): Promise<void> {
  await call("player_wake", unitSchema);
}

/**
 * Enter or leave preset edit mode (M8): while a track URI is set, the Rust
 * engine suspends automation for exactly that track. Pass null on exit.
 */
export async function setEditMode(trackUri: string | null): Promise<void> {
  await call("set_edit_mode", unitSchema, { trackUri });
}

/**
 * UI-initiated seek (edit-mode preview / exit-restore). The engine absorbs
 * the jump — it never counts as an automation action or a manual seek.
 */
export async function uiSeek(positionMs: number): Promise<void> {
  await call("ui_seek", unitSchema, { positionMs });
}
