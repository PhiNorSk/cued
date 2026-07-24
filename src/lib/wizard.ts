/**
 * Pure state machine of the 3-step setup wizard (M6). The UI layer holds a
 * WizardState and derives everything (progress bar, back button, escape
 * hatch) from it — no navigation logic lives in components.
 */

/** The three wizard steps: 1 create app, 2 paste Client ID, 3 connect. */
export type WizardStep = 1 | 2 | 3;

export interface WizardState {
  /** The step currently shown. */
  step: WizardStep;
  /**
   * Highest step the user has reached — progress-bar segments up to here
   * are clickable, so going back never locks the user out of later steps.
   */
  reached: WizardStep;
}

/**
 * Initial state: a stored Client ID skips straight to the connect step
 * (re-auth after logout / expired session); a first-time user starts at 1.
 */
export function initWizard(hasStoredClientId: boolean): WizardState {
  return hasStoredClientId ? { step: 3, reached: 3 } : { step: 1, reached: 1 };
}

/** Move forward one step (never past 3), extending the reached mark. */
export function advance(state: WizardState): WizardState {
  const step = Math.min(state.step + 1, 3) as WizardStep;
  return { step, reached: Math.max(step, state.reached) as WizardStep };
}

/** Move back one step (never below 1). Input on later steps is kept. */
export function back(state: WizardState): WizardState {
  return { step: Math.max(state.step - 1, 1) as WizardStep, reached: state.reached };
}

/** Jump to an already-reached step (progress-bar click); otherwise no-op. */
export function goTo(state: WizardState, step: WizardStep): WizardState {
  return step <= state.reached ? { step, reached: state.reached } : state;
}

/**
 * Escape hatch from the skip-to-connect path ("use a different Client ID"):
 * restart as a fresh setup at step 1. The caller is responsible for
 * forgetting the stored Client ID.
 */
export function restartForNewClientId(): WizardState {
  return { step: 1, reached: 1 };
}
