import { useEffect, useState } from "react";
import type { Api } from "../api";
import type { Diff, Preview, Version } from "../types";
import { ago, bytes, clock, day, shortWhen, splitPath } from "../format";
import { FinderIcon } from "./icons";

interface Props {
  api: Api;
  version: Version;
  now: number;
  refresh: number;
  onSelect: (v: Version) => void;
  /** Put this one file back the way it was at `at`. */
  onRestoreFile: (path: string, at: number, label: string) => void;
}

const VERB = { add: "Added", mod: "Changed", del: "Deleted" } as const;

export function Detail({ api, version: v, now, refresh, onSelect, onRestoreFile }: Props) {
  const [history, setHistory] = useState<Version[]>([]);
  const [mode, setMode] = useState<"change" | "now">("change");
  const [diff, setDiff] = useState<Diff | null>(null);
  const [lost, setLost] = useState<Preview | null>(null);

  useEffect(() => {
    let stale = false;
    api.fileVersions(v.root_id, v.path).then((h) => !stale && setHistory(h));
    return () => {
      stale = true;
    };
  }, [api, v.root_id, v.path, refresh]);

  useEffect(() => {
    let stale = false;
    setDiff(null);
    setLost(null);
    if (v.kind === "del") {
      // Show what was lost: the content just before the deletion.
      const before = history.find((h) => h.ts < v.ts || (h.ts === v.ts && h.id < v.id));
      if (before?.hash) api.preview(before.hash).then((p) => !stale && setLost(p));
    } else {
      (mode === "change" ? api.diffVersion(v.id) : api.diffWithDisk(v.id)).then((d) => !stale && setDiff(d));
    }
    return () => {
      stale = true;
    };
  }, [api, v.id, v.kind, v.ts, mode, history]);

  const [dir, name] = splitPath(v.path);
  const current = history[0];
  const isCurrent = current?.id === v.id;
  const existsNow = current ? current.kind !== "del" : true;

  let action: { label: string; at: number } | null = null;
  if (v.kind === "del") action = existsNow ? null : { label: "Bring it back", at: v.ts - 1 };
  else if (!isCurrent) action = { label: "Restore this version", at: v.ts };

  return (
    <section className="detail">
      <header className="detail-head">
        <div className="detail-title">
          <h2>
            <span className="dir">{dir}</span>
            {name}
          </h2>
          <p>
            <span className={`c-${v.kind}`}>{VERB[v.kind]}</span>{" "}
            {day(v.ts, now) === "Today"
              ? `at ${clock(v.ts)}`
              : day(v.ts, now) === "Yesterday"
                ? `yesterday at ${clock(v.ts)}`
                : `on ${day(v.ts, now)} at ${clock(v.ts)}`}
            <span className="quiet">, {ago(v.ts, now)}</span>
            {v.origin === "restore" && <span className="quiet">. Written by a restore.</span>}
          </p>
        </div>
        <div className="detail-actions">
          {existsNow && (
            <button className="btn ghost" onClick={() => api.reveal(v.root_id, v.path)} title="Show in the file manager">
              <FinderIcon /> Reveal
            </button>
          )}
          {action ? (
            <button className="btn primary" onClick={() => onRestoreFile(v.path, action!.at, action!.label)}>
              {action.label}
            </button>
          ) : (
            <span className="current-note">{v.kind === "del" ? "It exists again now" : "This is how it is now"}</span>
          )}
        </div>
      </header>

      {v.kind !== "del" && (
        <div className="segmented" role="radiogroup" aria-label="Compare">
          <button role="radio" aria-checked={mode === "change"} onClick={() => setMode("change")}>
            What changed here
          </button>
          <button role="radio" aria-checked={mode === "now"} onClick={() => setMode("now")} disabled={isCurrent}>
            Compared with now
          </button>
        </div>
      )}

      <div className="detail-body">
        {v.kind === "del" ? (
          lost ? (
            lost.text !== null ? (
              <>
                <p className="body-note">The file as it was just before it was deleted:</p>
                <pre className="preview">{lost.text}</pre>
              </>
            ) : (
              <p className="body-note">A binary file of {bytes(lost.size)}. Bring it back to open it.</p>
            )
          ) : (
            <p className="body-note quiet">Loading…</p>
          )
        ) : diff ? (
          <DiffView diff={diff} />
        ) : (
          <p className="body-note quiet">Loading…</p>
        )}
      </div>

      {history.length > 1 && (
        <div className="history">
          <h3>Every version of this file</h3>
          <ol>
            {history.slice(0, 60).map((h) => (
              <li key={h.id}>
                <button className={h.id === v.id ? "active" : ""} onClick={() => onSelect(h)}>
                  <span className={`dot k-${h.kind}`} />
                  <span>{shortWhen(h.ts, now)}</span>
                  <span className="quiet">{h.kind === "del" ? "deleted" : bytes(h.size)}</span>
                </button>
              </li>
            ))}
          </ol>
        </div>
      )}
    </section>
  );
}

function DiffView({ diff }: { diff: Diff }) {
  if (!diff.text) return <p className="body-note">A binary file: there is no text to compare. Restore it to open it.</p>;
  if (!diff.lines.length) return <p className="body-note quiet">No differences.</p>;
  return (
    <div className="diff" role="table" aria-label={`${diff.added} lines added, ${diff.removed} removed`}>
      <p className="diff-sum">
        <span className="c-add">+{diff.added}</span> <span className="c-del">−{diff.removed}</span>
      </p>
      {diff.lines.map((l, i) =>
        l.kind === "gap" ? (
          <div key={i} className="dl gap" role="row">
            ⋯
          </div>
        ) : (
          <div key={i} className={`dl ${l.kind}`} role="row">
            <span className="ln">{l.old_line ?? ""}</span>
            <span className="ln">{l.new_line ?? ""}</span>
            <code>{l.text || " "}</code>
          </div>
        ),
      )}
    </div>
  );
}
