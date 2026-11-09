import { useEffect, useRef, useState } from "react";
import type { Api } from "../api";
import type { Settings, Status } from "../types";
import { bytes } from "../format";

interface Props {
  api: Api;
  status: Status | null;
  onClose: () => void;
}

export function SettingsSheet({ api, status, onClose }: Props) {
  const ref = useRef<HTMLDialogElement>(null);
  const [s, setS] = useState<Settings | null>(null);
  const [auto, setAuto] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    ref.current?.showModal();
    api.settings().then(setS);
    api.autostart().then(setAuto).catch(() => {});
  }, [api]);

  async function save() {
    if (!s) return;
    try {
      await api.setSettings(s);
      onClose();
    } catch (e) {
      setError(String(e));
    }
  }

  const num = (k: keyof Settings, min: number) => (
    <input
      type="number"
      min={min}
      value={s?.[k] ?? ""}
      onChange={(e) => s && setS({ ...s, [k]: Math.max(min, Number(e.target.value) || min) })}
    />
  );

  return (
    <dialog ref={ref} className="sheet settings" onCancel={(e) => (e.preventDefault(), onClose())}>
      <h2>Settings</h2>
      <label className="field">
        <span>Keep history for</span>
        {num("keep_days", 1)}
        <span className="unit">days</span>
      </label>
      <label className="field">
        <span>Use at most</span>
        {num("max_store_mb", 16)}
        <span className="unit">MB of disk</span>
      </label>
      <label className="field">
        <span>Skip files larger than</span>
        {num("max_file_mb", 1)}
        <span className="unit">MB</span>
      </label>
      <p className="quiet small">
        Recent history is kept in full. After a day, one version per file per hour is kept; after a week, one per day.
        The current version of every file is never dropped.
      </p>
      <label className="check">
        <input
          type="checkbox"
          checked={auto}
          onChange={async (e) => {
            const on = e.target.checked;
            setAuto(on);
            await api.setAutostart(on).catch(() => setAuto(!on));
          }}
        />
        Open at login, in the background
      </label>
      {status && (
        <p className="quiet small">
          {status.stats.versions.toLocaleString()} versions, {bytes(status.stats.stored_bytes)} in {status.home}
        </p>
      )}
      <p className="quiet small">
        To skip more files in a folder, list patterns in a <code>.clessidraignore</code> file at its top, in{" "}
        <code>.gitignore</code> syntax.
      </p>
      {error && <p className="error">{error}</p>}
      <div className="sheet-actions">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={save}>
          Save
        </button>
      </div>
    </dialog>
  );
}
