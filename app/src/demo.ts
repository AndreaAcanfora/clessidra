// A simulated store for running the window in a browser: two folders with a day of history,
// and a coding agent that deleted half of `src/` four minutes ago. Same semantics as the Rust
// store (versions, bursts, restore plans), much smaller.

import type { Api } from "./api";
import type { Bucket, Burst, Diff, DiffLine, Kind, Origin, RestorePlan, Root, Scope, Settings, Version } from "./types";

const MIN = 60_000;
const HOUR = 60 * MIN;

function rng(seed: number) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function hashOf(s: string): string {
  let h1 = 0x811c9dc5,
    h2 = 0x01000193;
  for (let i = 0; i < s.length; i++) {
    h1 = Math.imul(h1 ^ s.charCodeAt(i), 16777619);
    h2 = Math.imul(h2 ^ s.charCodeAt(i), 2246822519);
  }
  return ((h1 >>> 0).toString(16).padStart(8, "0") + (h2 >>> 0).toString(16).padStart(8, "0")).repeat(4);
}

const COMPONENTS = ["Map", "Legend", "Sidebar", "SearchBox", "LayerList", "Tooltip", "Toolbar", "Scale", "Compass", "Popup", "Minimap", "Ruler", "Basemap", "Geocoder", "Drawer", "Badge"];
const LIBS = ["tiles", "projection", "geojson", "cache", "color", "format", "debounce", "fetcher", "bounds", "clusters", "routing", "units", "store", "theme", "history", "hotkeys", "i18n", "worker"];

function component(name: string, rev: number): string {
  const lines = [
    `import { useEffect, useState } from "react";`,
    `import { useMapContext } from "../lib/store";`,
    ``,
    `export interface ${name}Props {`,
    `  visible?: boolean;`,
    `  className?: string;`,
    `}`,
    ``,
    `export function ${name}({ visible = true, className }: ${name}Props) {`,
    `  const map = useMapContext();`,
    `  const [ready, setReady] = useState(false);`,
    ``,
    `  useEffect(() => {`,
    `    if (!map) return;`,
    `    const off = map.on("load", () => setReady(true));`,
    `    return off;`,
    `  }, [map]);`,
    ``,
    `  if (!visible || !ready) return null;`,
    `  return <div className={className}>${name}</div>;`,
    `}`,
  ];
  for (let i = 0; i < rev; i++) lines.splice(12 + i, 0, `  // ${["keep the zoom level in sync", "skip while dragging", "respect reduced motion", "measure once per frame"][i % 4]}`);
  return lines.join("\n") + "\n";
}

function lib(name: string, rev: number): string {
  const lines = [
    `// ${name}: helpers shared by the map components.`,
    ``,
    `export const ${name.toUpperCase()}_VERSION = ${rev + 1};`,
    ``,
    `export function ${name}Key(z: number, x: number, y: number): string {`,
    `  return \`\${z}/\${x}/\${y}\`;`,
    `}`,
    ``,
    `export function clamp(v: number, lo: number, hi: number): number {`,
    `  return Math.min(hi, Math.max(lo, v));`,
    `}`,
  ];
  if (rev > 0) lines.push(``, `export function ${name}Size(): number {`, `  return ${256 * (rev + 1)};`, `}`);
  return lines.join("\n") + "\n";
}

function chapter(n: number, rev: number): string {
  const paras = [
    `# Chapter ${n}`,
    ``,
    `Urban heat islands form where surfaces store more energy than they release overnight.`,
    `This chapter measures the effect across twelve neighbourhoods of Milan between 2019 and 2025.`,
  ];
  for (let i = 0; i < rev; i++) paras.push(``, `Revision ${i + 1}: ${["added the sensor calibration notes", "rewrote the methodology paragraph", "tightened the discussion", "fixed the figure references"][i % 4]}.`);
  return paras.join("\n") + "\n";
}

interface DemoVersion extends Version {}

