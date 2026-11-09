// The window's view of the store. Inside the app every call goes to the Rust side; opened in a
// plain browser (`pnpm dev`) the page runs against a simulated store instead, which is how the
// screenshots in the README are made.

import type {
  Bucket,
  Burst,
  Diff,
  Preview,
  RestorePlan,
  RestoreReport,
  Root,
  Scope,
  Settings,
  Status,
  Version,
} from "./types";
import { createDemo } from "./demo";

export interface Api {
  demo: boolean;
  status(): Promise<Status>;
  roots(): Promise<Root[]>;
  pickFolder(): Promise<string | null>;
  addRoot(path: string): Promise<Root>;
  removeRoot(id: number): Promise<void>;
  setPaused(id: number, paused: boolean): Promise<void>;
  timeline(rootId: number, from: number, to: number, buckets: number): Promise<Bucket[]>;
  events(rootId: number, before: number, limit: number, cursor?: [number, number]): Promise<Version[]>;
  treeAt(rootId: number, at: number): Promise<Version[]>;
  fileVersions(rootId: number, path: string): Promise<Version[]>;
  diffVersion(id: number): Promise<Diff>;
  diffWithDisk(id: number): Promise<Diff>;
  preview(hash: string): Promise<Preview>;
  bursts(rootId: number, since: number): Promise<Burst[]>;
  planRestore(rootId: number, at: number, scope: Scope): Promise<RestorePlan>;
  applyRestore(plan: RestorePlan): Promise<RestoreReport>;
  settings(): Promise<Settings>;
  setSettings(s: Settings): Promise<void>;
  reveal(rootId: number, path: string): Promise<void>;
  confirm(message: string, okLabel: string): Promise<boolean>;
  autostart(): Promise<boolean>;
  setAutostart(on: boolean): Promise<void>;
  /** Subscribe to new versions (root id, count) and bursts. Returns an unsubscribe function. */
  listen(onRecorded: (rootId: number) => void, onBurst: (b: Burst) => void, onSnapshot: (rootId: number) => void): () => void;
}

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriApi(): Promise<Api> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const dialog = await import("@tauri-apps/plugin-dialog");
  const opener = await import("@tauri-apps/plugin-opener");
  const autostart = await import("@tauri-apps/plugin-autostart");
  return {
    demo: false,
    status: () => invoke("status"),
    roots: () => invoke("roots"),
    pickFolder: async () => {
      const r = await dialog.open({ directory: true, multiple: false, title: "Choose a folder to keep the history of" });
      return typeof r === "string" ? r : null;
    },
    addRoot: (path) => invoke("add_root", { path }),
    removeRoot: (id) => invoke("remove_root", { id }),
    setPaused: (id, paused) => invoke("set_paused", { id, paused }),
    timeline: (rootId, from, to, buckets) => invoke("timeline", { rootId, from, to, buckets }),
    events: (rootId, before, limit, cursor) => invoke("events", { rootId, prefix: "", before, cursor: cursor ?? null, limit }),
    treeAt: (rootId, at) => invoke("tree_at", { rootId, at }),
    fileVersions: (rootId, path) => invoke("file_versions", { rootId, path }),
    diffVersion: (id) => invoke("diff_version", { id }),
    diffWithDisk: (id) => invoke("diff_with_disk", { id }),
    preview: (hash) => invoke("preview", { hash }),
    bursts: (rootId, since) => invoke("bursts", { rootId, since }),
    planRestore: (rootId, at, scope) => invoke("plan_restore", { rootId, at, scope }),
    applyRestore: (plan) => invoke("apply_restore", { plan }),
    settings: () => invoke("settings"),
    setSettings: (settings) => invoke("set_settings", { settings }),
    reveal: async (rootId, path) => {
      const abs: string = await invoke("reveal", { rootId, path });
      await opener.revealItemInDir(abs);
    },
    confirm: (message, okLabel) => dialog.ask(message, { title: "clessidra", kind: "warning", okLabel, cancelLabel: "Cancel" }),
    autostart: () => autostart.isEnabled(),
    setAutostart: (on) => (on ? autostart.enable() : autostart.disable()),
    listen: (onRecorded, onBurst, onSnapshot) => {
      const offs = [
        listen<[number, number]>("recorded", (e) => onRecorded(e.payload[0])),
        listen<Burst>("burst", (e) => onBurst(e.payload)),
        listen<[number, unknown]>("snapshot-done", (e) => onSnapshot(e.payload[0])),
      ];
      return () => offs.forEach((p) => p.then((off) => off()));
    },
  };
}

export const api: Promise<Api> = inTauri ? tauriApi() : Promise.resolve(createDemo());
