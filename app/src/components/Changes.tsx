import { useEffect, useMemo, useState } from "react";
import type { Api } from "../api";
import type { Version } from "../types";
import { clock, day, splitPath } from "../format";

interface Props {
  api: Api;
  rootId: number;
  /** Show what happened up to this moment. */
  at: number;
  live: boolean;
  refresh: number;
  selected: Version | null;
  onSelect: (v: Version) => void;
}

const PAGE = 150;

export function Changes({ api, rootId, at, live, refresh, selected, onSelect }: Props) {
  const [tab, setTab] = useState<"changes" | "files">("changes");
  return (
    <section className="list-pane">
      <div className="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "changes"} onClick={() => setTab("changes")}>
          Changes
        </button>
        <button role="tab" aria-selected={tab === "files"} onClick={() => setTab("files")}>
          {live ? "Files now" : `Files at ${clock(at, false)}`}
        </button>
      </div>
      {tab === "changes" ? (
        <ChangeList api={api} rootId={rootId} at={at} refresh={refresh} selected={selected} onSelect={onSelect} />
      ) : (
        <FileList api={api} rootId={rootId} at={at} refresh={refresh} selected={selected} onSelect={onSelect} />
      )}
    </section>
  );
}

const GLYPH = { add: "+", mod: "~", del: "−" } as const;
const VERB = { add: "Added", mod: "Changed", del: "Deleted" } as const;

function Row({ v, active, onSelect, time, plain }: { v: Version; active: boolean; onSelect: (v: Version) => void; time?: string; plain?: boolean }) {
  const [dir, name] = splitPath(v.path);
  return (
    <button className={`row k-${plain ? "file" : v.kind}${active ? " active" : ""}`} onClick={() => onSelect(v)} title={plain ? v.path : `${VERB[v.kind]} ${v.path}`}>
      {!plain && (
        <span className="glyph" aria-label={VERB[v.kind]}>
          {GLYPH[v.kind]}
        </span>
      )}
      <span className="path">
        <span className="dir">{dir}</span>
        <span className="name">{name}</span>
      </span>
      {time && <time>{time}</time>}
      {v.origin === "restore" && <span className="tag">restored</span>}
    </button>
  );
}

function ChangeList({ api, rootId, at, refresh, selected, onSelect }: Omit<Props, "live">) {
  const [items, setItems] = useState<Version[] | null>(null);
  const [more, setMore] = useState(false);

  useEffect(() => {
    let stale = false;
    api.events(rootId, at, PAGE).then((v) => {
      if (stale) return;
      setItems(v);
      setMore(v.length === PAGE);
    });
    return () => {
      stale = true;
    };
  }, [api, rootId, at, refresh]);

  async function loadMore() {
    if (!items?.length) return;
    const last = items[items.length - 1];
    const next = await api.events(rootId, at, PAGE, [last.ts, last.id]);
    setItems([...items, ...next]);
    setMore(next.length === PAGE);
  }

  const groups = useMemo(() => {
    const out: { label: string; rows: Version[] }[] = [];
    for (const v of items ?? []) {
      const label = day(v.ts);
      if (out[out.length - 1]?.label !== label) out.push({ label, rows: [] });
      out[out.length - 1].rows.push(v);
    }
    return out;
  }, [items]);

  if (items === null) return <div className="list-empty" />;
  if (!items.length)
    return (
      <div className="list-empty">
        <p>Nothing has changed here yet.</p>
        <p className="quiet">Every save from now on, from any app, shows up in this list.</p>
      </div>
    );
  return (
    <div className="list" role="list">
      {groups.map((g) => (
        <div key={g.label} className="group">
          <h3>{g.label}</h3>
          {g.rows.map((v) => (
            <Row key={v.id} v={v} active={selected?.id === v.id} onSelect={onSelect} time={clock(v.ts, false)} />
          ))}
        </div>
      ))}
      {more && (
        <button className="more" onClick={loadMore}>
          Show earlier changes
        </button>
      )}
    </div>
  );
}

function FileList({ api, rootId, at, refresh, selected, onSelect }: Omit<Props, "live">) {
  const [files, setFiles] = useState<Version[] | null>(null);
  const [q, setQ] = useState("");
  useEffect(() => {
    let stale = false;
    api.treeAt(rootId, at).then((f) => !stale && setFiles(f));
    return () => {
      stale = true;
    };
  }, [api, rootId, at, refresh]);
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (files ?? []).filter((f) => !needle || f.path.toLowerCase().includes(needle));
  }, [files, q]);
  return (
    <div className="list">
      <input className="filter" type="search" placeholder={`Filter ${files?.length ?? ""} files`} value={q} onChange={(e) => setQ(e.target.value)} aria-label="Filter files" />
      {shown.slice(0, 2000).map((v) => (
        <Row key={v.path} v={v} plain active={selected?.path === v.path} onSelect={onSelect} />
      ))}
      {files && !shown.length && <p className="list-empty quiet">No files match.</p>}
    </div>
  );
}
