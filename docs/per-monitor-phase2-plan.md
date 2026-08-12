# Per-monitor placement, Phase 2: getting off the hardcoded `"strip"` window label

Regenerated 2026-08-12 against the real tree. The previous plan lived only in a session transcript
and its `file:line` references had drifted. This file is the durable copy; regenerate it again if
it is picked up long after the date above.

Decisions this plan implements are in `.claude/todos/50-per-monitor-widget-placement-settings-ux.md`
under "Decisions (settled 2026-08-12)". Do not re-litigate them here.

## Two corrections to the old plan

- **`strip.rs` does not exist.** The strip window is declared statically in `tauri.conf.json:14-29`.
  Phase 2 has to CREATE `strip.rs`, not generalize it.
- **`poller.rs` is `src-tauri/src/system_stats/poller.rs`**, not top-level.

Grepping the literal `"strip"` window label also found two files the old list missed:
`src-tauri/src/lib.rs` and `src-tauri/capabilities/default.json`. The `settings.rs` hits are serde
tag literals for `Placement::Strip` in tests, unrelated to the window label.

## Window labelling

Tauri's static config has no templating, so it cannot declare one window per monitor. `overlay.rs`
already solved this for floating overlays: it builds each window at runtime with
`WebviewWindowBuilder` (`overlay.rs:62-72`), and `capabilities/default.json:5` lists `overlay-*` as
a glob so those dynamic windows still get `core:default`.

Scheme: the bare label `strip` stays as the primary monitor's window, so the single-monitor case has
zero behaviour change and existing tray/single-instance code keeps working. Every additional live
monitor gets a runtime-created `strip-<sanitized-device-name>`.

Hyphen, not colon: `overlay.rs:25-31`'s `label_for` already replaces every non-alphanumeric char
because Tauri documents labels as alphanumeric. Reuse that exact sanitizer.

**`capabilities/default.json:5` must gain `"strip-*"` in the same commit that introduces `strip.rs`.**
A dynamically created window missing from that glob gets no permissions at all: its webview loads and
then silently fails every `invoke`/`listen`. See the capabilities-watch gotcha in project memory.

A `StripState(Mutex<HashMap<label, device_name>>)` gives the reverse lookup, mirroring `OverlayState`
(`overlay.rs:16-22`), plus a `strip_monitor_key` command mirroring `overlay_widget_id`
(`overlay.rs:145-148`) so the frontend never parses a sanitized label itself.

## Single-window assumptions, per file

| File | Line | Assumption | Becomes |
| --- | --- | --- | --- |
| `taskbar/rect.rs` | 191 | `get_webview_window("strip")` in `position_strip` | takes an explicit target label / `DetectedTaskbar` |
| `autohide.rs` | 50, 21 | poller's single strip; `STRIP_HWND: AtomicIsize` | loop over live strips; `Mutex<Vec<isize>>` |
| `flyout.rs` | 65-66, 159 | anchors to the one strip | `open_flyout` gains `window: tauri::Window` |
| `tile_menu.rs` | 75-123 | widget-id identity | instance-id identity (Decision 4) |
| `system_stats/poller.rs` | 115, 141 | `emit_to("strip", ..)`, `is_watched` | loop all strip labels |
| `lib.rs` | 83-86, 104, 111, 177, 213 | various | resolve via `StripState` |

### Not N times the polling cost

`enumerate_taskbars()` (`taskbar/monitors.rs:68-75`) already does ONE `EnumWindows` pass and returns
every taskbar. Keep it at one call per tick, then loop the resulting `Vec<DetectedTaskbar>` in memory
against the N strips: only `HashMap` lookups and `SetWindowPos`, no extra Win32 enumeration.

Two supporting refactors make that possible:

- Split `taskbar_hidden(app)` (`rect.rs:152-179`) into a pure `taskbar_hidden_for(&DetectedTaskbar)`
  plus a thin wrapper, so it stops re-enumerating internally.
- `foreground_fullscreen()` (`autohide.rs:187-223`) already computes `wm.device_name` at line 215 and
  throws it away. Return `Option<String>` instead of `bool`, so each strip hides only for a
  fullscreen app on ITS monitor. Today every strip would hide for a fullscreen app anywhere, which is
  invisible with one strip and wrong with several.

