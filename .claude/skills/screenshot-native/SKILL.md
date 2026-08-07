---
name: screenshot-native
description: Triggers on /screenshot-native only. Screenshots this app's strip, flyouts, menus with nearest-neighbour zoom.
argument-hint: "[region|window|menu]"
disable-model-invocation: true
---

# /screenshot-native
> Screenshot a native window of this app (strip, flyout, context menu), mouse-only, correctly zoomed.

## Why this exists, not `/screenshot`

`/screenshot`'s Playwright flow drives a browser. This app has no browser: the strip, flyouts,
and native right-click menus are real Win32 windows. Its "Native windows" section already states
the output-path rule, the crop-and-zoom recipe, and the incident that made the rule non-optional -
read that section, this skill does not repeat it. This skill adds the piece `/screenshot` doesn't
cover: a reusable script plus the mouse-only interaction rules for driving the app before capture.

## Step 1 - Verify the helper script

The script must exist at `.claude/skills/screenshot-native/capture-native.ps1` (this repo, not
global). If missing, stop and tell the dev to restore it.

## Step 2 - Pick a mode

| Mode | Use for | Needs |
|---|---|---|
| `region` | Strip, flyout, anything with known screen coords | `-X -Y -Width -Height` |
| `window` | A window with a real title/MainWindowHandle | `-ProcessName` |
| `menu` | A native right-click context menu | `-ClickX -ClickY` plus a capture rect |

`region` is the default choice for the strip and flyouts: they are borderless overlay windows, so
`window` mode's `MainWindowHandle` lookup is not guaranteed to resolve for them (it does for the
main app window, not reliably for child/tool windows). If unsure, use `region` with coordinates
read off `[System.Windows.Forms.Screen]::PrimaryScreen.Bounds` or a prior full-screen capture.

## Step 3 - Zoom is mandatory and must be nearest-neighbour

The strip is ~30px tall; a 1x capture is unreadable and bicubic smears a 1px divider into mush.
`-Zoom 4` (default in the script) is the floor for the strip; a settings window or a menu can use
a lower zoom like 2 since its text is already legible at 1x.

## Step 4 - Mouse only, never SendKeys

`SendKeys` targets whatever window currently has keyboard focus, not necessarily this app - a
past session's `{UP}{RIGHT}{ENTER}` meant for a native menu landed in an unrelated app and
navigated it instead. The script drives real cursor position and mouse button events
(`SetCursorPos` + `mouse_event`) instead, which act on whatever is under the cursor regardless of
focus. Never add a `SendKeys` call to this flow.

## Step 5 - Menu captures are one script call, start to finish

A native popup menu is modal and can lose activation and dismiss if the invoking script exits
before the capture happens. `-Mode menu` opens the menu (right-click), optionally clicks a second
point (`-ThenClickX/-ThenClickY`, e.g. a menu item), and captures the result, all inside one
script invocation. To find the menu item's coordinates first, run once with no `-ThenClick*` to
see where the menu rendered, then re-run with the coordinates read off that image, since the
right-click has to happen again anyway to reopen the menu.

## Step 6 - Determine the output path

Follow `/screenshot`'s output-path rule: throwaway verification shots go under
`.for_bepy/screenshots/<claude-ancestor-pid>-<ancestor-start-ticks>/`, never the folder root.

## Step 7 - Run the script

One command, no chaining:

```
powershell -NoProfile -File ".claude/skills/screenshot-native/capture-native.ps1" -Mode region -X 0 -Y 1380 -Width 500 -Height 60 -Zoom 4 -Out ".for_bepy/screenshots/<pid>-<ticks>/strip.png"
```

Menu mode example, one call:

```
powershell -NoProfile -File ".claude/skills/screenshot-native/capture-native.ps1" -Mode menu -ClickX 120 -ClickY 1400 -CaptureX 0 -CaptureY 1200 -CaptureWidth 400 -CaptureHeight 250 -Zoom 2 -Out ".for_bepy/screenshots/<pid>-<ticks>/menu.png"
```

## Step 8 - Read the result back and sanity-check

Read the PNG back. A blank or black region is ambiguous, not automatically a failure: this app's
`hide_from_capture` setting deliberately excludes the strip/flyout from screen capture, and that
is correct behavior, not a crash. Before concluding anything from a blank image, confirm the
process is actually running and check whether capture-exclusion is enabled:

```
Get-Process -Name "windows-taskbar-widgets" -ErrorAction SilentlyContinue
```

Two related traps when checking the process: the exe is named `windows-taskbar-widgets.exe`, not
the product name "Taskbar Widgets", so filtering by product name finds nothing; and the installed
copy at `%LOCALAPPDATA%\Taskbar Widgets\` can silently block a dev build from ever starting
(`tauri_plugin_single_instance` hands off to it and exits), which reads exactly like a crash but
isn't one.

## Step 9 - Report the path

Return the saved PNG path(s). Do not delete captures outside your own session subfolder.
