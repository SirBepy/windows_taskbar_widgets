import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { fmtTime } from "./pomodoro";

// No jsdom here, so CLAUDE.md's getBoundingClientRect check isn't runnable.
// These assert the fixed width by construction instead: content never exceeds
// the 7-char budget, and the shipped rule hard-caps the box.

describe("fmtTime fits the reserved 7ch pomodoro tile budget", () => {
  const cases: [label: string, totalSec: number, expected: string][] = [
    ["00:00", 0, "00:00"],
    ["59:59", 59 * 60 + 59, "59:59"],
    ["1:00:00", 3600, "1:00:00"],
    ["9:59:59", 9 * 3600 + 59 * 60 + 59, "9:59:59"],
  ];

  it.each(cases)("%s -> %s, length <= 7", (_label, totalSec, expected) => {
    const out = fmtTime(totalSec);
    expect(out).toBe(expected);
    expect(out.length).toBeLessThanOrEqual(7);
  });
});

describe("tile.css pins .pomo-tile-time to a hard, non-growing width", () => {
  const cssPath = fileURLToPath(new URL("../styles/tile.css", import.meta.url));
  const css = readFileSync(cssPath, "utf-8");
  const rule = css.match(/\.pomo-tile-time\s*\{[^}]*\}/)?.[0] ?? "";

  it("uses width, not min-width, so shorter content can't shrink or grow the box", () => {
    expect(rule).toMatch(/width:\s*calc\(7ch \+ 6px\)/);
    expect(rule).not.toMatch(/min-width/);
  });

  it("clips instead of overflowing if a glyph estimate ever undershoots", () => {
    expect(rule).toMatch(/overflow:\s*hidden/);
    expect(rule).toMatch(/white-space:\s*nowrap/);
  });

  it("uses tabular-nums so digit swaps within the fixed box don't jitter", () => {
    expect(rule).toMatch(/font-variant-numeric:\s*tabular-nums/);
  });

  it("dropped the orphaned flyout CSS (no flyout is mounted anymore)", () => {
    expect(css).not.toMatch(/\.pomo-tab\b/);
    expect(css).not.toMatch(/\.pomo-btn\b/);
    expect(css).not.toMatch(/\.pomo-actions\b/);
  });
});
