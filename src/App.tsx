import { useEffect, useState } from "react";
import { AutomationToggle } from "./components/AutomationToggle";
import { ConnectedCard } from "./components/ConnectedCard";
import { Library } from "./components/Library";
import { Logo } from "./components/Logo";
import { NowPlaying } from "./components/NowPlaying";
import { SettingsPanel } from "./components/SettingsPanel";
import { SetupWizard } from "./components/SetupWizard";
import { useAutomation } from "./hooks/useAutomation";
import { appVersion } from "./lib/appInfo";
import {
  connectSpotify,
  getAuthStatus,
  getClientId,
  logout,
  saveClientId,
  type Profile,
} from "./lib/auth";
import { cn } from "./lib/cn";
import { appCopy, settingsCopy } from "./lib/copy";
import { friendlyAuthMessage } from "./lib/errorCopy";
import { getPresetDbHealth, type PresetDbHealth } from "./lib/presets";
import { analyzeSuggestions } from "./lib/suggestions";

type ConnState =
  | { phase: "loading" }
  | { phase: "disconnected"; storedClientId: string | null; error: string | null }
  | { phase: "connected"; profile: Profile; error: string | null };

type Tab = "now-playing" | "library";

function App() {
  const [conn, setConn] = useState<ConnState>({ phase: "loading" });
  const [tab, setTab] = useState<Tab>("now-playing");
  const [dbHealth, setDbHealth] = useState<PresetDbHealth | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const automation = useAutomation();

  useEffect(() => {
    let cancelled = false;
    const restore = async () => {
      let restoreError: string | null = null;
      try {
        const status = await getAuthStatus();
        if (cancelled) return;
        if (status.connected && status.profile) {
          setConn({ phase: "connected", profile: status.profile, error: null });
          return;
        }
      } catch (err) {
        restoreError = friendlyAuthMessage(err);
      }
      const storedClientId = await getClientId().catch(() => null);
      if (!cancelled) {
        setConn({ phase: "disconnected", storedClientId, error: restoreError });
      }
    };
    void restore();
    return () => {
      cancelled = true;
    };
  }, []);

  // One-time preset-storage health check for the startup notice (corrupt DB
  // rescued / storage unavailable). Independent of the Spotify session.
  useEffect(() => {
    let cancelled = false;
    getPresetDbHealth()
      .then((health) => {
        if (!cancelled) setDbHealth(health);
      })
      .catch((err: unknown) => {
        console.warn("cued: could not read preset db health", err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void appVersion().then((v) => {
      if (!cancelled) setVersion(v);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Refresh suggestion analysis when the app regains focus (one of the
  // opportunistic, debounced triggers; the backend coalesces overlapping runs
  // and only touches tracks with new events). Only while connected.
  useEffect(() => {
    if (conn.phase !== "connected") {
      return;
    }
    const onFocus = () => {
      void analyzeSuggestions().catch(() => undefined);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [conn.phase]);

  /** Wizard step 3: persist the Client ID, then run the PKCE login. */
  const connectWithClientId = async (clientId: string): Promise<Profile> => {
    await saveClientId(clientId);
    return connectSpotify();
  };

  // The playback engine lost its session (refresh failed for good).
  // getAuthStatus re-checks and wipes the dead tokens, so the app lands on
  // the wizard's connect step instead of erroring.
  const handleAuthLost = async () => {
    try {
      const status = await getAuthStatus();
      if (status.connected && status.profile) {
        setConn({ phase: "connected", profile: status.profile, error: null });
        return;
      }
    } catch {
      // fall through to the disconnected screen below
    }
    const storedClientId = await getClientId().catch(() => null);
    setConn({
      phase: "disconnected",
      storedClientId,
      error: appCopy.sessionExpired,
    });
  };

  const handleLogout = async (profile: Profile) => {
    try {
      await logout();
      const storedClientId = await getClientId().catch(() => null);
      setConn({ phase: "disconnected", storedClientId, error: null });
    } catch (err) {
      setConn({ phase: "connected", profile, error: friendlyAuthMessage(err) });
    }
  };

  if (conn.phase === "connected") {
    return (
      <main className="flex h-full flex-col bg-ground">
        <header className="flex items-center gap-3 border-b border-hairline px-6 py-3">
          <Logo className="h-7 w-7" />
          <span className="text-lg font-semibold tracking-tight text-text">
            Cued
          </span>
          <nav role="tablist" aria-label="Views" className="ml-6 flex gap-1">
            <TabButton
              label="Now Playing"
              active={tab === "now-playing"}
              onClick={() => setTab("now-playing")}
            />
            <TabButton
              label="Library"
              active={tab === "library"}
              onClick={() => setTab("library")}
            />
          </nav>
          <div className="ml-auto flex min-w-0 items-center gap-2">
            <div className="min-w-0">
              <AutomationToggle
                enabled={automation.enabled}
                ready={automation.ready}
                suspension={automation.suspension}
                error={automation.error}
                onChange={automation.setEnabled}
              />
            </div>
            <button
              type="button"
              aria-label={settingsCopy.open}
              onClick={() => {
                setSettingsOpen(true);
              }}
              className="shrink-0 rounded-full p-1.5 text-text-mut transition-colors hover:bg-surface-2 hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
            >
              <GearIcon className="h-5 w-5" />
            </button>
          </div>
        </header>
        <DbHealthNotice health={dbHealth} />
        <div
          key={tab}
          className="anim-rise-in flex min-h-0 flex-1 flex-col items-center overflow-y-auto p-6"
        >
          {tab === "now-playing" ? (
            <div className="my-auto flex flex-col items-center gap-6">
              <NowPlaying
                automationOn={automation.enabled}
                isPremium={conn.profile.isPremium}
                onAuthLost={() => {
                  void handleAuthLost();
                }}
              />
              <ConnectedCard
                profile={conn.profile}
                error={conn.error}
                onLogout={() => {
                  void handleLogout(conn.profile);
                }}
              />
            </div>
          ) : (
            <div className="w-full max-w-xl">
              <Library />
            </div>
          )}
        </div>
        {settingsOpen && (
          <SettingsPanel
            version={version}
            onClose={() => {
              setSettingsOpen(false);
            }}
          />
        )}
      </main>
    );
  }

  return (
    <main className="flex h-full flex-col bg-ground">
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-8 overflow-y-auto p-6">
        <div className="flex flex-col items-center gap-4">
          <Logo className="h-12 w-12" />
          <div className="flex flex-col items-center gap-1.5">
            <h1 className="text-3xl font-semibold tracking-tight text-text">
              Cued
            </h1>
            <p className="text-sm text-text-mut">
              Per-song start &amp; skip presets for Spotify
            </p>
          </div>
        </div>

        {conn.phase === "loading" ? (
          <p className="text-sm text-text-mut">Checking session…</p>
        ) : (
          <SetupWizard
            storedClientId={conn.storedClientId}
            initialError={conn.error}
            connect={connectWithClientId}
            onConnected={(profile) => {
              setConn({ phase: "connected", profile, error: null });
            }}
            onForgetClientId={() => {
              setConn({
                phase: "disconnected",
                storedClientId: null,
                error: null,
              });
            }}
          />
        )}
      </div>
    </main>
  );
}

function TabButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "rounded-full px-4 py-1.5 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi",
        active ? "bg-surface-2 text-text" : "text-text-mut hover:text-text",
      )}
    >
      {label}
    </button>
  );
}

/** Startup notice when the preset database was rescued or is unavailable. */
function DbHealthNotice({ health }: { health: PresetDbHealth | null }) {
  if (!health || (!health.recovered && health.failed === null)) {
    return null;
  }
  return (
    <p
      role="alert"
      className="border-b border-hairline bg-surface px-6 py-2 text-center text-xs text-amber"
    >
      {health.failed !== null
        ? `Preset storage is unavailable: ${health.failed}`
        : "Your preset library file was damaged and has been reset. The old file was kept next to it in the app data folder."}
    </p>
  );
}

/** Cog icon for the header settings button. */
function GearIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

export default App;
