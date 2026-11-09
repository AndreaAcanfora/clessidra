# Changelog

All notable changes to clessidra are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-11-09

The first release.

- **Engine**: every version of every file in the watched folders, in an SQLite index plus
  zstd-compressed, content-addressed blobs. Restores are planned before they are applied and
  recorded as new versions, so they can be undone. Old history is thinned out by age, dropped
  after `keep_days` and evicted oldest-first above `max_store_mb`.
- **Bursts**: ten or more files touched within a second are grouped, and can be undone in one go.
- **CLI**: `add`, `remove`, `list`, `daemon`, `log`, `versions`, `show`, `diff`, `restore`,
  `undo`, `bursts`, `status`, `prune`, `config`.
- **Desktop app**: timeline you can drag back to any moment, list of changes with diffs,
  one-click undo of a burst with a notification when one happens, tray icon, start at login.
