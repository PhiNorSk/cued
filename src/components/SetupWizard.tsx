import { useEffect, useRef, useState } from "react";
import { openSpotifyDashboard, type Profile } from "../lib/auth";
import { isValidClientId } from "../lib/clientId";
import {
  COPY_CONFIRM_MS,
  copyFeedback,
  copyTextToClipboard,
  expireFeedback,
  type CopyFeedback,
} from "../lib/clipboard";
import { cn } from "../lib/cn";
import { REDIRECT_URI, wizardCopy } from "../lib/copy";
import { friendlyAuthMessage } from "../lib/errorCopy";
import {
  advance,
  back,
  goTo,
  initWizard,
  restartForNewClientId,
  type WizardState,
  type WizardStep,
} from "../lib/wizard";
import { ConnectSpotifyButton } from "./ConnectSpotifyButton";

/** How long the "Connected as … ✓" confirmation stays before entering the app. */
const SUCCESS_ADVANCE_MS = 1500;

type ConnectStatus =
  | { kind: "idle" }
  | { kind: "connecting" }
  | { kind: "success"; profile: Profile }
  | { kind: "error"; message: string };

interface SetupWizardProps {
  /** Client ID already stored on disk, or null for a first-time setup. */
  storedClientId: string | null;
  /** Error carried in from outside the wizard (e.g. an expired session). */
  initialError: string | null;
  /** Persist the Client ID and run the PKCE login; resolves with the profile. */
  connect: (clientId: string) => Promise<Profile>;
  /** Called once the success confirmation has been shown. */
  onConnected: (profile: Profile) => void;
  /** Forget the stored Client ID (escape hatch from the re-connect path). */
  onForgetClientId: () => void;
}

/**
 * The guided 3-step first-run setup (M6): create a Spotify app → paste the
 * Client ID → connect. With a stored Client ID it opens directly on the
 * connect step; "Use a different Client ID" restarts from step 1. All step
 * navigation is the pure machine in src/lib/wizard.ts; all copy lives in
 * src/lib/copy.ts. Local state survives window hide/show because the window
 * is only hidden, never unmounted.
 */
export function SetupWizard({
  storedClientId,
  initialError,
  connect,
  onConnected,
  onForgetClientId,
}: SetupWizardProps) {
  const [wizard, setWizard] = useState<WizardState>(() =>
    initWizard(storedClientId !== null),
  );
  const [input, setInput] = useState(storedClientId ?? "");
  const [touched, setTouched] = useState(false);
  const [copyState, setCopyState] = useState<CopyFeedback>("idle");
  const [dashboardError, setDashboardError] = useState<string | null>(null);
  const [status, setStatus] = useState<ConnectStatus>(() =>
    initialError !== null
      ? { kind: "error", message: initialError }
      : { kind: "idle" },
  );

  const trimmed = input.trim();
  const validity: "neutral" | "valid" | "invalid" =
    trimmed === "" ? "neutral" : isValidClientId(trimmed) ? "valid" : "invalid";
  // Navigation is frozen while a login runs or the success line is shown.
  const locked = status.kind === "connecting" || status.kind === "success";

  const copyTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  const handleCopy = async () => {
    const ok = await copyTextToClipboard(REDIRECT_URI);
    setCopyState(copyFeedback(ok));
    window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => {
      setCopyState((current) => expireFeedback(current));
    }, COPY_CONFIRM_MS);
  };

  const handleOpenDashboard = async () => {
    setDashboardError(null);
    try {
      await openSpotifyDashboard();
    } catch (err) {
      setDashboardError(friendlyAuthMessage(err));
    }
  };

  const handleConnect = async () => {
    if (validity !== "valid") {
      // Only reachable via a stored Client ID that fails today's rules —
      // send the user to the paste step instead of a doomed login.
      setTouched(true);
      setWizard((w) => goTo(w, 2));
      return;
    }
    setStatus({ kind: "connecting" });
    try {
      const profile = await connect(trimmed);
      setStatus({ kind: "success", profile });
    } catch (err) {
      setStatus({ kind: "error", message: friendlyAuthMessage(err) });
    }
  };

  // Show the success line briefly, then let the app take over. The timer is
  // cleaned up on unmount; hide/show does not unmount, so it still fires.
  useEffect(() => {
    if (status.kind !== "success") {
      return;
    }
    const profile = status.profile;
    const timer = window.setTimeout(() => {
      onConnected(profile);
    }, SUCCESS_ADVANCE_MS);
    return () => window.clearTimeout(timer);
  }, [status, onConnected]);

  const handleUseDifferentClientId = () => {
    onForgetClientId();
    setInput("");
    setTouched(false);
    setStatus({ kind: "idle" });
    setWizard(restartForNewClientId());
  };

  return (
    <div className="flex w-[26rem] max-w-full flex-col gap-6">
      <StepProgress
        wizard={wizard}
        locked={locked}
        onGoTo={(step) => setWizard((w) => goTo(w, step))}
      />

      <div key={wizard.step} className="anim-rise-in flex flex-col gap-5">
        {wizard.step === 1 && (
          <StepCreateApp
            copyState={copyState}
            dashboardError={dashboardError}
            onOpenDashboard={() => {
              void handleOpenDashboard();
            }}
            onCopy={() => {
              void handleCopy();
            }}
          />
        )}
        {wizard.step === 2 && (
          <StepClientId
            input={input}
            validity={validity}
            showInvalid={validity === "invalid" && touched}
            onChange={(value) => {
              setInput(value);
              // a fresh Client ID invalidates any previous login failure
              setStatus((s) => (s.kind === "error" ? { kind: "idle" } : s));
            }}
            onBlur={() => setTouched(true)}
          />
        )}
        {wizard.step === 3 && (
          <StepConnect
            status={status}
            onConnect={() => {
              void handleConnect();
            }}
            onUseDifferentClientId={handleUseDifferentClientId}
          />
        )}

        <nav className="flex items-center justify-between">
          {wizard.step > 1 ? (
            <button
              type="button"
              disabled={locked}
              onClick={() => setWizard((w) => back(w))}
              className="rounded-full px-3 py-1.5 text-xs font-medium text-text-mut transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
            >
              ← {wizardCopy.back}
            </button>
          ) : (
            <span aria-hidden />
          )}
          {wizard.step < 3 && (
            <button
              type="button"
              disabled={wizard.step === 2 && validity !== "valid"}
              onClick={() => setWizard((w) => advance(w))}
              className="rounded-full bg-accent px-5 py-1.5 text-xs font-semibold text-ground transition-colors hover:bg-accent-hi focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-not-allowed disabled:opacity-50"
            >
              {wizardCopy.next} →
            </button>
          )}
        </nav>
      </div>
    </div>
  );
}

