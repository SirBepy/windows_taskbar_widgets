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
  const at = Math.max(0, Math.min(index, rest.length));
  return [...rest.slice(0, at), id, ...rest.slice(at)];
}