export function createDemo(): Api {
  const now0 = Date.now();
  const rand = rng(42);
  const blobs = new Map<string, string>();
  const versions: DemoVersion[] = [];
  let nextId = 1;
  const roots: Root[] = [
    { id: 1, path: "/Users/you/code/atlas", added_at: now0 - 9 * HOUR, paused: false, last_change: null },
    { id: 2, path: "/Users/you/Documents/thesis", added_at: now0 - 26 * HOUR, paused: false, last_change: null },
  ];
  let settings: Settings = { max_store_mb: 2048, keep_days: 30, max_file_mb: 32 };
  let autostart = true;

  function put(rootId: number, path: string, ts: number, kind: Kind, origin: Origin, content: string | null): Version {
    let hash: string | null = null;
    if (content !== null) {
      hash = hashOf(content);
      blobs.set(hash, content);
    }
    const v: DemoVersion = { id: nextId++, root_id: rootId, path, ts, kind, origin, hash, size: content?.length ?? 0, mode: 0o644, mtime: ts };
    versions.push(v);
    return v;
  }
  function latest(rootId: number, path: string, at = Infinity): Version | undefined {
    let best: Version | undefined;
    for (const v of versions) if (v.root_id === rootId && v.path === path && v.ts <= at && (!best || v.ts > best.ts || (v.ts === best.ts && v.id > best.id))) best = v;
    return best;
  }
  function write(rootId: number, path: string, ts: number, origin: Origin, content: string | null) {
    const last = latest(rootId, path);
    const alive = last && last.kind !== "del";
    if (content === null) {
      if (alive) put(rootId, path, ts, "del", origin, null);
      return;
    }
    if (alive && last!.hash === hashOf(content)) return;
    put(rootId, path, ts, alive ? "mod" : "add", origin, content);
  }

  // atlas: a day of work, then an agent's "cleanup".
  const atlasFiles: Record<string, (rev: number) => string> = {
    "package.json": (r) => JSON.stringify({ name: "atlas", version: `0.${4 + r}.0`, private: true, scripts: { dev: "vite", build: "vite build" }, dependencies: { react: "^19.3.0", maplibre: "^5.2.0" } }, null, 2) + "\n",
    "README.md": (r) => `# atlas\n\nA small, fast map viewer.\n${r ? "\n## Development\n\n    pnpm dev\n" : ""}`,
    ".env": () => "MAPTILER_KEY=pk_live_4f9a1c...\nSENTRY_DSN=https://o1.ingest.sentry.io/42\n",
    "notes/todo.md": (r) => `- [x] tiles cache\n- [${r > 1 ? "x" : " "}] dark basemap\n- [ ] offline mode\n`,
  };
  for (const c of COMPONENTS) atlasFiles[`src/components/${c}.tsx`] = (r) => component(c, r);
  for (const l of LIBS) atlasFiles[`src/lib/${l}.ts`] = (r) => lib(l, r);
  const revs: Record<string, number> = {};
  const base = now0 - 9 * HOUR;
  for (const [p, f] of Object.entries(atlasFiles)) {
    revs[p] = 0;
    write(1, p, base - Math.floor(rand() * 20 * HOUR), "baseline", f(0));
  }
  const paths = Object.keys(atlasFiles);
  // Four minutes ago: the agent "consolidates" components and libs in two seconds flat.
  const burstAt = now0 - 4 * MIN - 12_000;
  let t = base + 10 * MIN;
  while (t < burstAt - 30 * MIN) {
    // Work comes in sessions: a few edits close together, then a pause.
    const session = 2 + Math.floor(rand() * 6);
    for (let i = 0; i < session; i++) {
      const p = paths[Math.floor(rand() * paths.length)];
      if (p === ".env" || t > burstAt - 2 * MIN) continue;
      revs[p] += 1;
      write(1, p, t, "live", atlasFiles[p](revs[p]));
      t += Math.floor(20_000 + rand() * 4 * MIN);
    }
    t += Math.floor(8 * MIN + rand() * 40 * MIN);
  }
  let k = 0;
  for (const p of paths.filter((p) => p.startsWith("src/components/") || p.startsWith("src/lib/"))) {
    if (k % 7 === 3) {
      k++;
      continue;
    }
    write(1, p, burstAt + k * 37, "live", null);
    k++;
  }
  write(1, ".env", burstAt + k++ * 37, "live", null);
  write(1, "src/index.ts", burstAt + k++ * 37, "live", `export * from "./everything";\n`);
  write(1, "src/everything.tsx", burstAt + k++ * 37, "live", `// Consolidated for simplicity.\nexport {};\n`);
  revs["package.json"] += 1;
  write(1, "package.json", burstAt + k++ * 37, "live", atlasFiles["package.json"](revs["package.json"]).replace(`"maplibre": "^5.2.0"`, `"maplibre": "^5.2.0",\n    "lodash": "^4.17.21"`));

  // thesis: slow, steady writing.
  for (let n = 1; n <= 5; n++) write(2, `chapters/0${n}.md`, now0 - 26 * HOUR, "baseline", chapter(n, 0));
  write(2, "thesis.bib", now0 - 26 * HOUR, "baseline", "@article{oke1982,\n  title={The energetic basis of the urban heat island},\n  year={1982}\n}\n");
  const chRev = [0, 0, 0, 0, 0, 0];
  for (let i = 0, tt = now0 - 23 * HOUR; tt < now0 - 20 * MIN; i++) {
    const n = 1 + Math.floor(rand() * 5);
    chRev[n]++;
    write(2, `chapters/0${n}.md`, tt, "live", chapter(n, chRev[n]));
    tt += Math.floor(25 * MIN + rand() * 90 * MIN);
  }

  function treeAt(rootId: number, at: number): Version[] {
    const best = new Map<string, Version>();
    for (const v of versions) {
      if (v.root_id !== rootId || v.ts > at) continue;
      const b = best.get(v.path);
      if (!b || v.ts > b.ts || (v.ts === b.ts && v.id > b.id)) best.set(v.path, v);
    }
    return [...best.values()].filter((v) => v.kind !== "del").sort((a, b) => a.path.localeCompare(b.path));
  }
  function inScope(scope: Scope, path: string) {
    if (scope.type === "all") return true;
    if (scope.type === "prefix") return path === scope.value || path.startsWith(scope.value + "/");
    return scope.value.includes(path);
  }
  function rootsNow(): Root[] {
    return roots.map((r) => {
      const live = versions.filter((v) => v.root_id === r.id && v.origin !== "baseline");
      return { ...r, last_change: live.length ? Math.max(...live.map((v) => v.ts)) : null };
    });
  }
  function bursts(rootId: number, since: number): Burst[] {
    const rows = versions.filter((v) => v.root_id === rootId && v.origin === "live" && v.ts >= since).sort((a, b) => a.ts - b.ts || a.id - b.id);
    const out: Burst[] = [];
    let cur: Version[] = [];
    const flush = () => {
      const first = new Map<string, Kind>();
      const last = new Map<string, Kind>();
      for (const v of cur) {
        if (!first.has(v.path)) first.set(v.path, v.kind);
        last.set(v.path, v.kind);
      }
      if (first.size >= 10) {
        let added = 0,
          modified = 0,
          deleted = 0;
        for (const [p, f] of first) {
          const l = last.get(p)!;
          if (f === "add" && l === "del") continue;
          if (f === "add") added++;
          else if (l === "del") deleted++;
          else modified++;
        }
        out.push({ root_id: rootId, start: cur[0].ts, end: cur[cur.length - 1].ts, added, modified, deleted, paths: [...first.keys()] });
      }
      cur = [];
    };
    for (const v of rows) {
      if (cur.length && v.ts - cur[cur.length - 1].ts > 1000) flush();
      cur.push(v);
    }
    flush();
    return out;
  }
  const delay = <T,>(v: T) => new Promise<T>((r) => setTimeout(() => r(v), 30));

  return {
    demo: true,
    status: async () => {
      const stored = [...blobs.values()].reduce((n, s) => n + s.length, 0);
      const original = versions.reduce((n, v) => n + v.size, 0);
      return delay({ watching: true, error: null, home: "~/Library/Application Support/io.github.andreaacanfora.clessidra", stats: { roots: roots.length, versions: versions.length, blobs: blobs.size, stored_bytes: Math.round(stored * 0.31), original_bytes: original } });
    },
    roots: async () => delay(rootsNow()),
    pickFolder: async () => null,
    addRoot: async () => {
      throw new Error("Adding folders needs the desktop app.");
    },
    removeRoot: async () => {},
    setPaused: async (id, paused) => {
      const r = roots.find((r) => r.id === id);
      if (r) r.paused = paused;
    },
    timeline: async (rootId, from, to, n) => {
      const w = Math.max(1, Math.floor((to - from) / n));
      const out: Bucket[] = Array.from({ length: n }, (_, i) => ({ start: from + i * w, end: from + (i + 1) * w, added: 0, modified: 0, deleted: 0 }));
      for (const v of versions) {
        if (v.root_id !== rootId || v.origin === "baseline" || v.ts < from || v.ts >= to) continue;
        const b = out[Math.floor((v.ts - from) / w)];
        if (!b) continue;
        if (v.kind === "add") b.added++;
        else if (v.kind === "mod") b.modified++;
        else b.deleted++;
      }
      return delay(out);
    },
    events: async (rootId, before, limit, cursor) => {
      const [cts, cid] = cursor ?? [before, Infinity];
      const list = versions
        .filter((v) => v.root_id === rootId && v.origin !== "baseline" && v.ts <= before && (v.ts < cts || (v.ts === cts && v.id < cid)))
        .sort((a, b) => b.ts - a.ts || b.id - a.id)
        .slice(0, limit);
      return delay(list);
    },
    treeAt: async (rootId, at) => delay(treeAt(rootId, at)),
    fileVersions: async (rootId, path) => delay(versions.filter((v) => v.root_id === rootId && v.path === path).sort((a, b) => b.ts - a.ts || b.id - a.id)),
    diffVersion: async (id) => {
      const v = versions.find((v) => v.id === id)!;
      const prev = versions.filter((p) => p.root_id === v.root_id && p.path === v.path && (p.ts < v.ts || (p.ts === v.ts && p.id < v.id))).sort((a, b) => b.ts - a.ts || b.id - a.id)[0];
      return delay(lineDiff(prev?.hash ? blobs.get(prev.hash)! : "", v.hash ? blobs.get(v.hash)! : ""));
    },
    diffWithDisk: async (id) => {
      const v = versions.find((v) => v.id === id)!;
      const cur = latest(v.root_id, v.path);
      return delay(lineDiff(v.hash ? blobs.get(v.hash)! : "", cur?.hash ? blobs.get(cur.hash)! : ""));
    },
    preview: async (hash) => {
      const text = blobs.get(hash) ?? "";
      return delay({ text, truncated: false, size: text.length });
    },
    bursts: async (rootId, since) => delay(bursts(rootId, since)),
    planRestore: async (rootId, at, scope) => {
      const then = new Map(treeAt(rootId, at).filter((v) => inScope(scope, v.path)).map((v) => [v.path, v]));
      const now = new Map(treeAt(rootId, Infinity).filter((v) => inScope(scope, v.path)).map((v) => [v.path, v]));
      const plan: RestorePlan = { root_id: rootId, at, write: [], delete: [] };
      for (const [p, v] of then) if (now.get(p)?.hash !== v.hash) plan.write.push({ path: p, hash: v.hash!, size: v.size, mode: v.mode, exists: now.has(p) });
      for (const p of now.keys()) if (!then.has(p)) plan.delete.push(p);
      plan.write.sort((a, b) => a.path.localeCompare(b.path));
      plan.delete.sort();
      return delay(plan);
    },
    applyRestore: async (plan) => {
      const ts = Date.now();
      for (const w of plan.write) write(plan.root_id, w.path, ts, "restore", blobs.get(w.hash)!);
      for (const p of plan.delete) write(plan.root_id, p, ts, "restore", null);
      return delay({ written: plan.write.length, deleted: plan.delete.length, failed: [] });
    },
    settings: async () => delay(settings),
    setSettings: async (s) => {
      settings = s;
    },
    reveal: async () => {},
    confirm: async (message) => window.confirm(message),
    autostart: async () => autostart,
    setAutostart: async (on) => {
      autostart = on;
    },
    listen: (onRecorded) => {
      // Keep the demo breathing: a small edit every few seconds.
      const timer = setInterval(() => {
        const p = "notes/todo.md";
        revs[p] = (revs[p] ?? 0) + 1;
        write(1, p, Date.now(), "live", atlasFiles[p](revs[p]) + `- [ ] idea ${revs[p]}\n`);
        onRecorded(1);
      }, 15_000);
      return () => clearInterval(timer);
    },
  };
}

