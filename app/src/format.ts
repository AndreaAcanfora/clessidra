const two = (n: number) => String(n).padStart(2, "0");

/** 14:42:07 */
export function clock(ts: number, seconds = true): string {
  const d = new Date(ts);
  return `${two(d.getHours())}:${two(d.getMinutes())}${seconds ? `:${two(d.getSeconds())}` : ""}`;
}

function sameDay(a: Date, b: Date) {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** "Today", "Yesterday", or "Tuesday 10 November". */
export function day(ts: number, now = Date.now()): string {
  const d = new Date(ts);
  const n = new Date(now);
  if (sameDay(d, n)) return "Today";
  const y = new Date(now - 86_400_000);
  if (sameDay(d, y)) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
}

/** "14:42" today, "Mon 14:42" this week, "3 Nov 14:42" before that. */
export function shortWhen(ts: number, now = Date.now()): string {
  const d = new Date(ts);
  if (sameDay(d, new Date(now))) return clock(ts, false);
  if (now - ts < 6 * 86_400_000) return `${d.toLocaleDateString(undefined, { weekday: "short" })} ${clock(ts, false)}`;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${clock(ts, false)}`;
}

export function ago(ts: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s} seconds ago`;
  const m = Math.round(s / 60);
  if (m < 60) return m === 1 ? "a minute ago" : `${m} minutes ago`;
  const h = Math.round(m / 60);
  if (h < 24) return h === 1 ? "an hour ago" : `${h} hours ago`;
  const d = Math.round(h / 24);
  return d === 1 ? "yesterday" : `${d} days ago`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

/** ["src/components/", "Map.tsx"] */
export function splitPath(p: string): [string, string] {
  const i = p.lastIndexOf("/");
  return i < 0 ? ["", p] : [p.slice(0, i + 1), p.slice(i + 1)];
}

export function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** "~/code" for a path under the home folder. */
export function tildify(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
}

export function plural(n: number, one: string, many = one + "s"): string {
  return `${n} ${n === 1 ? one : many}`;
}
