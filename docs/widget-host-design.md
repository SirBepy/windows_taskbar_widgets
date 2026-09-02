# Widget host design: taskbar strip or floating overlay

Status: **approved; steps 1-3 and 5 shipped, step 4 is host-side only.** Written 2026-08-07 for
`.claude/todos/07-widgets-rename-and-overlay-placement.md`, then revised once against two
adversarial review passes (architectural correctness verified against the source, and product
coherence verified against Joe's stated intent). Signed off 2026-09-02, when the two remaining
questions (conductor's scope, and whether providers carry their own setting) were both closed.

Step 4, provider suppression, is the only half still open: this repo sends the message, and no
provider reads it yet. See "What ships in what order".

## Goal

Today this app is one Tauri window pinned over the taskbar's empty left side, hosting a row of
tiles. The goal is a general **widget host**: every widget is placed either in the taskbar strip
(as now) or as a free-floating overlay somewhere on screen, and provider apps that currently draw
their own overlays stop doing so while their widget is hosted here.

Joe, 2026-08-03:

> the idea is that in the future we would rename this app into widgets, and it would let you choose
> if you want a widget to be in the taskbar or somewhere on the screen... so, realistically, while
> this app is on, no other overlay shows up (if they support having overlays in this app). and then
> in this app you can configure how you want the overlay to look

## The decisions

The first five were parked as open questions on the todo. The sixth covers "configure how you want
the overlay to look", which the todo's Approach listed but its later checklist dropped.

### 1. What does a floating overlay render? A third mount target, no measured fallback.

`TaskbarWidget` (`src/shared/widget.ts:14`) gains an optional third render target:

```ts
overlay?: { widthCss: number; heightCss: number };
mountOverlay?(root: HTMLElement): () => void;
```

Same contract as the existing two: given a fresh root element, mount and return a cleanup closure.

Rejected: reusing `mountTile` at a larger size (the tile is authored for a 48px strip, its layout is
horizontal and cramped by design) and reusing `mountFlyout` (the flyout is a transient hover panel
whose content assumes it is dismissed in seconds, and its dims are chosen for a popup anchored to
the taskbar). Overloading either fights CLAUDE.md's fixed-size rule the moment an overlay needs
different dimensions.

**Renderer falls back, size does not.** The renderer resolves as
`mountOverlay ?? mountFlyout ?? mountTile`, so all six current widgets are overlay-placeable without
being rewritten. The **size** resolves as `overlay ?? flyout`, full stop. A widget declaring neither
is simply not offered as overlay-placeable in the UI.

Since overlays are freely resizable (decision 6), the declared `overlay` dims are the **default and
the minimum**, not a fixed footprint. Measuring a mounted tile to derive that default was considered
and **rejected**: a measurement taken before a content-variable widget (a process list, a drive
list) fills up would produce a nonsense minimum. `pomodoro` has no `flyout` today, so it needs one
line declaring `overlay` dims before it can be placed as an overlay.

### 2. Placement model: drag-to-place, persisted as monitor plus x/y, with edge snapping.

An overlay is dragged anywhere on any monitor. Its position persists as a `device_name` plus CSS
coordinates relative to that monitor's work area, never as raw desktop coordinates. This is the same
approach `taskbar_monitor` already takes (`src-tauri/src/taskbar/monitors.rs:18`), and it is what
survives a monitor being unplugged, a resolution change, or the displays being rearranged. If the
saved `device_name` is gone at load time, the overlay falls back to the primary monitor at the same
relative offset, clamped into the work area.

While dragging, the overlay snaps to monitor edges and corners within a 12px threshold. Snapping is
assistance, not a constraint: the stored value is still free x/y.

**Recovery is part of the model, not an afterthought.** The drag handler clamps live so at least
48px of the overlay stays inside the work area, which makes an overlay physically undraggable off
screen. The placement UI additionally carries a per-overlay "Reset position" that re-centres it on
the primary monitor, for the case where a stored position predates a display change.

Rejected: snap-target-only placement (too rigid for a "put it where I want it" feature) and raw
desktop coordinates (breaks on every display change).

### 3. Provider suppression: an outbound message on the existing bridges.

No new transport. Both bridges already have a channel back to the provider, though they are at very
different levels of readiness:

- **pomodoro** (`src-tauri/src/bridge_pomodoro.rs`): this app is the TCP client, and there is
  already a writer thread (`bridge_pomodoro.rs:82`) fed by the `mpsc::Sender<String>` held in
  `PomodoroBridgeState` (`bridge_pomodoro.rs:38`), used today by `pomodoro_cmd`
  (`bridge_pomodoro.rs:108`) to send `{"cmd":...}` lines. `pomodoro_cmd`'s `(action, phase)`
  signature cannot carry the new payload, so this needs a sibling command that builds
  `json!({"cmd":"host_overlay","hosted":hosted})` and pushes it through the same sender, plus a
  send at the connect site right after `set_writer(app, Some(tx))` (`bridge_pomodoro.rs:81`).
  **pomodoro-overlay is the only provider that actually draws its own overlay, so it is the whole
  feature.**
- **conductor** (`src-tauri/src/bridge_conductor.rs`): the WS write half is dropped unused at line
  59, and the alternative outbound path (HTTP POST to `/api/rpc`) is gated by a `SAFE_METHODS`
  allowlist that lives in conductor's repo, not this one (verified: no such symbol exists here).
  **Nothing is sent to conductor, settled 2026-09-02 and not on readiness grounds.** Conductor's two
  surfaces are not duplicates of what this app draws: its tray icon is the app icon plus status dots
  (`claude_usage_in_taskbar/src-tauri/src/tray/icon_render.rs:82`), and its `session-overlay` window
  only exists while the user holds it open from a tray left-click, destroyed on toggle
  (`ipc/overlay_window.rs:143`). Neither is on screen unbidden, so neither is "the same thing
  twice". Suppressing the tray icon would also take away conductor's only handle for settings and
  quit. An earlier draft said conductor "does not draw an overlay"; that was wrong, and the ruling
  survives the correction on its own reasoning.

Protocol, sent on connect and again whenever the relevant settings change:

```json
{"cmd":"host_overlay","suppress_compact":true,"suppress_fullscreen":false}
```

Revised 2026-08-08 from an earlier `{"hosted":bool}` shape. Joe's refinement:

> i wish the overlay showed when it was fullscreen, but when it was just the lil overlay, i dont
> wanna see it... i wonder if theres a way to set that up in Pomodoro overlay, that it detects if
> the widgets app is running, and if it is, then hide the lil overlays, but still choose to show
> big overlays

So suppression is selective, not total: a plain `hosted` boolean cannot express "hide the small one,
keep the big one", hence two explicit fields instead of one. `suppress_fullscreen` is always `false`
today: this app has no fullscreen render target of its own, so it never has standing to suppress
pomodoro-overlay's fullscreen view. The field still ships (not omitted) so the receiver never has to
assume an absent key means "unaffected" versus "suppress everything" - the wire format states both
facts explicitly every time.

Sending it on connect makes the state self-correcting: the provider never has to remember anything
across restarts, and a provider that does not understand `host_overlay` ignores an unknown `cmd` and
keeps its current behaviour, so this change is backwards compatible in both directions.

**`suppress_compact` tracks presence in this app, not overlay placement specifically.** Settled by
Joe on 2026-08-07: the moment a provider's widget is enabled here at all, taskbar tile included, the
provider suppresses its own compact overlay. This is the literal reading of "while this app is on,
no other overlay shows up", and it is the simpler rule to explain: one app owns the compact surface,
full stop.

So the trigger is `enabled_widgets` membership minus `hidden_widgets` (`Settings::is_active`), not
`widget_placement`. The message resends on every `save_settings` call (unconditional, same pattern
as `overlay::reconcile`), and once more on every bridge (re)connect.

### Receiving contract for pomodoro-overlay

This repo's half (shipped 2026-08-08, `src-tauri/src/bridge_pomodoro.rs`): the pomodoro TCP client
pushes the line above through the existing writer channel (`PomodoroBridgeState`'s
`mpsc::Sender<String>`, the same one `pomodoro_cmd` already uses) at two moments:

1. **On connect**, right after the writer is installed (`bridge_pomodoro.rs`'s `try_connect`,
   immediately after `set_writer(app, Some(tx))`), computed fresh from current settings.
2. **On every settings save** (`save_settings` in `lib.rs`), unconditionally - covers pomodoro
   being enabled/disabled and any placement change, without needing to diff old vs new.

Nothing is sent while disconnected; `send_host_overlay` is a no-op if the writer is `None`, and the
next connect resends the current state anyway.

What pomodoro-overlay (the receiver, its own repo, out of scope here) should do with each field:

- **`suppress_compact: true`** - hide its own small/compact overlay. It was already open: close it.
  It was already closed: stay closed. No animation requirement, this is a state, not an event.
- **`suppress_compact: false`** - show its own small/compact overlay again (the widget is no longer
  hosted here, e.g. the user disabled or removed it in Widgets).
- **`suppress_fullscreen: false`** (the only value this app ever sends) - never hide the fullscreen
  overlay in response to this message. Whatever independently controls when pomodoro-overlay's
  fullscreen view shows (a focus session starting, etc.) is untouched by `host_overlay` entirely.
- **Unknown fields or an unrecognised `cmd`** - ignore and keep current behaviour, per the
  backwards-compatible design above.
- **Connection drops** (this app quits, crashes, or is killed - the TCP client, so the socket close
  is visible to pomodoro-overlay as the server) - restore its own compact overlay immediately, same
  as if it had just received `suppress_compact: false`. This is decision 4's disconnect-driven
  restore: one mechanism covers clean quit, crash, and kill alike, and there is no state to leak.
  **This is the part that matters most**: skipping it leaves the user with no overlay at all, small
  or big, the moment Widgets closes.

The provider-side change lands in the `pomodoro-overlay` repo and is out of scope for this one.

#### Correction 2026-09-02: pomodoro has one window, not two

The two-field protocol reads as "hide surface A, leave surface B alone", but pomodoro-overlay has
no surface B. `src-tauri/tauri.conf.json` declares a **single** window, label `main`, 280x80,
`visible: false`. `set_window_fullscreen(true)` (`src-tauri/src/ipc/commands.rs:246`) resizes that
same window to the monitor work area and shows it with `SW_SHOWNOACTIVATE`. Compact and fullscreen
are two **modes of one window**.

So `suppress_compact` is a mode gate, not a window gate. The rule the receiver implements:

- While suppressed, `main` stays hidden **in compact mode**.
- A fullscreen trigger still shows it fullscreen, because `suppress_fullscreen` is false.
- When fullscreen exits (`exitOverlayFullscreen`, `src/shared/fullscreen.ts:41`), it returns to
  hidden instead of back to the corner. That exit path is the one site that must consult the
  suppression flag rather than unconditionally restoring the corner window.

This is what Joe asked for on 2026-09-02, confirming his 2026-08-08 quote: "on pomodoro, id still
want to have the option of showing the fullscreen break timer, i think thats helpful". The option
itself is already pomodoro's own (`fullscreen_on_focus_end`, `meeting_break_fullscreen`,
`src-tauri/src/settings.rs:30,47`) and `host_overlay` never touches it.

**Blocker on the receiving side:** `dispatch_command` (`src-tauri/src/bridge.rs:150`) flattens every
inbound line to `json!({"action": cmd, "phase": phase})` before emitting `bridge-command`, so
`suppress_compact` and `suppress_fullscreen` are dropped before the frontend ever sees them. The
provider half needs that function widened, not just a new frontend branch. The upside of the same
finding: `host_overlay` currently arrives as an unrecognised `action` and is ignored, so the
backwards-compatibility claim above holds in practice today, verified not assumed.

### 4. When Widgets closes, provider overlays come back immediately, driven by disconnect.

The provider restores its own overlay when the bridge connection **drops**, not on receipt of an
explicit "unsuppress". This app is the TCP client and pomodoro-overlay is the server, so the server
sees the socket close whether this app quit cleanly, crashed, or was killed. One mechanism covers
all three, and there is no state to leak.

A graceful placement change while both apps keep running is covered by the explicit
`{"hosted":false}` message from decision 3.

Explicitly NOT sticky: the suppression never outlives the connection.

### 5. The rename keeps the identifier.

Product name, window titles, tray tooltip and README become **Widgets**. The bundle identifier stays
`com.sirbepy.taskbar-widgets`.

The identifier is what `resolve_path` (`settings.rs:69`) turns into
`%APPDATA%\<identifier>\settings.json`. Changing it silently orphans every existing install's
settings, and the identifier is invisible to users. Renaming it would be pure cosmetics paid for
with a migration that can lose data. The repo directory name and the crate name are equally
invisible and equally not worth churning.

### 6. Per-overlay appearance: opacity override and free resize.

Joe asked to "configure how you want the overlay to look". Two knobs, both stored on the placement
record itself rather than in a parallel store:

- **Opacity override**, `null` meaning "use the global `opacity` setting". The global value and its
  hover boost already apply to the strip and the flyout via `applyOpacity`
  (`src/shared/widget.ts:44`); `src/overlay-main.ts` must call it too, and the override simply
  supplies a different number to the same CSS variable.
- **Free resize by drag handle**, persisting width and height on the placement record. Chosen by
  Joe on 2026-08-07 over a fixed set of scale steps.

Theme is deliberately NOT per overlay: it follows the app theme the kit already owns, so overlays
match the strip and the settings screen without a third source of truth.

**Free resize costs real work, and this is where it lands.** Three consequences, all of which are
part of step 3 in the shipping order:

1. **Every overlay renderer must reflow.** Today a widget is authored at exactly one size. An
   arbitrary user-chosen size means each `mountOverlay` has to lay out fluidly, which is per-widget
   work, not a host-level feature. The conductor dial and the process lists are the expensive ones.
2. **Bounds are enforced by the host.** Minimum is the widget's declared `overlay` dims, so a
   widget can never be dragged smaller than it was authored to survive. Maximum is the monitor work
   area.
3. **CLAUDE.md's fixed-size rule needs a sanctioned exception before this ships.** The rule bans a
   mounted widget changing size, with a carve-out for a one-time config toggle. A live resize handle
   is neither: it is a continuous deliberate user action. The invariant that must survive is the one
   the rule actually protects, namely that **content never drives a size change**. Only the user's
   handle may, and content reflows into whatever size the user chose. Add this to CLAUDE.md's
   "Sanctioned exceptions" section as part of step 3, not before.

## Architecture

### One native window per placed overlay

Overlay windows are created at runtime with `WebviewWindowBuilder`, one per widget placed as an
overlay. This is a departure from the current shape, where all three windows (`strip`, `flyout`,
`settings`) are declared up front in `tauri.conf.json` and merely shown or hidden.

Each overlay window is transparent, undecorated, `skipTaskbar`, `alwaysOnTop`, sized from the
declared dims of decision 1 times the scale of decision 6, and positioned from the resolved
placement of decision 2. It loads `overlay.html`, a fourth entry point alongside the existing three,
whose bootstrap (`src/overlay-main.ts`) reads its own widget id from the window label, applies
opacity, and mounts exactly one widget.

Adding that entry point is not free: `src/overlay.html` has to be created and registered in
`vite.config.ts`'s `rollupOptions.input`, which currently lists exactly three entries
(`vite.config.ts:32`). No `tauri.conf.json` change is needed, since the window is built at runtime.

Rejected: a single full-screen transparent window hosting all overlays. It reads as simpler until
hit-testing arrives, at which point every non-widget pixel has to be made click-through, and it
fits one monitor only. Per-widget windows get correct hit-testing, per-monitor placement, and
independent z-order for free from the OS.

**Window label convention: `overlay-<sanitized id>`, no colons.** Tauri documents window labels as
alphanumeric (`WindowConfig::label`), and divider ids already contain a colon
(`DIVIDER_PREFIX`, `settings.rs:80`), so a naive `overlay:<id>` yields `overlay:divider:abc`. The
sanitizer replaces every non-alphanumeric character with `-`, and the reconciler keeps the mapping
back to the real widget id rather than parsing the label.

### Capture exclusion and window enumeration must stop being hardcoded

`apply_capture_exclusion` iterates a literal `["strip", "flyout"]` (`src-tauri/src/lib.rs:79`). A
user with `hide_from_capture` on would still leak every overlay into a screen recording. The
reconciler applies exclusion to each overlay window as it creates one, and that hardcoded list
becomes a dynamic enumeration. This is a correctness requirement, not a nicety, and it is in
Acceptance below.

`autohide.rs` looks windows up by the literal `"strip"` label (`autohide.rs:50`) and never
enumerates, so it needs no change: overlays sit outside the taskbar band and outside its reach.

### Placement lives in settings, not in a new store

`Settings` (`src-tauri/src/settings.rs:10`) gains one field:

```rust
pub widget_placement: HashMap<String, Placement>,

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement {
    Strip,
    Overlay {
        monitor: String,          // device_name, "" = primary
        x: f64,
        y: f64,
        #[serde(default)]
        w: Option<f64>,           // None = the widget's declared overlay width
        #[serde(default)]
        h: Option<f64>,
        #[serde(default)]
        opacity: Option<u32>,     // None = inherit the global setting
    },
}
```

The explicit `#[serde(tag = ...)]` matters. Serde's default external tagging would write
`{"Overlay":{...}}`, a shape nothing else in this settings file uses; internal tagging gives
`{"kind":"overlay","monitor":"","x":12,"y":40}`, which reads like the rest of the file and is what
the TS side will expect.

Absent key means `Strip`, so `#[serde(default)]` on the struct handles the migration by itself:
every existing install keeps every widget in the strip, and `enabled_widgets` ordering is untouched.
Two companion edits are required and easy to forget: `impl Default for Settings`
(`settings.rs:38`) enumerates every field with no `..Default::default()` spread, so it needs
`widget_placement: HashMap::new()`; and the TS mirror `Settings` in `src/shared/widget.ts:31` needs
the matching field and a `Placement` union, since it is hand-maintained.

`enabled_widgets` and `hidden_widgets` keep their current meaning. Placement is orthogonal, with one
precedence rule: **hidden wins.** A widget that is hidden gets no overlay window regardless of its
placement, so hiding stays the single "make it go away" action it is today. A widget placed as an
overlay is skipped by the strip's render loop in `src/main.ts` and gets a window instead.

### Change propagation

Reuse the existing `"widgets-changed"` broadcast (`lib.rs:38`). On receiving it, a Rust-side
reconciler diffs the desired overlay set against the live one and creates, moves, resizes or closes
windows to match. One code path handles startup, a settings change, and a drag-to-place, which
avoids the class of bug where the drag path and the settings path drift apart.

### Interaction on an overlay

- **Drag** to move: pointer-down on the overlay background calls Tauri's `start_dragging()`, then
  the window's `moved` event writes the clamped, snapped placement back to settings, debounced.
  Widgets that already own pointer events, notably `conductor` with its `isDragging()` guard from
  `src/shared/widget.ts`, need that guard extended to the overlay case so a click on a dial is not
  swallowed by a window drag.
- **Right-click** opens the same native menu the tile has (`tile_menu.rs`), minus the strip-only
  entries (move left, move right, remove divider), plus **Move to taskbar**. The widget's own
  `menuItems()` entries are unchanged, so "Open Task Manager" and friends work identically.
- **Hover does NOT open a flyout.** The overlay is already the roomy view; layering a transient
  panel on top of a persistent one is noise, and it would put a second fixed-size surface in play
  for no benefit. The flyout stays a strip-only affordance.

## What ships in what order

1. **Contract and settings.** `overlay`/`mountOverlay` on `TaskbarWidget`, the renderer fallback
   chain, `Placement` plus `widget_placement` in `Settings` and its TS mirror. No UI. Verifiable by
   unit test, and it changes nothing a user can see.
2. **Overlay windows (developer checkpoint, not a user-facing release).** `src/overlay.html`,
   `src/overlay-main.ts`, the vite entry, the runtime window builder, the reconciler, capture
   exclusion. Placement is settable only by hand-editing `settings.json` at this point, so this
   proves the mechanism works but ships nothing usable. Do not cut a release here.
3. **Placement UI.** A placement control per widget in the settings screen's existing per-widget
   accordion (`src/views/settings/widget-strip-field.ts`), the appearance knobs from decision 6,
   drag-to-place, snapping, reset-position. **This is the first user-visible release.**
4. **Provider suppression.** The `host_overlay` message on the pomodoro bridge - **this repo's half
   shipped 2026-08-08**, see decision 3's receiving contract - plus the matching change in the
   `pomodoro-overlay` repo, which is separate work in that repo. **Still open as of 2026-09-02:**
   `host_overlay` has zero occurrences in the `pomodoro-overlay` tree, so this app has been sending
   the message to a receiver that ignores it. Nothing in this repo is missing; the remaining work is
   entirely in `pomodoro-overlay`, tracked in that repo's own backlog. Conductor is out of scope by
   decision 3's 2026-09-02 ruling, so pomodoro is the whole of step 4.
5. **The rename.** Shipped 2026-08-08. Product name and identifier kept exactly as-is (see decision
   5's note on the todo's later, more specific ruling); only user-visible strings changed.

## Acceptance

Carried from the todo, plus what this design adds:

- Nothing regresses in the strip: left-anchored positioning, hover-close reliability (`134f842`
  native cursor polling in `src-tauri/src/flyout.rs`), drag-to-reorder.
- Existing installs open with every widget still in the strip and settings intact, with no
  migration code beyond `#[serde(default)]`.
- An overlay never resizes on hover or on content change. Only the user's resize handle changes its
  size, and content reflows into whatever size was chosen. See decision 6 point 3.
- `hide_from_capture` excludes overlay windows too, verified by a screen capture with an overlay
  placed.
- The global opacity setting and its hover boost apply to an overlay that has no override.
- An overlay stays on its assigned monitor across a resolution change, falls back to primary when
  that monitor is unplugged, and cannot be dragged fully off screen.
- A hidden widget spawns no overlay window.
- Killing Widgets brings pomodoro-overlay's own overlay back without user action.

## Open questions for sign-off

- ~~Does `hosted:true` fire for any placement, or only overlay placement?~~ **Settled 2026-08-07:
  any placement in this app.** See decision 3.
- ~~Scale steps or free resize?~~ **Settled 2026-08-07: free resize.** See decision 6.
- ~~Does conductor need suppressing too?~~ **Settled 2026-09-02: nothing is sent to conductor.**
  See decision 3's conductor bullet for the reasoning, which is about its surfaces not duplicating
  anything, not about the write channel being unbuilt.
- ~~Should the provider app carry its own setting for this, or only the host?~~ **Settled
  2026-09-02: host-only, the 2026-08-07 ruling stands.** No opt-out in pomodoro or conductor. The
  provider stays a pure receiver: obey the wire message, restore on disconnect. Only one place can
  be wrong about suppression, and it is the place the user is already looking when configuring
  widgets. A provider-side override would create the state where a widget is disabled in Widgets and
  still nothing shows, with the cause in the other app's settings.

All design questions are now closed, and Joe approved the document as a whole on 2026-09-02.

## Open risks

- ~~`WebviewWindowBuilder` at runtime is unexercised here.~~ **Retired 2026-08-07 by a live run.**
  Runtime-built windows, transparency and always-on-top all work. The spike found one real trap:
  `src-tauri/capabilities/default.json` lists window labels explicitly, and a runtime window not
  matching that list is denied `event.listen`, `start_dragging` and `start_resize_dragging` with no
  visible failure beyond a JS rejection. The fix is the `overlay-*` glob in that list. Any future
  runtime-created window has the same trap.
- **N webviews is N WebView2 processes.** Six overlays is a real memory cost. Worth measuring at
  step 2, before the placement UI invites users to place everything.
