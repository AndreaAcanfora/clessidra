// Mirrors the serialized types of clessidra-core.

export type Kind = "add" | "mod" | "del";
export type Origin = "live" | "baseline" | "restore" | "catchup";

export interface Root {
  id: number;
  path: string;
  added_at: number;
  paused: boolean;
  last_change: number | null;
}

export interface Version {
  id: number;
  root_id: number;
  path: string;
  ts: number;
  kind: Kind;
  origin: Origin;
  hash: string | null;
  size: number;
  mode: number;
  mtime: number;
}

export interface Bucket {
  start: number;
  end: number;
  added: number;
  modified: number;
  deleted: number;
}

export interface Burst {
  root_id: number;
  start: number;
  end: number;
  added: number;
  modified: number;
  deleted: number;
  paths: string[];
}

export interface DiffLine {
  kind: "context" | "added" | "removed" | "gap";
  text: string;
  old_line: number | null;
  new_line: number | null;
}

export interface Diff {
  text: boolean;
  added: number;
  removed: number;
  lines: DiffLine[];
}

export interface Preview {
  text: string | null;
  truncated: boolean;
  size: number;
}

export type Scope = { type: "all" } | { type: "prefix"; value: string } | { type: "paths"; value: string[] };

export interface RestoreItem {
  path: string;
  hash: string;
  size: number;
  mode: number;
  exists: boolean;
}

export interface RestorePlan {
  root_id: number;
  at: number;
  write: RestoreItem[];
  delete: string[];
}

export interface RestoreReport {
  written: number;
  deleted: number;
  failed: [string, string][];
}

export interface Stats {
  roots: number;
  versions: number;
  blobs: number;
  stored_bytes: number;
  original_bytes: number;
}

export interface Status {
  watching: boolean;
  error: string | null;
  stats: Stats;
  home: string;
}

export interface Settings {
  max_store_mb: number;
  keep_days: number;
  max_file_mb: number;
}
