/** Order math for the settings strip's drag-and-drop, kept DOM-free so the
 * insert / move / remove cases stay unit-testable. */

export function removeId(order: string[], id: string): string[] {
  return order.filter((x) => x !== id);
}

/** Index is counted among the entries that are NOT `id`, which is what the drop
 * slot's position reads as - so moving an existing tile and dropping in a new
 * one share one path instead of needing a shift correction for the removal. */
export function placeAt(order: string[], id: string, index: number): string[] {
  const rest = removeId(order, id);
  return insertAt(rest, id, index);
}

/** Removes ONE copy. A lane may legitimately hold two placements of the same widget
 * kind, and those are the same string, so a filter would take the one nobody dragged. */
export function removeFirst(order: string[], id: string): string[] {
  const at = order.indexOf(id);
  return at === -1 ? [...order] : [...order.slice(0, at), ...order.slice(at + 1)];
}

/** Inserts at `index`, clamped to the array. */
export function insertAt(order: string[], id: string, index: number): string[] {
  const at = Math.max(0, Math.min(index, order.length));
  return [...order.slice(0, at), id, ...order.slice(at)];
}
