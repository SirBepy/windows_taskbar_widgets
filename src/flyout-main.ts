import "@phosphor-icons/web/regular";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { widgetById } from "./widgets/registry";
import { createFlyoutPanel, PanelShow } from "./shared/flyout-panel";
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

const panel = createFlyoutPanel({
  panel: document.getElementById("flyout")!,
  mountWidget: (id, root) => widgetById(id)?.mountFlyout?.(root) ?? null,
  onHidden: () => invoke("flyout_hide_done").catch(() => {}),
});

listen<PanelShow>("flyout-show", (e) => panel.show(e.payload));

// Hiding the window does not pause this page, so unmounting on hide is the only
// thing that stops the widget's polling. See flyout-mount.ts.
listen("flyout-hide", () => panel.hide());

// The open_flyout emit can beat this page's listener registration (first open
// right after launch), so also pull the current panel once on load.
invoke<PanelShow | null>("get_current_flyout")
  .then((p) => {
    if (p) panel.show(p);
  })
  .catch(() => {});