Fold `strip::reconcile` into this same tick. There is no `WM_DISPLAYCHANGE` listener anywhere in the
codebase (grepped, none found), so the hotplug reassert from commit `6dd9c8e` already relies purely
on blind re-polling. Strip-window existence has to use the same mechanism.

## Decision 2: locking `""` to a device name

The sibling case already works for overlays: `overlay-main.ts:56-59` calls `monitor_at_point` at
drag-end and always writes back a concrete `monitor.name`, so overlay placements never persist `""`.
Strips have no equivalent write path because the Phase 4 UI does not exist yet.

- Add pure `resolve_monitor_key(monitors: &[(String, bool)], key: &str) -> Option<String>` next to
  `select_taskbar` (`taskbar/monitors.rs:18-28`), which takes a plain slice for exactly this
  testability reason. Non-empty key resolves only if still present, with **no** fallback to primary
  (unlike `select_taskbar`'s deliberate unplug fallback at `monitors.rs:27`). `""` resolves to the
  primary.
- The lock itself is a write-time contract for Phase 4: a `set_instance_monitor(app, instance_id,
  monitor)` command that writes the already-concrete device name the lane was built from. Land it in
  Phase 2 so Phase 4 needs no Rust changes.
- **Unplugged monitor keeps its widgets by construction.** `MonitorWidgets` is a plain
  `HashMap<String, Vec<StripInstance>>` with no live-monitor validation on load or save, so an absent
  device's entry is simply never touched. Reconcile skips building that window; the next tick after
  replug rebuilds it from the untouched entry. The safety comes from never mutating on absence, not
  from special-casing.

## Decision 3: one shared flyout, re-anchored

`open_flyout` gains a `window: tauri::Window` parameter, the same pattern `show_tile_menu`
(`tile_menu.rs:29-34`) already uses, and every internal `get_webview_window("strip")` becomes that
passed-in window. `spawn_poll_loop` takes the originating strip's label rather than re-fetching a
fixed one. No new window is created.

## Todo 52's call sites: neither is a mechanical rename

**`overlay.rs:101` (`s.overlays()`).** `overlays_by_instance()` returns `(InstanceId, OverlaySpec)`
and drops the widget kind, but `overlay_widget_id` (`overlay.rs:145-148`) has to tell
`overlay-main.ts` which widget module to mount. Widen it to `(InstanceId, widget_id, OverlaySpec)`
triples (both halves already sit together in `StripInstance`), key `OverlayState` by
`label_for(instance_id)` so two placements of one widget stay distinct, and have `overlay_widget_id`
return the widget_id half. The frontend contract is unchanged.

**`bridge_pomodoro.rs:74` (`is_active("pomodoro")`).** This call site only knows the widget KIND, not
which placement, so `is_active_instance` cannot replace it. It needs a genuinely new helper:

```rust
pub fn is_widget_active(&self, widget_id: &str) -> bool {
    self.monitor_widgets.all().any(|si| si.widget_id == widget_id
        && !self.hidden_widgets.iter().any(|h| h == &si.instance_id))
}
```

That generalizes the method's own doc comment ("enabled anywhere in this app") to "any non-hidden
placement of this kind, on any monitor".

Then delete `is_active`/`overlays` (`settings.rs:113-127`), rewrite
`overlays_skips_hidden_and_strip_placed_widgets` (`settings.rs:399-413`) against the new pair rather
than dropping it, and remove the two `#[allow(dead_code)]` at `settings.rs:133,139`.

## Ordered steps, each independently committable

Each leaves `cargo check` and `cargo test` green on its own.

1. `taskbar/monitors.rs`: pure `resolve_monitor_key` + `resolve_live_monitor_key` wrapper, with unit
   tests alongside the existing ones. No other file touched.
2. New `strip.rs`: `StripState`, `label_for`, `build()`, and a pure wanted-labels function taking
   `&MonitorWidgets` + live monitors so it is testable without an `AppHandle`. Register
   `.manage(...)` in `lib.rs`. Add `"strip-*"` to `capabilities/default.json:5`. Not yet called.
3. Wire `strip::reconcile` into `lib.rs::setup()` (next to `overlay::reconcile`, line 216) and
   `save_settings` (line 57). Generalize `set_strip_width` (`lib.rs:83-86`) to take a window.
4. `taskbar/rect.rs`: split out `taskbar_hidden_for`, generalize `position_strip`/`taskbar_rect`.
5. `autohide.rs`: fold strip hide/show/reassert and `strip::reconcile` into the one 250ms tick;
   `STRIP_HWND` becomes `Mutex<Vec<isize>>`; `foreground_fullscreen` returns `Option<String>`.
6. `flyout.rs`: thread the originating window through `open_flyout` and `spawn_poll_loop`. Add the
   missing `Rect::padded`/`contains` unit tests while in here.
7. `tile_menu.rs`: widget-id to instance-id identity, `move_widget` mutating the right monitor's lane.
   **Highest risk step, own commit**, with new tests for "hiding one instance does not hide a sibling
   of the same kind" and "reorder stays within one lane".
8. `system_stats/poller.rs`: loop all strip labels, matching the overlay loop already at lines 121-125.
9. Todo 52's migration and deletion, per the section above.

## What Phase 2 did NOT ship, and Phase 3 must

Found by an adversarial review of the whole 9-step diff on 2026-08-12. None of it is reachable by a
user today, because nothing can yet write a non-`""` monitor key, but all of it blocks Phase 3.

- **`main.ts` has no per-window awareness at all.** `renderTiles` renders the full flat
  `enabled_widgets` list regardless of which window it runs in, so a secondary strip would show an
  exact duplicate of the primary's whole tile set. There is no strip equivalent of
  `overlay_widget_id`. This is the first thing Phase 3 has to fix.
- **`set_instance_monitor` and `strip_monitor_key` were specified here but never written.** Grep
  confirms no hits for either. `strip_monitor_key` is what lets a strip window learn its own monitor
  without parsing its sanitized label; `set_instance_monitor` is the write path that locks a
  placement to a concrete device name (Decision 2). Phase 3/4 needs both.
- **`lib.rs`'s `toggle_strip` still only touches the bare `strip` label**, so a secondary strip would
  ignore a tray hide. Superseded anyway by
  [[55-tray-left-click-opens-settings]], which respecs the tray and already calls for iterating
  every strip.

Two HIGH bugs the same review found WERE fixed, in `9ffcee2`: `monitor_widgets` never gained an
instance for a widget adopted after first run (so Hide, move-left/right and Floating all silently
no-opped for pomodoro and spotify), and a hide-then-re-add orphaned an instance id in
`hidden_widgets` that made the widget vanish. Both now have regression tests. `Settings::persist`
and `settings::load` both call `ensure_instances`, which is the self-heal choke point every writer
converges on - keep it that way rather than adding per-writer fixups.

## Deferred to the live multi-monitor sitting

- Whether polling `available_monitors()` every 250ms reflects a real hotplug promptly.
- WebView2 create/teardown latency and flicker when a strip is built or destroyed live. Strips churn
  far more often than overlays do, so `overlay.rs`'s experience does not transfer.
- Cross-monitor DPI: `rect.rs`/`overlay.rs` physical-px math when two monitors have different scale
  factors.
- Whether `Mutex<Vec<isize>>` re-raises every strip on an Explorer restart (`autohide.rs:165-182`).
- Flyout re-anchor feel across two physical monitors: `HOVER_PAD_PX`/`COLD_CLOSE_MS` were tuned for
  one strip+flyout pair at short pixel distances.
- CPU/RAM cost of N strips plus the shared poller.
- **A monitor with no taskbar at all.** Step 5's `strip_should_hide` treats "no taskbar found for
  this device" as hidden, which matches what the old `taskbar_hidden` did when `FindWindowW` came
  back null. But with Windows' "show my taskbar on all displays" turned OFF, a secondary strip would
  be permanently hidden while `position_strip` would still happily place it against the work area.
  The two disagree. Nobody hits it with taskbars on every display, which is the feature's premise,
  so it is not worth pre-solving, but confirm the behaviour and decide then: either hide (current) or
  fall back to the work area in both paths.
- **`foreground_fullscreen`'s narrowed `SHQueryUserNotificationState` branch.** Step 5 made that
  branch require a resolvable, non-`Progman`/`WorkerW` foreground window, because it now has to
  attribute a device name to decide WHICH strip hides. Previously it could fire regardless of the
  foreground window's class. Narrow edge case (system reports presentation/D3D/busy while the desktop
  is somehow foreground), but it is a real behaviour change: verify a real fullscreen game and a
  real presentation-mode app both still hide the strip on that monitor.
