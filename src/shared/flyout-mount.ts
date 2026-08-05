/**
 * Hiding the flyout must unmount: a hidden webview keeps running its JS, so a
 * widget left mounted polls forever - 15x the app's idle CPU after one hover.
 */
export function createFlyoutController(mount: (id: string) => (() => void) | null) {
  let cleanup: (() => void) | null = null;
  let currentId: string | null = null;

  return {
    show(id: string | null): void {
      if (!id || id === currentId) return;
      cleanup?.();
      currentId = id;
      cleanup = mount(id);
    },
    hide(): void {
      cleanup?.();
      cleanup = null;
      currentId = null;
    },
    mounted: (): string | null => currentId,
  };
}
