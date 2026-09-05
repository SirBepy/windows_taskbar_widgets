---
name: window-probe
description: Triggers on /window-probe only. Native Win32 top-level window introspection across the whole desktop - rects, ex-styles, hidden windows, what a click actually hits.
argument-hint: "[-Pid <id> | -At x,y | -Region l,t,r,b | -Topmost | -WatchClicks <secs>]"
disable-model-invocation: true
---

# /window-probe

> What windows exist right now, where they sit, and what a click at a point actually lands on.

## Why this exists, not `/cdp-drive`

`/cdp-drive` talks to this app's own webview PAGES over the DevTools protocol: DOM, layout,
screenshots of rendered content. It cannot see a window as Windows sees it, cannot see another
process, and cannot see a window that is hidden.

This skill is the other half: native `EnumWindows` introspection over every top-level window on the
desktop, hidden ones included. It exists because the 2026-09-03 `window_park.rs` session hand-wrote
five separate `Add-Type -MemberDefinition` blocks in one sitting, each re-declaring the same
P/Invoke set, and two of them failed on PowerShell 5.1 parsing before they ran.

## The script

`.claude/skills/window-probe/window-probe.ps1` (this repo, not global). One shared P/Invoke block,
five modes. If it is missing, stop and tell the dev to restore it - do not hand-roll `Add-Type`
again, that is the whole thing this replaces.

Always invoke with `powershell -NoProfile -File`:

```
powershell -NoProfile -File ".claude\skills\window-probe\window-probe.ps1" -Topmost
```

## Modes

| Mode | Shows | Example |
|---|---|---|
| `-Pid <id>` | Every window of a process AND its WebView2 child processes, hidden included | `-Pid 43740` |
| `-At x,y` | `WindowFromPoint`'s answer, then every window whose rect contains the point, front to back, hidden included | `-At 500,1400` |
| `-Region l,t,r,b` | Every VISIBLE window overlapping a rect | `-Region 0,1392,468,1440` |
| `-Topmost` | Every visible `WS_EX_TOPMOST` window, desktop-wide | `-Topmost` |
| `-WatchClicks <secs>` | One line per left-click: cursor position and the window it hit | `-WatchClicks 20` |

Coordinates are passed as ONE comma-separated string, not an array. `powershell -File` hands
`500,1400` to a single parameter, so the script parses it itself; `-At "500,1400"` and
`-At 500,1400` both work, in-session and from `-File` alike.

Every mode prints `ExStyle` as raw hex plus a decoded `Flags` column (`TOPMOST`, `LAYERED`,
`NOACTIVATE`) - that is the pair the parked-window work actually needed.

## What this probe CANNOT see

**A clean result never proves a region is clickable. Only a real click does.**

A Tauri window that has been `hide()`n still swallows every mouse click inside its rect, because the
WebView2 composition layer keeps claiming that region. `WindowFromPoint` reports the app UNDERNEATH
it, so `-At` names an innocent window and the region still eats clicks. This is the exact trap that
cost the 2026-09-03 session a full diagnostic round, and it is why `-At` prints a warning line above
its results. See "A hidden window is not an absent window" in the repo's `CLAUDE.md`, and
`src-tauri/src/window_park.rs` for the fix.

The corollary is what `-Pid` is for: it lists hidden windows with their rects, so a window sitting
at its build position instead of parked at `-32000,-32000` is visible in the table even though no
click probe would ever have found it.

## Verifying a parked window

The reference check, reproducing the 2026-09-03 verification:

```
powershell -NoProfile -File ".claude\skills\window-probe\window-probe.ps1" -Pid <widgets-pid>
```

Every window this app hides rather than destroys must read `Visible=False` at `-32000,-32000`. A
hidden window sitting anywhere else is a `window_park` gap.

## PowerShell 5.1 traps this script already avoids

Both bit the session that motivated this skill, and both cost a round trip to diagnose. Keep them
out of any edit to the script:

- **Never inline `-f` inside an `.Add(...)` call.** `$list.Add("{0} {1}" -f $a, $b)` binds the
  commas as METHOD arguments, not format arguments. Build the string into a variable first, then
  `.Add($line)`.
- **Never put `try {} catch {}` inside a hash literal.** It is a parse error, not a runtime one, so
  nothing in the file runs. Compute the value above the literal.

## Safety

Read-only. It calls no `SetWindowPos`, no `SetForegroundWindow`, no `SendInput` - it never moves,
focuses or clicks anything. `-WatchClicks` observes `GetAsyncKeyState` passively and drives nothing,
so it is safe to run while the dev is using the machine.
