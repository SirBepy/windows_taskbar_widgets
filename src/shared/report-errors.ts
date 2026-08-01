import { invoke } from "@tauri-apps/api/core";

export function reportErrors(page: string) {
  const send = (level: string, msg: string) =>
    invoke("log_js", { level, msg: `[${page}] ${msg}` }).catch(() => {});
  window.addEventListener("error", (e) =>
    send("error", `${e.message} @ ${e.filename}:${e.lineno}`),
  );
  window.addEventListener("unhandledrejection", (e) =>
    send("error", `unhandled rejection: ${String(e.reason)}`),
  );
  send("info", "booted");
}
