import type { Profile } from "../lib/auth";

interface ConnectedCardProps {
  profile: Profile;
  onLogout: () => void;
  error: string | null;
}

/** Connected state: who is logged in, Premium status, logout. */
export function ConnectedCard({ profile, onLogout, error }: ConnectedCardProps) {
  return (
    <div className="flex w-96 flex-col items-center gap-3">
      <p className="text-sm text-text">
        Connected as{" "}
        <span className="font-semibold">{profile.displayName}</span>
        {" · "}
        <span className={profile.isPremium ? "text-accent-hi" : "text-amber"}>
          {profile.isPremium ? "Spotify Premium" : "Spotify Free"}
        </span>
      </p>
      {!profile.isPremium && (
        <p className="max-w-full text-center text-xs text-amber">
          This account has no Spotify Premium. Playback control (start, skip)
          requires Premium — Cued stays connected, but presets won&apos;t be
          able to control playback.
        </p>
      )}
      {error && (
        <p role="alert" className="max-w-full text-center text-xs text-amber">
          {error}
        </p>
      )}
      <button
        type="button"
        onClick={onLogout}
        className="rounded text-xs text-text-mut underline underline-offset-2 transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
      >
        Log out
      </button>
    </div>
  );
}
