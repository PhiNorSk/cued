/**
 * Copy-to-clipboard with a short visual confirmation (wizard step 1).
 * The feedback transitions are pure functions so they are unit-testable;
 * the component wires them to a timer.
 */

/** How long the "Copied ✓" confirmation stays visible. */
export const COPY_CONFIRM_MS = 2000;

/** What the copy button currently shows. */
export type CopyFeedback = "idle" | "copied" | "failed";

/** Feedback to show once a copy attempt has resolved. */
export function copyFeedback(succeeded: boolean): CopyFeedback {
  return succeeded ? "copied" : "failed";
}

/** Feedback after the confirmation window has elapsed. */
export function expireFeedback(current: CopyFeedback): CopyFeedback {
  return current === "idle" ? current : "idle";
}

/**
 * Write text to the system clipboard. Uses the async clipboard API when the
 * webview exposes it and falls back to a hidden textarea + execCommand
 * (the API is gated on secure contexts, which the dev server is not).
 * Returns whether the copy succeeded — never throws.
 */
export async function copyTextToClipboard(text: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch (err) {
      console.warn("cued: async clipboard write failed, falling back", err);
    }
  }
  return execCommandCopy(text);
}

function execCommandCopy(text: string): boolean {
  if (typeof document === "undefined") {
    return false;
  }
  const textarea = document.createElement("textarea");
  textarea.value = text;
  // keep it out of view without display:none (which would prevent selection)
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch (err) {
    console.warn("cued: execCommand copy failed", err);
  }
  textarea.remove();
  return ok;
}
