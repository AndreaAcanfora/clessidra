import { useEffect, useRef } from "react";
import type { RestorePlan } from "../types";
import { plural, splitPath } from "../format";

interface Props {
  title: string;
  plan: RestorePlan;
  busy: boolean;
  onCancel: () => void;
  onApply: () => void;
}

export function RestoreSheet({ title, plan, busy, onCancel, onApply }: Props) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  const back = plan.write.filter((w) => !w.exists);
  const rewrite = plan.write.filter((w) => w.exists);
  const total = plan.write.length + plan.delete.length;
  const parts = [
    back.length && `${plural(back.length, "file")} come${back.length === 1 ? "s" : ""} back`,
    rewrite.length && `${plural(rewrite.length, "file")} ${rewrite.length === 1 ? "is" : "are"} rewritten`,
    plan.delete.length && `${plural(plan.delete.length, "file")} ${plan.delete.length === 1 ? "is" : "are"} removed`,
  ].filter(Boolean);

  return (
    <dialog ref={ref} className="sheet" onCancel={(e) => (e.preventDefault(), onCancel())}>
      <h2>{title}</h2>
      {total === 0 ? (
        <p>Nothing to do: the files already look like that.</p>
      ) : (
        <>
          <p>{parts.join(", ")}.</p>
          <ul className="plan">
            {back.map((w) => (
              <PlanRow key={w.path} path={w.path} kind="add" verb="comes back" />
            ))}
            {rewrite.map((w) => (
              <PlanRow key={w.path} path={w.path} kind="mod" verb="rewritten" />
            ))}
            {plan.delete.map((p) => (
              <PlanRow key={p} path={p} kind="del" verb="removed" />
            ))}
          </ul>
          <p className="quiet small">Whatever this overwrites stays in the history, so you can undo it too.</p>
        </>
      )}
      <div className="sheet-actions">
        <button className="btn" onClick={onCancel} disabled={busy}>
          Cancel
        </button>
        {total > 0 && (
          <button className="btn primary" onClick={onApply} disabled={busy} autoFocus>
            {busy ? "Restoring…" : `Restore ${plural(total, "file")}`}
          </button>
        )}
      </div>
    </dialog>
  );
}

function PlanRow({ path, kind, verb }: { path: string; kind: string; verb: string }) {
  const [dir, name] = splitPath(path);
  return (
    <li className={`k-${kind}`}>
      <span className="path">
        <span className="dir">{dir}</span>
        <span className="name">{name}</span>
      </span>
      <span className="verb">{verb}</span>
    </li>
  );
}
