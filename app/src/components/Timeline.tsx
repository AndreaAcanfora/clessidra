import { useEffect, useMemo, useRef, useState } from "react";
import type { Bucket, Burst } from "../types";
import { clock, shortWhen } from "../format";

interface Props {
  buckets: Bucket[];
  from: number;
  to: number;
  /** The selected moment; null means now. */
  cursor: number | null;
  onCursor: (t: number | null) => void;
  bursts: Burst[];
  onWidth: (w: number) => void;
}

const H = 96;
const BAR_TOP = 18;
const BAR_BOTTOM = H - 22;

export function Timeline({ buckets, from, to, cursor, onCursor, bursts, onWidth }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(800);
  const [hover, setHover] = useState<number | null>(null);
  const dragging = useRef(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => {
      const width = Math.round(e.contentRect.width);
      setW(width);
      onWidth(width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [onWidth]);

  const span = to - from;
  const xOf = (t: number) => ((t - from) / span) * w;
  const tOf = (x: number) => from + (Math.min(Math.max(x, 0), w) / w) * span;

  const max = useMemo(() => Math.max(1, ...buckets.map((b) => b.added + b.modified + b.deleted)), [buckets]);
  const bw = buckets.length ? w / buckets.length : w;

  const ticks = useMemo(() => {
    const n = Math.max(2, Math.min(7, Math.floor(w / 130)));
    return Array.from({ length: n }, (_, i) => from + ((i + 0.5) * span) / n);
  }, [from, span, w]);

  function pick(clientX: number) {
    const rect = ref.current!.getBoundingClientRect();
    const x = clientX - rect.left;
    // The last few pixels mean "now": that is where the cursor rests when nothing is selected.
    onCursor(x >= w - 6 ? null : Math.round(tOf(x)));
  }

  function onKey(e: React.KeyboardEvent) {
    const step = buckets.length ? span / buckets.length : span / 100;
    const cur = cursor ?? to;
    if (e.key === "ArrowLeft") onCursor(Math.max(from, Math.round(cur - (e.shiftKey ? step * 10 : step))));
    else if (e.key === "ArrowRight") {
      const next = cur + (e.shiftKey ? step * 10 : step);
      onCursor(next >= to ? null : Math.round(next));
    } else if (e.key === "End" || e.key === "Escape") onCursor(null);
    else if (e.key === "Home") onCursor(from);
    else return;
    e.preventDefault();
  }

  const cx = cursor === null ? w : xOf(cursor);
  const hoverBucket = hover === null ? null : buckets[Math.floor(hover / bw)];

  return (
    <div
      className="timeline"
      ref={ref}
      tabIndex={0}
      role="slider"
      aria-label="Moment in time"
      aria-valuemin={from}
      aria-valuemax={to}
      aria-valuenow={cursor ?? to}
      aria-valuetext={cursor === null ? "Now" : `${shortWhen(cursor)}`}
      onKeyDown={onKey}
      onPointerDown={(e) => {
        dragging.current = true;
        (e.target as Element).setPointerCapture?.(e.pointerId);
        pick(e.clientX);
      }}
      onPointerMove={(e) => {
        const rect = ref.current!.getBoundingClientRect();
        setHover(e.clientX - rect.left);
        if (dragging.current) pick(e.clientX);
      }}
      onPointerUp={() => (dragging.current = false)}
      onPointerLeave={() => setHover(null)}
    >
      <svg width={w} height={H} aria-hidden="true">
        <line className="tl-base" x1={0} x2={w} y1={BAR_BOTTOM + 0.5} y2={BAR_BOTTOM + 0.5} />
        {buckets.map((b, i) => {
          const total = b.added + b.modified + b.deleted;
          if (!total) return null;
          const h = (0.16 + 0.84 * (Math.log1p(total) / Math.log1p(max))) * (BAR_BOTTOM - BAR_TOP);
          const x = i * bw + 0.5;
          const width = Math.max(1, bw - 1);
          const past = cursor === null || b.start <= cursor;
          let y = BAR_BOTTOM;
          const parts: [number, string][] = [
            [b.deleted, "del"],
            [b.added, "add"],
            [b.modified, "mod"],
          ];
          return (
            <g key={i} className={past ? "" : "future"}>
              {parts.map(([n, k]) => {
                if (!n) return null;
                const ph = (n / total) * h;
                y -= ph;
                return <rect key={k} className={`tl-${k}`} x={x} y={y} width={width} height={ph} rx={Math.min(1.5, width / 2)} />;
              })}
            </g>
          );
        })}
        {bursts.map((b) => {
          const x = xOf(b.start);
          if (x < 0 || x > w) return null;
          return <path key={b.start} className="tl-burst" d={`M${x - 5} 2 L${x + 5} 2 L${x} 9 Z`} />;
        })}
        {ticks.map((t) => (
          <text key={t} className="tl-tick" x={xOf(t)} y={H - 6} textAnchor="middle">
            {span <= 36 * 3_600_000 ? clock(t, false) : shortWhen(t)}
          </text>
        ))}
        {hover !== null && !dragging.current && <line className="tl-hover" x1={hover} x2={hover} y1={BAR_TOP - 6} y2={BAR_BOTTOM} />}
        <g className={cursor === null ? "tl-cursor live" : "tl-cursor"} transform={`translate(${Math.min(Math.max(cx, 1), w - 1)} 0)`}>
          <line x1={0} x2={0} y1={10} y2={BAR_BOTTOM + 4} />
          <circle cx={0} cy={10} r={5} />
        </g>
      </svg>
      {hover !== null && hoverBucket && (
        <div className="tl-tip" style={{ left: Math.min(Math.max(hover, 70), w - 70) }}>
          <strong>{span <= 36 * 3_600_000 ? clock(tOf(hover), false) : shortWhen(tOf(hover))}</strong>
          {hoverBucket.added + hoverBucket.modified + hoverBucket.deleted > 0 ? (
            <span>
              {hoverBucket.added > 0 && <em className="c-add">+{hoverBucket.added}</em>}
              {hoverBucket.modified > 0 && <em className="c-mod">~{hoverBucket.modified}</em>}
              {hoverBucket.deleted > 0 && <em className="c-del">−{hoverBucket.deleted}</em>}
            </span>
          ) : (
            <span className="quiet">no changes</span>
          )}
        </div>
      )}
    </div>
  );
}
