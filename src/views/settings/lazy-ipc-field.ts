import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

/**
 * Fetches `cmd` once over IPC, stores the result via `setValue`, then emits
 * "settings-reset" so the kit repaints. Returns an idempotent ensureLoaded().
 */
export function lazyIpcField<T>(cmd: string, setValue: (value: T) => void): () => void {
  let started = false;
  return () => {
    if (started) return;
    started = true;
    invoke<T>(cmd)
      .then((value) => {
        setValue(value);
        void emit("settings-reset");
      })
      .catch(() => {});
  };
}
