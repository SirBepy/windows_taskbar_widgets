import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { widgetById } from "./widgets/registry";
import { reportErrors } from "./shared/report-errors";
import { applyOpacity, Settings } from "./shared/widget";

reportErrors("flyout");

// #flyout itself stays fully opaque (see its comment in base.css); only
// .flyout-content, layered on that solid background, reads this var.
const refreshOpacity = () => invoke<Settings>("get_settings").then(applyOpacity).catch(() => {});
refreshOpacity();
listen("widgets-changed", refreshOpacity);

// No native context/inspect menu anywhere in this app's webviews.
document.addEventListener("contextmenu", (e) => e.preventDefault());

let cleanup: (() => void) | null = null;
let currentId: string | null = null;

function show(id: string | null) {
  if (!id || id === currentId) return;
  cleanup?.();
  cleanup = null;
  currentId = id;
  const root = document.getElementById("flyout")!;
  // Fresh container per widget: lit-html caches its ChildPart on the render
  // target, so reusing a wiped node renders into detached DOM (blank panel).
  const mount = document.createElement("div");
  mount.className = "flyout-content";
  root.replaceChildren(mount);
  const widget = widgetById(id);
  if (widget?.mountFlyout) cleanup = widget.mountFlyout(mount) ?? null;
}

listen<{ widget_id: string }>("flyout-show", (e) => show(e.payload.widget_id));

// The open_flyout emit can beat this page's listener registration (first open
// right after launch), so also pull the current id once on load.
invoke<string | null>("get_current_flyout_widget").then(show).catch(() => {});
