/**
 * All user-facing strings of the setup wizard plus shared app copy, in one
 * module so a later i18n pass only has to touch this file (M6 constraint —
 * no literals scattered in JSX).
 */

/** Must match src-tauri/src/server.rs exactly (BIND_ADDR + CALLBACK_PATH). */
export const REDIRECT_URI = "http://127.0.0.1:8917/callback";

export const SPOTIFY_DASHBOARD_HOST = "developer.spotify.com/dashboard";

export const wizardCopy = {
  stepLabels: ["Create app", "Client ID", "Connect"] as const,
  progressLabel: "Setup progress",
  back: "Back",
  next: "Next",

  step1: {
    title: "Create your free Spotify app",
    why: "Cued is independent and free, so Spotify asks every user to create their own free access key. It takes about a minute, needs no coding, and stays on your Spotify account.",
    openDashboard: "Open Spotify Dashboard",
    openDashboardHint: `Opens ${SPOTIFY_DASHBOARD_HOST} in your browser.`,
    instructions: [
      "Log in with your Spotify account",
      "Click “Create app”",
      "Give it any name and description",
      "Check “Web API”",
    ] as const,
    redirectLabel: "Redirect URI — paste this exactly",
    redirectNote:
      "Spotify asks for a “Redirect URI” in the same form. It must match this address character for character.",
    copy: "Copy",
    copied: "Copied ✓",
    copyFailed: "Couldn't copy — select and copy it by hand.",
  },

  step2: {
    title: "Paste your Client ID",
    lead: "Open your new app on the Spotify dashboard. The Client ID is the ~32-character code shown under the app's name.",
    placeholder: "Paste your Client ID",
    valid: "Looks good ✓",
    invalid:
      "That doesn't look right — the Client ID is a ~32-character code of letters and digits found under your app's name.",
  },

  step3: {
    title: "Connect to Spotify",
    lead: "Cued opens Spotify in your browser so you can allow access. You only do this once.",
    connect: "Connect with Spotify",
    connecting: "Waiting for Spotify in your browser…",
    connectingHint: "This screen updates automatically once you're done.",
    connectedAs: (name: string) => `Connected as ${name} ✓`,
    tryAgain: "Try again",
    useDifferentClientId: "Use a different Client ID",
  },
} as const;

export const appCopy = {
  sessionExpired: "Your Spotify session expired. Please connect again.",
  aboutLine: (version: string) => `Cued v${version}`,
  libraryEmptyTitle: "No presets yet",
  libraryEmptyHint:
    "Play a song in Spotify, then set its start and skip points on the Now Playing tab — it will appear here.",
} as const;

export const settingsCopy = {
  title: "Settings",
  open: "Settings",
  close: "Close",
  insightsHeading: "Listening insights",
  insightsBlurb:
    "Cued can record where you skip or seek, so it can suggest better start and skip points later. This stays only on your device, is never uploaded, and you can delete it anytime.",
  deleteAction: "Delete all insights data",
  deleteConfirmPrompt: "Delete all collected insights? This can't be undone.",
  deleteConfirm: "Delete",
  deleteCancel: "Cancel",
  // M10 suggestion controls.
  suggestionsHeading: "Suggestions",
  suggestSkipPoints: "Suggest skip points",
  suggestStartPoints: "Suggest start points",
  suggestAutoSkip: "Suggest auto-skipping songs",
  // Shown ONCE here, never on every card.
  provenance:
    "Based on your listening on this device. Nothing leaves your computer.",
  // M14 launch at login (opt-in, default off).
  startupHeading: "Startup",
  autostartLabel: "Start Cued at login",
  autostartBlurb:
    "Cued starts quietly in the menu bar when you log in — no window opens until you ask for it.",
  // Voluntary tip-jar link (M13). A quiet footer row — never a banner, a
  // nag, or a feature gate.
  supportLabel: "Support Cued ♥",
  supportFailed: "Couldn't open your browser — you can find Cued on ko-fi.com.",
} as const;

/**
 * All suggestion-surface copy (M10). Evidence-first, calm — never "we
 * noticed", never exclamation marks. Builders take already-formatted m:ss
 * strings so time formatting stays in one place (`formatTimeMs`).
 */
export const suggestionsCopy = {
  // Proactive-card questions (observed fact + a quiet question).
  skipPointFact: (region: string, matching: number, total: number) =>
    `You've skipped from ${region} in ${matching} of your last ${total} plays.`,
  startPointFact: (target: string, matching: number, total: number) =>
    `You've jumped to ${target} in ${matching} of your last ${total} plays.`,
  autoSkipFact: (matching: number, total: number) =>
    `You skip this song early in ${matching} of your last ${total} plays.`,

  setSkipPoint: "Set skip point",
  setStartPoint: "Set start point",
  autoSkipIt: "Auto-skip it",
  keepPlaying: "Keep playing it",
  noThanks: "No thanks",

  // Applied morph (the card becomes a quiet confirmation with an escape hatch).
  skipPointSet: (region: string) => `Skip point set ${region}`,
  startPointSet: (target: string) => `Start point set ${target}`,
  autoSkipSet: "Cued will skip this song from now on",
  adjust: "Adjust",
  undo: "Undo",

  // Library "Suggestions (n)" section.
  librarySectionTitle: "Suggestions",
  autoSkippedBadge: "Auto-skipped",
  turnOff: "Turn off",
  dismiss: "Dismiss",
} as const;
