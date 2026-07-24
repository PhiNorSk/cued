import { describe, expect, it } from "vitest";
import {
  advance,
  back,
  goTo,
  initWizard,
  restartForNewClientId,
} from "./wizard";

describe("initWizard", () => {
  it("starts a first-time user on step 1", () => {
    expect(initWizard(false)).toEqual({ step: 1, reached: 1 });
  });

  it("skips straight to step 3 when a Client ID is already stored", () => {
    expect(initWizard(true)).toEqual({ step: 3, reached: 3 });
  });
});

describe("advance", () => {
  it("moves forward one step and extends the reached mark", () => {
    expect(advance({ step: 1, reached: 1 })).toEqual({ step: 2, reached: 2 });
    expect(advance({ step: 2, reached: 2 })).toEqual({ step: 3, reached: 3 });
  });

  it("does not advance past step 3", () => {
    expect(advance({ step: 3, reached: 3 })).toEqual({ step: 3, reached: 3 });
  });

  it("keeps the reached mark when re-advancing after going back", () => {
    expect(advance({ step: 1, reached: 3 })).toEqual({ step: 2, reached: 3 });
  });
});

describe("back", () => {
  it("moves back one step without losing the reached mark", () => {
    expect(back({ step: 3, reached: 3 })).toEqual({ step: 2, reached: 3 });
    expect(back({ step: 2, reached: 3 })).toEqual({ step: 1, reached: 3 });
  });

  it("does not go below step 1", () => {
    expect(back({ step: 1, reached: 2 })).toEqual({ step: 1, reached: 2 });
  });
});

describe("goTo", () => {
  it("jumps to any already-reached step", () => {
    expect(goTo({ step: 3, reached: 3 }, 1)).toEqual({ step: 1, reached: 3 });
    expect(goTo({ step: 1, reached: 3 }, 3)).toEqual({ step: 3, reached: 3 });
  });

  it("ignores jumps to steps not reached yet", () => {
    expect(goTo({ step: 1, reached: 1 }, 3)).toEqual({ step: 1, reached: 1 });
    expect(goTo({ step: 2, reached: 2 }, 3)).toEqual({ step: 2, reached: 2 });
  });
});

describe("restartForNewClientId", () => {
  it("restarts the wizard from step 1 (escape hatch from the skip-to-connect path)", () => {
    expect(restartForNewClientId()).toEqual({ step: 1, reached: 1 });
  });
});
