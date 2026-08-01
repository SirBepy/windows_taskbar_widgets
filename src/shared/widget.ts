// v1 widget contract. Built-ins implement this directly; the planned external
// manifest/bundle loader will adapt third-party widgets onto this same shape.
export interface TaskbarWidget {
  id: string;
  name: string;
  /** CSS size of the hover flyout; omit for tile-only widgets. */
  flyout?: { widthCss: number; heightCss: number };
  /** Render the always-visible strip tile. Returns a cleanup fn. */
  mountTile(root: HTMLElement): () => void;
  mountFlyout?(root: HTMLElement): () => void;
  /** Extra entries appended to this tile's native context menu. */
  menuItems?: () => { id: string; label: string }[];
  onMenuAction?: (id: string) => void;
}

export function fmtBytes(bytes: number, digits = 1): string {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1024) return `${(gb / 1024).toFixed(digits)} TB`;
  return `${gb.toFixed(digits)} GB`;
}

/** "2h 14m" until an RFC3339 instant; empty when past or missing. */
export function fmtCountdown(resetsAt: string | null | undefined): string {
  if (!resetsAt) return "";
  const ms = new Date(resetsAt).getTime() - Date.now();
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const totalMin = Math.ceil(ms / 60_000);
  const d = Math.floor(totalMin / 1440);
  const h = Math.floor((totalMin % 1440) / 60);
  const m = totalMin % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}
