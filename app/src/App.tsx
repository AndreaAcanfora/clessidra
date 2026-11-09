import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Api } from "./api";
import type { Bucket, Burst, RestorePlan, Root, Scope, Status, Version } from "./types";
import { ago, clock, day, folderName, plural, tildify } from "./format";
import { Sidebar } from "./components/Sidebar";
import { Timeline } from "./components/Timeline";
import { Changes } from "./components/Changes";
import { Detail } from "./components/Detail";
import { RestoreSheet } from "./components/RestoreSheet";
import { SettingsSheet } from "./components/SettingsSheet";
import { CloseIcon, Hourglass } from "./components/icons";

const RANGES = [
  { key: "1h", label: "1 hour", ms: 3_600_000 },
  { key: "6h", label: "6 hours", ms: 6 * 3_600_000 },
  { key: "24h", label: "24 hours", ms: 24 * 3_600_000 },
  { key: "7d", label: "7 days", ms: 7 * 86_400_000 },
  { key: "30d", label: "30 days", ms: 30 * 86_400_000 },
] as const;

/** Bursts worth interrupting someone for. */
const notable = (b: Burst) => b.deleted >= 5 || b.paths.length >= 25;

interface Sheet {
  title: string;
  plan: RestorePlan;
  scope: Scope;
  /** Set when the sheet undoes a burst, so its banner goes away once done. */
  burst?: number;
}

interface Toast {
  text: string;
  undo?: () => void;
}

