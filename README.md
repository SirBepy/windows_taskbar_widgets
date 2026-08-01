# Taskbar Widgets

A Tauri 2 host app that renders a strip of live widgets over the empty left
region of the Windows 11 taskbar. Widgets show a compact always-visible tile;
hovering a tile pops a richer flyout window anchored above the taskbar.

## Built-in widgets

- **System stats** - CPU %, RAM used/total, GPU % + temp + VRAM (NVIDIA via
  NVML), per-drive free space, CPU temp when the board exposes ACPI thermal
  zones (hidden otherwise).
- **Claude usage** - per-account 5h/7d usage dials read directly (read-only)
  from Claude Conductor's on-disk state (`%APPDATA%\claude-conductor`:
  `accounts.json` + `companion.db`). Conductor does not need to be running.

## Architecture

- `src-tauri/src/taskbar.rs` - taskbar rect via `SHAppBarMessage`, strip
  positioning (left-anchored, content-sized via `set_strip_width`).
- `src-tauri/src/flyout.rs` - single reusable flyout window; hover choreography
  (strip + flyout zones, 300ms grace close).
- `src-tauri/src/system_stats.rs` - background poller (sysinfo / nvml / wmi),
  pushes `system-stats` events.
- `src-tauri/src/conductor_data.rs` - read-only SQLite/JSON bridge into
  Conductor's data dir.
- `src/shared/widget.ts` - the widget contract (`mountTile` / `mountFlyout` /
  flyout size). Built-ins register in `src/widgets/registry.ts`. A future
  manifest loader for third-party app widgets adapts onto the same contract.

## Dev

```
npm install
npm run tauri dev
```

Webview console errors are forwarded to the Rust log (`log_js` command), so
they show up in the supervised dev logs / `<log-dir>/app.log`.
