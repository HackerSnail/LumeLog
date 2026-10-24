import { invoke } from "@tauri-apps/api/core";

// Task 8 first-paint probe (spec §9 `app_ping`, the P1 interactivity endpoint).
// The real editor UI (CodeMirror, tabs, status bar) lands in Task 9; the large
// file virtual list in Task 10.
async function ping(): Promise<void> {
  const el = document.getElementById("root");
  try {
    const res = await invoke<string>("app_ping");
    if (el) el.textContent = `LumeLog shell online. app_ping -> ${res}`;
  } catch (err) {
    if (el) el.textContent = `app_ping failed: ${String(err)}`;
    console.error(err);
  }
}

ping();
