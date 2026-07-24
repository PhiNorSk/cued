import { describe, expect, it } from "vitest";
import {
  COPY_CONFIRM_MS,
  copyFeedback,
  expireFeedback,
} from "./clipboard";

describe("copyFeedback", () => {
  it("confirms a successful copy", () => {
    expect(copyFeedback(true)).toBe("copied");
  });

  it("reports a failed copy", () => {
    expect(copyFeedback(false)).toBe("failed");
  });
});

describe("expireFeedback", () => {
  it("returns to idle after the confirmation window", () => {
    expect(expireFeedback("copied")).toBe("idle");
    expect(expireFeedback("failed")).toBe("idle");
  });

  it("stays idle when there is nothing to expire", () => {
    expect(expireFeedback("idle")).toBe("idle");
  });
});

describe("COPY_CONFIRM_MS", () => {
  it("shows the confirmation for about two seconds", () => {
    expect(COPY_CONFIRM_MS).toBe(2000);
  });
});
