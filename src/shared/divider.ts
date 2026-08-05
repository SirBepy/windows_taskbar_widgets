// Divider entries live in the same enabled_widgets array as real widget ids
// (src-tauri/src/tile_menu.rs's reorder_widgets persists it verbatim), but unlike
// a widget id, multiple dividers can coexist - so each instance gets a uuid
// suffix rather than a fixed, reused id.
const DIVIDER_PREFIX = "divider:";

export function isDividerId(id: string): boolean {
  return id.startsWith(DIVIDER_PREFIX);
}

export function makeDividerId(): string {
  return `${DIVIDER_PREFIX}${crypto.randomUUID()}`;
}