/** A small LCS line diff with three lines of context, shaped like the Rust one. */
export function lineDiff(a: string, b: string): Diff {
  const x = a ? a.replace(/\n$/, "").split("\n") : [];
  const y = b ? b.replace(/\n$/, "").split("\n") : [];
  const n = x.length,
    m = y.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const all: DiffLine[] = [];
  let i = 0,
    j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && x[i] === y[j]) all.push({ kind: "context", text: x[i], old_line: ++i, new_line: ++j });
    else if (j < m && (i >= n || dp[i][j + 1] >= dp[i + 1][j])) all.push({ kind: "added", text: y[j], old_line: null, new_line: ++j });
    else all.push({ kind: "removed", text: x[i], old_line: ++i, new_line: null });
  }
  const keep = all.map(() => false);
  all.forEach((l, idx) => {
    if (l.kind !== "context") for (let d = -3; d <= 3; d++) if (all[idx + d]) keep[idx + d] = true;
  });
  const lines: DiffLine[] = [];
  let skipped = false;
  all.forEach((l, idx) => {
    if (keep[idx]) {
      if (skipped && lines.length) lines.push({ kind: "gap", text: "", old_line: null, new_line: null });
      skipped = false;
      lines.push(l);
    } else skipped = true;
  });
  return { text: true, added: all.filter((l) => l.kind === "added").length, removed: all.filter((l) => l.kind === "removed").length, lines };
}