export function App({ api }: { api: Api }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [roots, setRoots] = useState<Root[] | null>(null);
  const [rootId, setRootId] = useState<number | null>(null);
  const [range, setRange] = useState<(typeof RANGES)[number]["key"]>("6h");
  const [now, setNow] = useState(Date.now());
  const [cursor, setCursor] = useState<number | null>(null);
  const [width, setWidth] = useState(800);
  const [buckets, setBuckets] = useState<Bucket[]>([]);
  const [bursts, setBursts] = useState<Burst[]>([]);
  const [selected, setSelected] = useState<Version | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<Toast | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [dismissed, setDismissed] = useState<Set<number>>(new Set());

  const root = roots?.find((r) => r.id === rootId) ?? null;
  const span = RANGES.find((r) => r.key === range)!.ms;
  const from = now - span;
  const at = cursor ?? now;

  const loadRoots = useCallback(async () => {
    const [r, s] = await Promise.all([api.roots(), api.status()]);
    setRoots(r);
    setStatus(s);
    setRootId((cur) => (cur !== null && r.some((x) => x.id === cur) ? cur : (r[0]?.id ?? null)));
  }, [api]);

  useEffect(() => {
    loadRoots();
  }, [loadRoots]);

  // Live updates: new versions refresh what is on screen (a few times a second at most).
  const pending = useRef<number | null>(null);
  useEffect(() => {
    const bump = () => {
      if (pending.current) return;
      pending.current = window.setTimeout(() => {
        pending.current = null;
        setNow(Date.now());
        setRefresh((n) => n + 1);
        loadRoots();
      }, 400);
    };
    const off = api.listen(
      () => bump(),
      (b) => {
        bump();
        if (notable(b)) setRootId((cur) => cur ?? b.root_id);
      },
      () => bump(),
    );
    const tick = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => {
      off();
      clearInterval(tick);
    };
  }, [api, loadRoots]);

  useEffect(() => {
    if (rootId === null) return;
    let stale = false;
    const n = Math.max(24, Math.min(400, Math.floor(width / 5)));
    api.timeline(rootId, from, now, n).then((b) => !stale && setBuckets(b));
    api.bursts(rootId, now - 30 * 86_400_000).then((b) => !stale && setBursts(b));
    return () => {
      stale = true;
    };
  }, [api, rootId, from, now, width, refresh]);

  // Switching folders starts over at "now".
  useEffect(() => {
    setCursor(null);
    setSelected(null);
  }, [rootId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !sheet && !settingsOpen) setCursor(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sheet, settingsOpen]);

  const banner = useMemo(() => {
    const recent = bursts.filter((b) => notable(b) && now - b.end < 24 * 3_600_000 && !dismissed.has(b.start));
    return recent[recent.length - 1] ?? null;
  }, [bursts, now, dismissed]);

  async function addFolder() {
    const path = await api.pickFolder();
    if (!path) return;
    try {
      const r = await api.addRoot(path);
      await loadRoots();
      setRootId(r.id);
      setToast({ text: `Keeping the history of ${folderName(r.path)}. Taking a first copy of every file…` });
    } catch (e) {
      setToast({ text: String(e) });
    }
  }

  async function togglePause(r: Root) {
    await api.setPaused(r.id, !r.paused);
    loadRoots();
  }

  async function openPlan(title: string, t: number, scope: Scope, burst?: number) {
    if (rootId === null) return;
    const plan = await api.planRestore(rootId, t, scope);
    setSheet({ title, plan, scope, burst });
  }

  /** Apply a plan, then offer to undo it. */
  async function apply(plan: RestorePlan, scope: Scope, done: string): Promise<boolean> {
    const before = Date.now();
    setBusy(true);
    try {
      const r = await api.applyRestore(plan);
      setSheet(null);
      setRefresh((n) => n + 1);
      setNow(Date.now());
      loadRoots();
      const failed = r.failed.length ? ` ${plural(r.failed.length, "file")} could not be written: ${r.failed[0][1]}` : "";
      setToast({
        text: `${done}${failed}`,
        undo: async () => {
          const back = await api.planRestore(plan.root_id, before, scope);
          await api.applyRestore(back);
          setRefresh((n) => n + 1);
          setNow(Date.now());
          setToast({ text: "Undone." });
        },
      });
      return true;
    } catch (e) {
      setToast({ text: `Could not restore: ${e}` });
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function restoreFile(path: string, t: number, label: string) {
    if (rootId === null) return;
    const scope: Scope = { type: "prefix", value: path };
    const plan = await api.planRestore(rootId, t, scope);
    if (!plan.write.length && !plan.delete.length) return setToast({ text: "It already looks like that." });
    await apply(plan, scope, label === "Bring it back" ? `Brought back ${path}.` : `Restored ${path} to ${clock(t)}.`);
  }

  if (roots === null) return <div className="boot" />;

  return (
    <div className="app">
      <Sidebar
        roots={roots}
        selected={rootId}
        status={status}
        now={now}
        onSelect={setRootId}
        onAdd={addFolder}
        onPause={togglePause}
        onSettings={() => setSettingsOpen(true)}
      />

      {root === null ? (
        <main className="empty" data-tauri-drag-region>
          <Hourglass className="empty-mark" />
          <h1>Keep every version of every file</h1>
          <p>
            Pick a folder. From then on, every save from any app, script or AI agent becomes a version you can go back to,
            and a deleted file is never really gone.
          </p>
          <button className="btn primary big" onClick={addFolder}>
            Choose a folder
          </button>
        </main>
      ) : (
        <main className="main">
          <header className="top" data-tauri-drag-region>
            <div className="where" data-tauri-drag-region>
              <h1>{folderName(root.path)}</h1>
              <span className="quiet">{tildify(root.path)}</span>
            </div>
            <div className="ranges" role="radiogroup" aria-label="Time span">
              {RANGES.map((r) => (
                <button key={r.key} role="radio" aria-checked={range === r.key} title={`Last ${r.label}`} onClick={() => setRange(r.key)}>
                  {r.key}
                </button>
              ))}
            </div>
          </header>

          {banner && (
            <div className="burst" role="status">
              <p>
                <strong>
                  {banner.deleted > 0 ? `${plural(banner.deleted, "file")} deleted` : `${plural(banner.paths.length, "file")} changed`}
                </strong>
                {banner.deleted > 0 && banner.paths.length > banner.deleted && <> and {banner.paths.length - banner.deleted} more changed</>} within{" "}
                {Math.max(1, Math.round((banner.end - banner.start) / 1000))} s, {ago(banner.end, now)}.
              </p>
              <div className="burst-actions">
                <button className="btn" onClick={() => setCursor(Math.min(now, banner.end + 500))}>
                  See what happened
                </button>
                <button
                  className="btn primary"
                  onClick={() =>
                    openPlan(`Undo the changes made at ${clock(banner.start)}`, banner.start - 1, { type: "paths", value: banner.paths }, banner.start)
                  }
                >
                  Undo all of it
                </button>
                <button className="icon-btn" aria-label="Dismiss" title="Dismiss" onClick={() => setDismissed(new Set(dismissed).add(banner.start))}>
                  <CloseIcon />
                </button>
              </div>
            </div>
          )}

          <section className="when">
            <div className="readout">
              <div className={`moment${cursor === null ? " live" : ""}`} aria-live="polite">
                {cursor === null ? "Now" : clock(cursor)}
              </div>
              <div className="moment-sub">
                {cursor === null ? (
                  root.paused ? (
                    "Recording is paused for this folder"
                  ) : (
                    <>
                      <span className="pulse" aria-hidden="true" /> Recording every change. Drag the timeline to go back.
                    </>
                  )
                ) : (
                  <>
                    {day(cursor, now)}, {ago(cursor, now)}.{" "}
                    <button className="link" onClick={() => setCursor(null)}>
                      Back to now
                    </button>
                  </>
                )}
              </div>
              {cursor !== null && (
                <button
                  className="btn primary rewind"
                  onClick={() => openPlan(`Rewind ${folderName(root.path)} to ${clock(cursor)}`, cursor, { type: "all" })}
                >
                  Rewind {folderName(root.path)} to {clock(cursor, false)}
                </button>
              )}
            </div>
            <Timeline buckets={buckets} from={from} to={now} cursor={cursor} onCursor={setCursor} bursts={bursts.filter(notable)} onWidth={setWidth} />
          </section>

          <div className="panes">
            <Changes api={api} rootId={root.id} at={at} live={cursor === null} refresh={refresh} selected={selected} onSelect={setSelected} />
            {selected ? (
              <Detail api={api} version={selected} now={now} refresh={refresh} onSelect={setSelected} onRestoreFile={restoreFile} />
            ) : (
              <section className="detail placeholder">
                <p>Pick a change to see what it did.</p>
              </section>
            )}
          </div>
        </main>
      )}

      {sheet && (
        <RestoreSheet
          title={sheet.title}
          plan={sheet.plan}
          busy={busy}
          onCancel={() => setSheet(null)}
          onApply={async () => {
            const ok = await apply(sheet.plan, sheet.scope, `Restored ${plural(sheet.plan.write.length + sheet.plan.delete.length, "file")}.`);
            if (ok && sheet.burst !== undefined) setDismissed((d) => new Set(d).add(sheet.burst!));
          }}
        />
      )}
      {settingsOpen && <SettingsSheet api={api} status={status} onClose={() => setSettingsOpen(false)} />}
      {toast && <ToastView toast={toast} onClose={() => setToast(null)} />}
    </div>
  );
}

function ToastView({ toast, onClose }: { toast: Toast; onClose: () => void }) {
  useEffect(() => {
    const t = window.setTimeout(onClose, toast.undo ? 12_000 : 5_000);
    return () => clearTimeout(t);
  }, [toast, onClose]);
  return (
    <div className="toast" role="status">
      <span>{toast.text}</span>
      {toast.undo && (
        <button className="link" onClick={toast.undo}>
          Undo
        </button>
      )}
      <button className="icon-btn" aria-label="Close" onClick={onClose}>
        <CloseIcon />
      </button>
    </div>
  );
}
