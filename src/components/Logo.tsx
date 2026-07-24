import { cn } from "../lib/cn";

type LogoProps = {
  className?: string;
};

/**
 * Cued mark: a track ring with a cue point and a play head.
 * The ring gap + dot read as "the spot the song is cued to".
 */
export function Logo({ className }: LogoProps) {
  return (
    <svg
      viewBox="0 0 48 48"
      fill="none"
      role="img"
      aria-label="Cued logo"
      className={cn("text-accent", className)}
    >
      <circle
        cx="24"
        cy="24"
        r="19"
        stroke="currentColor"
        strokeWidth="3.5"
        strokeLinecap="round"
        strokeDasharray="95 25"
        transform="rotate(-58 24 24)"
      />
      <circle cx="24" cy="5" r="4" fill="var(--accent-hi)" />
      <path d="M20 17.5v13l11-6.5z" fill="var(--text)" />
    </svg>
  );
}
