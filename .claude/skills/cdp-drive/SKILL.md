---
name: cdp-drive
description: Triggers on /cdp-drive. Brings this app's dev build up with WebView2 remote debugging and drives or measures its real webviews (strip, flyout, settings, overlay) over CDP.
argument-hint: "[targets|strips|settings|shot <dir>|eval <page> <js>]"
---

# /cdp-drive

> Attach to this app's own WebView2 processes and read or drive the real DOM, instead of
> automating Windows.

## Why this exists

This is the highest-leverage verification technique the project has. It produced the
`getBoundingClientRect` measurement that closed todo 56, diagnosed and then confirmed the fix
for todo 46 after four prior sessions failed on it, and drove the phase-4 lane drag that
verified todo 50. It reaches the real DOM in the real app with no focus stealing.

It supersedes the `Cursor.Position` hover simulation in the `run-and-debug-mechanics` project
memory, which was measured unreliable on 2026-08-09 across ~6 attempts.

`/screenshot` and `/flutter-e2e` cover browser automation generally. This is specifically
about attaching to a Tauri app's own WebView2 processes, which is why it lives in this repo.

## Step 1 - Close the installed app first, and say so

`tauri_plugin_single_instance` makes `npm run tauri dev` compile, launch, hand off to the
already-running installed instance, and exit. The supervisor entry then flips to `stopped` with
no error line, which reads exactly like a crash.

The process name is `windows-taskbar-widgets.exe`, NOT the productName "Taskbar Widgets" - a
filter on the product name matches nothing and looks like the app never started.

Closing Joe's running app is his call, not yours: ask before doing it. Back up `settings.json`
(`%APPDATA%\com.sirbepy.taskbar-widgets\settings.json`) before anything that writes it, and
relaunch `%LOCALAPPDATA%\Taskbar Widgets\windows-taskbar-widgets.exe` when the sitting ends.

## Step 2 - Launch

Run `.claude/skills/cdp-drive/dev-with-cdp.cmd` through `/supervised-run`. The env var must be
set by the `.cmd` file: `sv.ps1 -Cmd` mis-tokenises an inline `cmd /c "set X=Y&& ..."` string.

Readiness is `http://127.0.0.1:9333/json/version` answering, polled - NOT a `sv.ps1 logs` grep
for a "Running" marker, which fails because cargo's output is ANSI-coloured and the escape codes
break a naive pattern.

A cold `cargo tauri dev` can take an hour: `target-dir` is a shared `D:/cargo-target` across
every Rust project on the machine, so this build queues behind any other one. Check
`Get-Process cargo,rustc` for rising CPU time before calling anything hung, and never kill
another project's build to jump the queue.

## Step 3 - Drive

```
node .claude/skills/cdp-drive/drive.mjs <command>
```

| Command | Does |
|---|---|
| `targets` | Every CDP target, with a hung-build warning (see below) |
| `strips` | Per live strip window: its tiles' widget + instance ids, and the row width |
| `settings` | Opens Settings > Widgets, dumps lane heads, per-lane tiles, palette, config rows |
| `shot <out-dir>` | Screenshots the settings page and the lanes block |
| `click <selector>` | A real pointer click in Settings > Widgets, with a raw-mouse fallback |
| `select <selector> <value>` | Sets a dropdown, e.g. the Placement select, and reports the config rows after |
| `drag <widget> <lane>` | Drags a preview tile into lane N (0-based), then reports every lane |
| `eval <page> <js>` | Runs an expression in a page: `strip`, `settings`, `flyout`, `overlay`, or a URL substring |

Playwright is resolved by `~/.claude/skills/_shared/playwright-resolve.cjs`; this repo has none
of its own and needs none.

## The traps this skill exists to stop you re-hitting

- **The strip page must be matched with `/localhost:3102\/?$/`.** A plain
  `.includes("localhost:3102/")` also matches the flyout, settings and overlay pages. Cost two
  round trips in one session, twice. `drive.mjs` already anchors it.
- **`settings.html` opens on a menu, not the widgets editor.** Nothing matching `.wsf-*` exists
  until `[data-nav="section-widgets"]` is clicked. The nav rows are plain divs with `data-nav`
  attributes (`section-widgets`, `section-host`, `system`, `about`) - click the attribute, never
  the label text.
- **A page target sitting at `about:blank` is a hung window build**, the signature of
  `WebviewWindowBuilder::build()` dispatched into one of the event loop's own dispatches instead
  of queued for `RunEvent::MainEventsCleared` (todos 46 and 61). `targets` flags it explicitly.
  If you see it, the window shell exists and its webview never loaded - do not read it as a
  rendering bug.
- **The settings window does not have to be visible.** `page.screenshot()` and a full
  `page.mouse` drag both work against a hidden window, which is how the phase-4 lane drag was
  verified without touching Joe's desktop.
- **Preview tiles listen to `pointerdown` for drag**, so a DOM `.click()` does nothing. Use a
  real Playwright click, and a real `mouse.down`/`move`/`up` sequence for a drag.
- **A kit toggle's real `input` is 0x0 and `opacity: 0`** - the clickable surface is
  `.kit-toggle-track`. Playwright also refuses any element inside a `label` whose first labelable
  control is disabled, reporting "element is not enabled" about a plain `span`. `click` falls back
  to a raw `mouse.click` at the element's centre for exactly that case; if the fallback fires and
  nothing happens, the click is reaching the wrong control and that is an app bug, not a driver
  one (it was, on 2026-09-01 - see commit `308abd0`).

## Output paths

Screenshots go to `.for_bepy/screenshots/<id>/`, where `<id>` is
`~/.claude/skills/close/rename-session.ps1 -GetId`. Gitignored and disposable.
