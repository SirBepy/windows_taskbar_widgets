import "@phosphor-icons/web/regular";
import { listen } from "@tauri-apps/api/event";
import { html, render } from "lit-html";
import { reportErrors } from "./shared/report-errors";
import { mountSettings } from "./views/settings/settings";
import { selectWidgetInStrip } from "./views/settings/widget-strip-field";

reportErrors("settings");

// No native context/inspect menu anywhere in this app's webviews.
document.addEventListener("contextmenu", (e) => e.preventDefault());

const headerRoot = document.getElementById("settings-header")!;
const bodyRoot = document.getElementById("settings-body")!;

// Kept fresh by every renderSettingsPage page-change, so goToRoot() (the deep
// link's "get back to root before navigating into Widgets" step) always pops
// against the current stack depth rather than a stale snapshot.
let depth = 1;
let pop: () => void = () => {};

function renderHeader(title: string, d: number, p: () => void): void {
  depth = d;
  pop = p;
  render(
    html`
      <div class="kit-header">
        ${depth > 1
          ? html`<button class="kit-header-back" @click=${() => pop()}>‹ Back</button>`
          : html`<span class="kit-header-spacer"></span>`}
        <h1 class="kit-header-title">${title}</h1>
        <span class="kit-header-spacer"></span>
      </div>
    `,
    headerRoot,
  );
}

function goToRoot(): void {
  while (depth > 1) pop();
}

mountSettings(bodyRoot, { onHeaderChange: renderHeader });

// "Edit this widget" from a tile's right-click menu (tile_menu.rs's open_settings):
// null section just surfaces the widget list, an id also expands + scrolls to it.
listen<{ section: string | null }>("settings-navigate", (e) => {
  goToRoot();
  document.querySelector<HTMLElement>('[data-nav="section-widgets"]')?.click();
  const widgetId = e.payload.section;
  if (widgetId) selectWidgetInStrip(widgetId);
});
