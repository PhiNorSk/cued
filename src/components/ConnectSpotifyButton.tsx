interface ConnectSpotifyButtonProps {
  onClick: () => void;
  disabled?: boolean;
  label?: string;
}

/** Landing CTA — triggers the PKCE login in the system browser. */
export function ConnectSpotifyButton({
  onClick,
  disabled = false,
  label = "Connect with Spotify",
}: ConnectSpotifyButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="rounded-full bg-accent px-8 py-3 text-sm font-semibold text-ground transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-60"
    >
      {label}
    </button>
  );
}