/** Three labeled segments; visited ones are clickable to jump back or forward. */
function StepProgress({
  wizard,
  locked,
  onGoTo,
}: {
  wizard: WizardState;
  locked: boolean;
  onGoTo: (step: WizardStep) => void;
}) {
  return (
    <ol aria-label={wizardCopy.progressLabel} className="flex w-full gap-2">
      {wizardCopy.stepLabels.map((label, index) => {
        const step = (index + 1) as WizardStep;
        const current = step === wizard.step;
        return (
          <li key={label} className="min-w-0 flex-1">
            <button
              type="button"
              aria-current={current ? "step" : undefined}
              disabled={locked || step > wizard.reached}
              onClick={() => onGoTo(step)}
              className="w-full rounded focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi disabled:cursor-default"
            >
              <span
                aria-hidden
                className={cn(
                  "block h-1 rounded-full transition-colors",
                  current
                    ? "bg-accent-hi"
                    : step <= wizard.reached
                      ? "bg-accent"
                      : "bg-surface-2",
                )}
              />
              <span
                className={cn(
                  "mt-1.5 block truncate text-center text-[11px] font-medium transition-colors",
                  current ? "text-text" : "text-text-mut",
                )}
              >
                {label}
              </span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}

function StepCreateApp({
  copyState,
  dashboardError,
  onOpenDashboard,
  onCopy,
}: {
  copyState: CopyFeedback;
  dashboardError: string | null;
  onOpenDashboard: () => void;
  onCopy: () => void;
}) {
  const copy = wizardCopy.step1;
  return (
    <section className="flex flex-col gap-4">
      <header>
        <h2 className="text-base font-semibold text-text">{copy.title}</h2>
        <p className="mt-1 text-sm leading-relaxed text-text-mut">{copy.why}</p>
      </header>

      <div className="flex flex-col items-start gap-1.5">
        <button
          type="button"
          onClick={onOpenDashboard}
          className="rounded-full border border-hairline bg-surface-2 px-4 py-2 text-sm font-medium text-text transition-colors hover:border-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
        >
          {copy.openDashboard} ↗
        </button>
        <p className="text-xs text-text-mut">{copy.openDashboardHint}</p>
        {dashboardError && (
          <p role="alert" className="text-xs text-amber">
            {dashboardError}
          </p>
        )}
      </div>

      <ol className="flex flex-col gap-1.5 rounded-lg border border-hairline bg-surface p-3">
        {copy.instructions.map((text, index) => (
          <li key={text} className="flex items-baseline gap-2 text-sm text-text">
            <span className="w-4 shrink-0 text-right text-xs font-semibold tabular-nums text-accent-hi">
              {index + 1}.
            </span>
            {text}
          </li>
        ))}
      </ol>

      <div className="flex flex-col gap-1.5">
        <p className="text-xs font-medium text-text">{copy.redirectLabel}</p>
        <div className="flex items-stretch gap-2">
          <code className="min-w-0 flex-1 select-text truncate rounded-lg border border-hairline bg-surface px-3 py-2 font-mono text-sm text-text">
            {REDIRECT_URI}
          </code>
          <button
            type="button"
            onClick={onCopy}
            className={cn(
              "w-24 shrink-0 rounded-lg border text-xs font-semibold transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi",
              copyState === "copied"
                ? "border-accent bg-accent/15 text-accent-hi"
                : "border-hairline bg-surface-2 text-text hover:border-accent",
            )}
          >
            {copyState === "copied" ? copy.copied : copy.copy}
          </button>
        </div>
        <p className="text-xs text-text-mut">{copy.redirectNote}</p>
        {copyState === "failed" && (
          <p role="alert" className="text-xs text-amber">
            {copy.copyFailed}
          </p>
        )}
      </div>
    </section>
  );
}

function StepClientId({
  input,
  validity,
  showInvalid,
  onChange,
  onBlur,
}: {
  input: string;
  validity: "neutral" | "valid" | "invalid";
  showInvalid: boolean;
  onChange: (value: string) => void;
  onBlur: () => void;
}) {
  const copy = wizardCopy.step2;
  return (
    <section className="flex flex-col gap-3">
      <header>
        <h2 className="text-base font-semibold text-text">{copy.title}</h2>
        <p className="mt-1 text-sm leading-relaxed text-text-mut">{copy.lead}</p>
      </header>
      <input
        type="text"
        value={input}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        placeholder={copy.placeholder}
        aria-invalid={showInvalid}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        className={cn(
          "w-full rounded-lg border bg-surface px-4 py-2.5 font-mono text-sm text-text placeholder:font-sans placeholder:text-text-mut focus:outline-none",
          validity === "valid"
            ? "border-accent"
            : showInvalid
              ? "border-amber"
              : "border-hairline focus:border-accent",
        )}
      />
      {validity === "valid" && (
        <p className="text-xs text-accent-hi">{copy.valid}</p>
      )}
      {showInvalid && (
        <p role="alert" className="text-xs leading-relaxed text-amber">
          {copy.invalid}
        </p>
      )}
    </section>
  );
}

function StepConnect({
  status,
  onConnect,
  onUseDifferentClientId,
}: {
  status: ConnectStatus;
  onConnect: () => void;
  onUseDifferentClientId: () => void;
}) {
  const copy = wizardCopy.step3;
  return (
    <section className="flex flex-col items-center gap-4 text-center">
      <header>
        <h2 className="text-base font-semibold text-text">{copy.title}</h2>
        <p className="mt-1 text-sm leading-relaxed text-text-mut">{copy.lead}</p>
      </header>

      {status.kind === "connecting" ? (
        <div className="flex flex-col items-center gap-2 py-2">
          <p className="text-sm text-text">{copy.connecting}</p>
          <p className="text-xs text-text-mut">{copy.connectingHint}</p>
        </div>
      ) : status.kind === "success" ? (
        <p className="anim-rise-in py-3 text-sm font-medium text-accent-hi">
          {copy.connectedAs(status.profile.displayName)}
        </p>
      ) : (
        <>
          {status.kind === "error" && (
            <p
              role="alert"
              className="anim-rise-in max-w-full text-xs leading-relaxed text-amber"
            >
              {status.message}
            </p>
          )}
          <ConnectSpotifyButton
            onClick={onConnect}
            label={status.kind === "error" ? copy.tryAgain : copy.connect}
          />
          <button
            type="button"
            onClick={onUseDifferentClientId}
            className="rounded text-xs text-text-mut underline underline-offset-2 transition-colors hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent-hi"
          >
            {copy.useDifferentClientId}
          </button>
        </>
      )}
    </section>
  );
}
