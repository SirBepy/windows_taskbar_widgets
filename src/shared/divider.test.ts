import { describe, expect, it } from "vitest";
import { isDividerId, makeDividerId } from "./divider";

describe("divider id scheme", () => {
  it("mints ids that isDividerId recognizes", () => {
    const id = makeDividerId();
    expect(isDividerId(id)).toBe(true);
    expect(id.startsWith("divider:")).toBe(true);
  });

  it("never collides across many instances (crypto.randomUUID's collision space)", () => {
    const ids = new Set(Array.from({ length: 500 }, () => makeDividerId()));
    expect(ids.size).toBe(500);
  });

  it("does not mistake a real widget id, or one that merely contains the word, for a divider", () => {
    expect(isDividerId("cpu")).toBe(false);
    expect(isDividerId("my-divider-widget")).toBe(false);
  });
});
