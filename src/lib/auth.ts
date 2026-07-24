import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

/** Connected-user info returned by the Rust backend. */
export const profileSchema = z.object({
  displayName: z.string(),
  isPremium: z.boolean(),
});
export type Profile = z.infer<typeof profileSchema>;

/** Result of the startup session-restore attempt. */
export const authStatusSchema = z.object({
  connected: z.boolean(),
  profile: profileSchema.nullable(),
});
export type AuthStatus = z.infer<typeof authStatusSchema>;

const backendErrorSchema = z.object({
  code: z.string(),
  message: z.string(),
});

/** Typed error thrown by every auth IPC wrapper. `message` is display-ready. */
export class AuthError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "AuthError";
    this.code = code;
  }
}

function toAuthError(raw: unknown): AuthError {
  const parsed = backendErrorSchema.safeParse(raw);
  if (parsed.success) {
    return new AuthError(parsed.data.code, parsed.data.message);
  }
  return new AuthError("unknown", "Something went wrong. Please try again.");
}

/**
 * Invoke a Tauri command, Zod-parse the result, and normalize failures to
 * AuthError. Every call gets a hard timeout — a hang is worse than an error.
 * Shared by all IPC wrapper modules in src/lib/ — never inline invoke().
 */
export async function call<T>(
  command: string,
  schema: z.ZodType<T>,
  args?: Record<string, unknown>,
  timeoutMs = 30_000,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new AuthError("timeout", "The app did not respond in time."));
    }, timeoutMs);
  });
  try {
    const raw = await Promise.race([invoke(command, args), timeout]);
    const parsed = schema.safeParse(raw);
    if (!parsed.success) {
      throw new AuthError("bad_ipc_shape", "Unexpected response from the app backend.");
    }
    return parsed.data;
  } catch (err) {
    throw err instanceof AuthError ? err : toAuthError(err);
  } finally {
    clearTimeout(timer);
  }
}

/** Rust `()` command results arrive as `null` over IPC. */
export const unitSchema = z.union([z.null(), z.undefined()]);

/** Read the stored Spotify Client ID (null when not configured yet). */
export function getClientId(): Promise<string | null> {
  return call("get_client_id", z.string().nullable());
}

/** Validate and persist a Client ID (not a secret — plain config file). */
export async function saveClientId(clientId: string): Promise<void> {
  await call("save_client_id", unitSchema, { clientId });
}

/** Open the Spotify Developer Dashboard in the system browser (wizard step 1). */
export async function openSpotifyDashboard(): Promise<void> {
  await call("open_spotify_dashboard", unitSchema);
}

/**
 * Run the full PKCE login in the system browser. Resolves once the user has
 * authorized (up to the backend's 5-minute callback deadline).
 */
export function connectSpotify(): Promise<Profile> {
  return call("connect_spotify", profileSchema, undefined, 5.5 * 60_000);
}

/** Restore the session from the OS keychain (refreshes if expired). */
export function getAuthStatus(): Promise<AuthStatus> {
  return call("get_auth_status", authStatusSchema);
}

/** Delete both tokens from the OS keychain and the in-memory copy. */
export async function logout(): Promise<void> {
  await call("logout", unitSchema);
}
