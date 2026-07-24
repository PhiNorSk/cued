import { describe, expect, it } from "vitest";
import { isValidClientId } from "./clientId";

describe("isValidClientId", () => {
  it("accepts a real-looking 32-char hex client id", () => {
    expect(isValidClientId("5f573c9620494bae87890c0f08a60293")).toBe(true);
  });

  it("accepts mixed-case alphanumeric ids between 20 and 40 chars", () => {
    expect(isValidClientId("Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z")).toBe(true);
    expect(isValidClientId("a".repeat(20))).toBe(true);
    expect(isValidClientId("a".repeat(40))).toBe(true);
  });

  it("rejects too-short ids", () => {
    expect(isValidClientId("abc123")).toBe(false);
    expect(isValidClientId("")).toBe(false);
  });

  it("rejects too-long ids", () => {
    expect(isValidClientId("a".repeat(41))).toBe(false);
  });

  it("rejects illegal characters", () => {
    expect(isValidClientId("5f573c96-2049-4bae-8789-0c0f08a602")).toBe(false);
    expect(isValidClientId("5f573c9620494bae87890c0f08a6029 ")).toBe(false);
    expect(isValidClientId("5f573c9620494bae87890c0f08a6029ä")).toBe(false);
  });
});
