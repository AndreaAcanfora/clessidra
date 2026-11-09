import type { Root, Status } from "../types";
import { ago, bytes, folderName, tildify } from "../format";
import { Hourglass, PauseIcon, PlayIcon, PlusIcon, GearIcon } from "./icons";

interface Props {
  roots: Root[];
  selected: number | null;
  status: Status | null;
  now: number;
  onSelect: (id: number) => void;
  onAdd: () => void;
  onPause: (r: Root) => void;
  onSettings: () => void;
}

export function Sidebar({ roots, selected, status, now, onSelect, onAdd, onPause, onSettings }: Props) {
  return (
    <aside className="sidebar">
      <div className="brand" data-tauri-drag-region>
        <Hourglass className="brand-mark" />
        <span>clessidra</span>
      </div>

      {status?.error && (
        <p className="side-warning" role="alert">
          Not recording: {status.error}
        </p>
      )}

      <nav className="roots" aria-label="Watched folders">
        {roots.map((r) => (
          <div key={r.id} className={`root${r.id === selected ? " selected" : ""}${r.paused ? " paused" : ""}`}>
            <button className="root-main" onClick={() => onSelect(r.id)} aria-current={r.id === selected}>
              <span className="root-name">{folderName(r.path)}</span>
              <span className="root-meta">
                {r.paused ? "Paused" : r.last_change ? `Changed ${ago(r.last_change, now)}` : "No changes yet"}
              </span>
            </button>
            <button
              className="icon-btn root-pause"
              title={r.paused ? `Resume recording ${tildify(r.path)}` : `Pause recording ${tildify(r.path)}`}
              aria-label={r.paused ? "Resume" : "Pause"}
              onClick={() => onPause(r)}
            >
              {r.paused ? <PlayIcon /> : <PauseIcon />}
            </button>
          </div>
        ))}
        <button className="add-root" onClick={onAdd}>
          <PlusIcon /> Add a folder
        </button>
      </nav>

      <footer className="side-foot">
        {status && (
          <p className="usage" title={status.home}>
            <strong>{bytes(status.stats.stored_bytes)}</strong> on disk for {bytes(status.stats.original_bytes)} of history
          </p>
        )}
        <button className="icon-btn" onClick={onSettings} aria-label="Settings" title="Settings">
          <GearIcon />
        </button>
      </footer>
    </aside>
  );
}
