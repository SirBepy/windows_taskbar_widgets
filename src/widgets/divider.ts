import { html, render } from "lit-html";
import { TaskbarWidget } from "../shared/widget";

// Synthetic per-instance widget: registry.ts routes any id starting with "divider:"
// here instead of the static ALL array, since divider ids are minted at add-time
// (shared/divider.ts's makeDividerId) rather than declared up front like real widgets.
// No flyout/menuItems/configFields - a divider has none of those.
export function dividerWidget(id: string): TaskbarWidget {
  return {
    id,
    name: "Divider",
    mountTile(root) {
      root.classList.add("divider-tile");
      render(html`<span class="divider-rule"></span>`, root);
      return () => {};
    },
  };
}
