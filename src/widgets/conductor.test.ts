import { describe, expect, it } from "vitest";
import { accountCard, miniDial, mergeLive } from "./conductor";

// No jsdom here: flatten a lit TemplateResult (strings interleaved with values,
// recursing into nested results and arrays) into markup to assert on.
function flatten(v: unknown): string {
  if (Array.isArray(v)) return v.map(flatten).join("");
  if (v && typeof v === "object" && "strings" in v && "values" in v) {
    const t = v as { strings: readonly string[]; values: unknown[] };
    return t.strings.map((s, i) => s + (i < t.values.length ? flatten(t.values[i]) : "")).join("");
  }
  return v === null || v === undefined ? "" : String(v);
}

const account = (over: Partial<Parameters<typeof miniDial>[0]> = {}) => ({
  id: "a",
  label: "personal",
  colour: "#a855f7",
  icon: "user",
  captured_at: "2026-11-09T10:00:00Z",
  five_hour_pct: 10,
  five_hour_resets_at: null,
  five_hour_limited: true,
  seven_day_pct: 6,
  seven_day_resets_at: null,
  ...over,
});

// Concentric tile rings: outer r=19, inner r=12 (OUTER_R / INNER_R).
const ringRadii = (markup: string) => [...markup.matchAll(/<circle[^>]* r="(\d+)"/g)].map((m) => m[1]);

describe("tile dial", () => {
  it("draws two concentric rings for an account with a 5h limit", () => {
    const radii = new Set(ringRadii(flatten(miniDial(account()))));
    expect(radii).toEqual(new Set(["19", "12"]));
  });

  it("draws one outer ring, the weekly one, for an account with no 5h limit", () => {
    const markup = flatten(miniDial(account({ five_hour_limited: false })));
    expect(new Set(ringRadii(markup))).toEqual(new Set(["19"]));
    expect(markup).toContain("personal: 7d 6%");
    expect(markup).not.toContain("5h");
  });
});

describe("flyout card", () => {
  it("shows both ring columns for an account with a 5h limit", () => {
    expect(flatten(accountCard(account())).match(/class="ring-col"/g)).toHaveLength(2);
  });

  it("shows only the weekly ring column for an account with no 5h limit", () => {
    const markup = flatten(accountCard(account({ five_hour_limited: false })));
    expect(markup.match(/class="ring-col"/g)).toHaveLength(1);
    expect(markup).toContain("Weekly (7d)");
    expect(markup).not.toContain("Session (5h)");
  });
});

describe("mergeLive", () => {
  const snap = (five_hour_limited?: boolean) => ({
    captured_at: "2026-11-09T11:00:00Z",
    five_hour: { utilization: 0, resets_at: null },
    ...(five_hour_limited === undefined ? {} : { five_hour_limited }),
    seven_day: { utilization: 7, resets_at: null },
    account_id: "a",
  });
  const base = { available: true, accounts: [account()] };

  it("carries a live no-5h-limit flag onto the account", () => {
    expect(mergeLive(base, snap(false))?.accounts[0].five_hour_limited).toBe(false);
  });

  it("reads a push without the flag as limited", () => {
    const off = mergeLive(base, snap(false));
    expect(mergeLive(off, snap())?.accounts[0].five_hour_limited).toBe(true);
  });
});
