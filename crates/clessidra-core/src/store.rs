use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::diff::{self, Diff};
use crate::filter::Filter;
use crate::now_ms;
use crate::paths::{absolute, relative, within};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS roots (
    id       INTEGER PRIMARY KEY,
    path     TEXT    NOT NULL UNIQUE,
    added_at INTEGER NOT NULL,
    paused   INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS versions (
    id      INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    path    TEXT    NOT NULL,
    ts      INTEGER NOT NULL,
    kind    INTEGER NOT NULL,
    origin  INTEGER NOT NULL,
    hash    TEXT,
    size    INTEGER NOT NULL DEFAULT 0,
    mode    INTEGER NOT NULL DEFAULT 0,
    mtime   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS versions_by_path ON versions(root_id, path, ts);
CREATE INDEX IF NOT EXISTS versions_by_time ON versions(root_id, ts);
CREATE INDEX IF NOT EXISTS versions_by_hash ON versions(hash);
CREATE TABLE IF NOT EXISTS blobs (
    hash   TEXT    PRIMARY KEY,
    size   INTEGER NOT NULL,
    stored INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

const VERSION_COLS: &str = "id, root_id, path, ts, kind, origin, hash, size, mode, mtime";

/// What happened to a file.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Add,
    Mod,
    Del,
}

impl Kind {
    fn code(self) -> i64 {
        match self {
            Kind::Add => 0,
            Kind::Mod => 1,
            Kind::Del => 2,
        }
    }
    fn from_code(c: i64) -> Kind {
        match c {
            0 => Kind::Add,
            1 => Kind::Mod,
            _ => Kind::Del,
        }
    }
}

/// How a version came to be recorded.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Seen by the watcher as it happened.
    Live,
    /// The first snapshot taken when a folder was added.
    Baseline,
    /// Written by a restore.
    Restore,
    /// Found by a rescan: the change happened while nothing was watching.
    CatchUp,
}

impl Origin {
    fn code(self) -> i64 {
        match self {
            Origin::Live => 0,
            Origin::Baseline => 1,
            Origin::Restore => 2,
            Origin::CatchUp => 3,
        }
    }
    fn from_code(c: i64) -> Origin {
        match c {
            1 => Origin::Baseline,
            2 => Origin::Restore,
            3 => Origin::CatchUp,
            _ => Origin::Live,
        }
    }
}

/// A watched folder.
#[derive(Serialize, Clone, Debug)]
pub struct Root {
    pub id: i64,
    pub path: String,
    pub added_at: i64,
    pub paused: bool,
    /// Time of the most recent change, if any (the initial snapshot does not count).
    pub last_change: Option<i64>,
}

impl Root {
    pub fn dir(&self) -> &Path {
        Path::new(&self.path)
    }
}

/// One recorded state of one file.
#[derive(Serialize, Clone, Debug)]
pub struct Version {
    pub id: i64,
    pub root_id: i64,
    /// Relative to the root, `/`-separated.
    pub path: String,
    /// When it was recorded (ms since the epoch).
    pub ts: i64,
    pub kind: Kind,
    pub origin: Origin,
    /// Content hash; `None` for deletions.
    pub hash: Option<String>,
    pub size: i64,
    pub mode: u32,
    pub mtime: i64,
}

impl Version {
    fn from_row(r: &Row) -> rusqlite::Result<Version> {
        Ok(Version {
            id: r.get(0)?,
            root_id: r.get(1)?,
            path: r.get(2)?,
            ts: r.get(3)?,
            kind: Kind::from_code(r.get(4)?),
            origin: Origin::from_code(r.get(5)?),
            hash: r.get(6)?,
            size: r.get(7)?,
            mode: r.get::<_, i64>(8)? as u32,
            mtime: r.get(9)?,
        })
    }
}

/// Live file contents at a moment in time.
pub type FileAt = Version;

/// Activity in one slice of a timeline.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Bucket {
    pub start: i64,
    pub end: i64,
    pub added: i64,
    pub modified: i64,
    pub deleted: i64,
}

/// Many files changed within a few seconds of each other: usually one program (a script, an
/// agent, a sync client, a bad `git checkout`) doing one thing. It can be undone as a unit.
#[derive(Serialize, Clone, Debug)]
pub struct Burst {
    pub root_id: i64,
    pub start: i64,
    pub end: i64,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    /// Every path touched, in the order first touched.
    pub paths: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Stats {
    pub roots: i64,
    pub versions: i64,
    pub blobs: i64,
    /// Bytes on disk, after deduplication and compression.
    pub stored_bytes: i64,
    /// What the same versions would take as plain copies.
    pub original_bytes: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Old versions are thinned out and then dropped, oldest first, to stay under this size.
    pub max_store_mb: i64,
    /// Versions older than this are dropped (the current version of each file is always kept).
    pub keep_days: i64,
    /// Files larger than this are not versioned.
    pub max_file_mb: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_store_mb: 2048,
            keep_days: 30,
            max_file_mb: 32,
        }
    }
}

/// Which files a restore touches.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Scope {
    /// The whole folder.
    All,
    /// One file, or everything under one directory.
    Prefix(String),
    /// Exactly these files.
    Paths(Vec<String>),
}

impl Scope {
    fn prefix(&self) -> &str {
        match self {
            Scope::Prefix(p) => p,
            _ => "",
        }
    }
    fn contains(&self, path: &str) -> bool {
        match self {
            Scope::All => true,
            Scope::Prefix(p) => within(path, p),
            Scope::Paths(ps) => ps.iter().any(|p| p == path),
        }
    }
}

/// What a restore would do. Nothing happens until it is passed to [`Store::apply_restore`].
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestorePlan {
    pub root_id: i64,
    pub at: i64,
    /// Files to (re)write with their content at `at`.
    pub write: Vec<RestoreItem>,
    /// Files that did not exist at `at` and will be removed.
    pub delete: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestoreItem {
    pub path: String,
    pub hash: String,
    pub size: i64,
    pub mode: u32,
    /// Whether some version of the file exists on disk right now.
    pub exists: bool,
}

impl RestorePlan {
    pub fn is_empty(&self) -> bool {
        self.write.is_empty() && self.delete.is_empty()
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct RestoreReport {
    pub written: usize,
    pub deleted: usize,
    pub failed: Vec<(String, String)>,
}

pub struct Store {
    home: PathBuf,
    conn: Mutex<Connection>,
    settings: Mutex<Settings>,
    filters: Mutex<HashMap<i64, Arc<Filter>>>,
}

impl Store {
    /// Where the store lives: `$CLESSIDRA_HOME`, or the platform's application data directory.
    pub fn default_home() -> Result<PathBuf> {
        if let Some(h) = std::env::var_os("CLESSIDRA_HOME") {
            return Ok(PathBuf::from(h));
        }
        directories::ProjectDirs::from("io.github", "andreaacanfora", "clessidra")
            .map(|d| d.data_dir().to_path_buf())
            .ok_or_else(|| anyhow!("cannot find a home directory for the store"))
    }

    pub fn open_default() -> Result<Store> {
        Store::open(Store::default_home()?)
    }

    pub fn open(home: impl Into<PathBuf>) -> Result<Store> {
        let home = home.into();
        fs::create_dir_all(home.join("objects")).with_context(|| format!("creating {}", home.display()))?;
        let home = home.canonicalize()?;
        let conn = Connection::open(home.join("index.sqlite"))?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch(SCHEMA)?;
        let settings = conn
            .query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Store {
            home,
            conn: Mutex::new(conn),
            settings: Mutex::new(settings),
            filters: Mutex::new(HashMap::new()),
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_settings(&self, s: Settings) -> Result<()> {
        if s.max_store_mb < 16 || s.keep_days < 1 || s.max_file_mb < 1 {
            bail!("settings out of range");
        }
        self.db().execute(
            "INSERT INTO settings(key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(&s)?],
        )?;
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = s;
        Ok(())
    }

    // ---- roots ----------------------------------------------------------------------------

    pub fn roots(&self) -> Result<Vec<Root>> {
        let db = self.db();
        let mut st = db.prepare(
            "SELECT r.id, r.path, r.added_at, r.paused,
                    (SELECT MAX(ts) FROM versions v WHERE v.root_id = r.id AND v.origin != 1)
             FROM roots r ORDER BY r.path",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Root {
                id: r.get(0)?,
                path: r.get(1)?,
                added_at: r.get(2)?,
                paused: r.get::<_, i64>(3)? != 0,
                last_change: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn root(&self, id: i64) -> Result<Root> {
        self.roots()?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow!("no watched folder with id {id}"))
    }

    /// The watched folder containing `path`, and `path` relative to it.
    pub fn root_for(&self, path: &Path) -> Result<Option<(Root, String)>> {
        let abs = canonical_or_parent(path);
        Ok(self
            .roots()?
            .into_iter()
            .filter_map(|r| relative(r.dir(), &abs).map(|rel| (r, rel)))
            .max_by_key(|(r, _)| r.path.len()))
    }

    /// Start keeping history for a folder. The caller should follow up with
    /// [`Store::snapshot`] to take the initial copy of every file.
    pub fn add_root(&self, path: &Path) -> Result<Root> {
        let dir = path
            .canonicalize()
            .with_context(|| format!("{} does not exist", path.display()))?;
        if !dir.is_dir() {
            bail!("{} is not a folder", dir.display());
        }
        if dir.parent().is_none() {
            bail!("refusing to watch the whole filesystem");
        }
        // The store may live inside a watched folder (it is filtered out), not the other way round.
        if dir.starts_with(&self.home) {
            bail!("{} is clessidra's own store", dir.display());
        }
        for r in self.roots()? {
            if dir.starts_with(r.dir()) {
                bail!("{} is already inside the watched folder {}", dir.display(), r.path);
            }
            if r.dir().starts_with(&dir) {
                bail!(
                    "{} contains the watched folder {}; remove that one first",
                    dir.display(),
                    r.path
                );
            }
        }
        let path = dir.to_string_lossy().into_owned();
        self.db().execute(
            "INSERT INTO roots(path, added_at) VALUES (?1, ?2)",
            params![path, now_ms()],
        )?;
        let id = self.db().last_insert_rowid();
        self.root(id)
    }

    /// Stop watching a folder and forget its history.
    pub fn remove_root(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM roots WHERE id = ?1", [id])?;
        self.filters.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        self.gc()?;
        Ok(())
    }

    pub fn set_paused(&self, id: i64, paused: bool) -> Result<()> {
        self.db()
            .execute("UPDATE roots SET paused = ?2 WHERE id = ?1", params![id, paused as i64])?;
        Ok(())
    }

    pub fn filter(&self, root: &Root) -> Arc<Filter> {
        let mut filters = self.filters.lock().unwrap_or_else(|e| e.into_inner());
        filters
            .entry(root.id)
            .or_insert_with(|| Arc::new(Filter::new(root.dir(), Some(&self.home))))
            .clone()
    }

    /// Re-read `.clessidraignore` next time the filter is needed.
    pub fn invalidate_filter(&self, root_id: i64) {
        self.filters.lock().unwrap_or_else(|e| e.into_inner()).remove(&root_id);
    }

    // ---- recording ------------------------------------------------------------------------

    /// Take the initial copy of every file in a newly added folder.
    pub fn snapshot(&self, root: &Root) -> Result<usize> {
        Ok(self.scan(root, "", Origin::Baseline)?.len())
    }

    /// Compare a folder with its history and record whatever changed while nobody was watching.
    pub fn catch_up(&self, root: &Root) -> Result<Vec<Version>> {
        self.scan(root, "", Origin::CatchUp)
    }

    /// Record the current state of one path (a file, a directory or something that is gone).
    /// Returns the versions created; none when nothing changed.
    pub fn record(&self, root: &Root, rel: &str, origin: Origin) -> Result<Vec<Version>> {
        if rel.is_empty() {
            return self.scan(root, "", origin);
        }
        let abs = absolute(root.dir(), rel);
        let filter = self.filter(root);
        match fs::symlink_metadata(&abs) {
            Ok(m) if m.is_dir() => {
                if filter.ignores(&abs, true) {
                    return Ok(Vec::new());
                }
                self.scan(root, rel, origin)
            }
            Ok(m) if m.is_file() => {
                if filter.ignores(&abs, false) {
                    return Ok(Vec::new());
                }
                Ok(self.record_file(root, rel, &abs, &m, origin)?.into_iter().collect())
            }
            Ok(_) => Ok(Vec::new()), // symlinks, sockets, fifos
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.record_gone(root, rel, origin),
            Err(e) => Err(e.into()),
        }
    }

    fn record_file(
        &self,
        root: &Root,
        rel: &str,
        abs: &Path,
        meta: &fs::Metadata,
        origin: Origin,
    ) -> Result<Option<Version>> {
        let size = meta.len() as i64;
        if size > self.settings().max_file_mb * 1024 * 1024 {
            return Ok(None);
        }
        let mtime = mtime_ms(meta);
        let mode = mode_of(meta);
        let last = self.latest(root.id, rel)?;
        let alive = last.as_ref().filter(|v| v.kind != Kind::Del);
        if let Some(l) = alive {
            if l.size == size && l.mtime == mtime && l.mode == mode {
                return Ok(None);
            }
        }
        let bytes = match fs::read(abs) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(self.record_gone(root, rel, origin)?.into_iter().next());
            }
            Err(e) => return Err(e.into()),
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();
        if let Some(l) = alive {
            if l.hash.as_deref() == Some(hash.as_str()) && l.mode == mode {
                // Touched but not changed: remember the new mtime so we do not read it again.
                self.db()
                    .execute("UPDATE versions SET mtime = ?2 WHERE id = ?1", params![l.id, mtime])?;
                return Ok(None);
            }
        }
        let now = now_ms();
        let ts = match origin {
            // A change found by a scan happened at its mtime, as far as we can tell.
            Origin::CatchUp | Origin::Baseline => mtime.min(now),
            _ => now,
        };
        let ts = ts.max(last.as_ref().map_or(0, |l| l.ts + 1));
        let kind = if alive.is_some() { Kind::Mod } else { Kind::Add };
        let compressed = zstd::encode_all(&bytes[..], 3)?;
        let blob = self.blob_path(&hash);
        let db = self.db();
        // The blob is written while holding the index lock so a concurrent garbage collection
        // cannot remove it between the existence check and the version that references it.
        if !blob.exists() {
            write_atomically(&blob, &compressed)?;
        }
        db.execute(
            "INSERT OR IGNORE INTO blobs(hash, size, stored) VALUES (?1, ?2, ?3)",
            params![hash, size, compressed.len() as i64],
        )?;
        db.execute(
            "INSERT INTO versions(root_id, path, ts, kind, origin, hash, size, mode, mtime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                root.id,
                rel,
                ts,
                kind.code(),
                origin.code(),
                hash,
                size,
                mode as i64,
                mtime
            ],
        )?;
        let id = db.last_insert_rowid();
        Ok(Some(Version {
            id,
            root_id: root.id,
            path: rel.to_string(),
            ts,
            kind,
            origin,
            hash: Some(hash),
            size,
            mode,
            mtime,
        }))
    }

    /// `rel` is gone: mark it, and every live file under it, as deleted.
    fn record_gone(&self, root: &Root, rel: &str, origin: Origin) -> Result<Vec<Version>> {
        let mut out = Vec::new();
        for v in self.tree_at(root.id, rel, i64::MAX)? {
            if fs::symlink_metadata(absolute(root.dir(), &v.path)).is_err() {
                out.push(self.insert_deletion(root.id, &v.path, v.ts, origin)?);
            }
        }
        Ok(out)
    }

    fn insert_deletion(&self, root_id: i64, path: &str, after: i64, origin: Origin) -> Result<Version> {
        let ts = now_ms().max(after + 1);
        let db = self.db();
        db.execute(
            "INSERT INTO versions(root_id, path, ts, kind, origin) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![root_id, path, ts, Kind::Del.code(), origin.code()],
        )?;
        Ok(Version {
            id: db.last_insert_rowid(),
            root_id,
            path: path.to_string(),
            ts,
            kind: Kind::Del,
            origin,
            hash: None,
            size: 0,
            mode: 0,
            mtime: 0,
        })
    }

    /// Walk a directory inside a root and record every file that changed, appeared or vanished.
    fn scan(&self, root: &Root, sub: &str, origin: Origin) -> Result<Vec<Version>> {
        let base = absolute(root.dir(), sub);
        let filter = self.filter(root);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        if base.is_dir() {
            let walker = WalkDir::new(&base)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| !filter.ignores(e.path(), e.file_type().is_dir()));
            for entry in walker {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        log::debug!("scan: {e}");
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let Some(rel) = relative(root.dir(), entry.path()) else {
                    continue;
                };
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                match self.record_file(root, &rel, entry.path(), &meta, origin) {
                    Ok(Some(v)) => out.push(v),
                    Ok(None) => {}
                    Err(e) => log::warn!("{}: {e:#}", entry.path().display()),
                }
                seen.insert(rel);
            }
        }
        for v in self.tree_at(root.id, sub, i64::MAX)? {
            if !seen.contains(&v.path) && fs::symlink_metadata(absolute(root.dir(), &v.path)).is_err() {
                out.push(self.insert_deletion(root.id, &v.path, v.ts, origin)?);
            }
        }
        Ok(out)
    }

    // ---- blobs ----------------------------------------------------------------------------

    pub(crate) fn blob_path(&self, hash: &str) -> PathBuf {
        self.home.join("objects").join(&hash[..2]).join(&hash[2..])
    }

    /// The content of a version, by hash.
    pub fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if hash.len() < 3 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("not a content hash: {hash}");
        }
        let raw = fs::read(self.blob_path(hash)).with_context(|| format!("content {hash} is missing"))?;
        Ok(zstd::decode_all(&raw[..])?)
    }

    // ---- queries --------------------------------------------------------------------------

    pub fn version(&self, id: i64) -> Result<Version> {
        let sql = format!("SELECT {VERSION_COLS} FROM versions WHERE id = ?1");
        self.db()
            .query_row(&sql, [id], Version::from_row)
            .optional()?
            .ok_or_else(|| anyhow!("no version {id}"))
    }

    /// The newest version of a path, deletions included.
    pub fn latest(&self, root_id: i64, path: &str) -> Result<Option<Version>> {
        let sql = format!(
            "SELECT {VERSION_COLS} FROM versions WHERE root_id = ?1 AND path = ?2 ORDER BY ts DESC, id DESC LIMIT 1"
        );
        Ok(self
            .db()
            .query_row(&sql, params![root_id, path], Version::from_row)
            .optional()?)
    }

    /// The version of the same file just before `v`.
    pub fn previous(&self, v: &Version) -> Result<Option<Version>> {
        let sql = format!(
            "SELECT {VERSION_COLS} FROM versions
             WHERE root_id = ?1 AND path = ?2 AND (ts < ?3 OR (ts = ?3 AND id < ?4))
             ORDER BY ts DESC, id DESC LIMIT 1"
        );
        Ok(self
            .db()
            .query_row(&sql, params![v.root_id, v.path, v.ts, v.id], Version::from_row)
            .optional()?)
    }

    /// Every version of one file, newest first.
    pub fn file_versions(&self, root_id: i64, path: &str) -> Result<Vec<Version>> {
        let sql =
            format!("SELECT {VERSION_COLS} FROM versions WHERE root_id = ?1 AND path = ?2 ORDER BY ts DESC, id DESC");
        let db = self.db();
        let mut st = db.prepare(&sql)?;
        let rows = st.query_map(params![root_id, path], Version::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Changes, newest first, recorded at or before `before` (ms) and not part of the initial
    /// snapshot. `cursor` continues a previous page: pass the `(ts, id)` of its last entry.
    pub fn events(
        &self,
        root_id: i64,
        prefix: &str,
        before: i64,
        cursor: Option<(i64, i64)>,
        limit: usize,
    ) -> Result<Vec<Version>> {
        let (cts, cid) = cursor.unwrap_or((before, i64::MAX));
        let sql = format!(
            "SELECT {VERSION_COLS} FROM versions
             WHERE root_id = ?1 AND origin != 1 AND ts <= ?2 AND (ts < ?3 OR (ts = ?3 AND id < ?4))
               AND (?5 = '' OR path = ?5 OR substr(path, 1, length(?5) + 1) = ?5 || '/')
             ORDER BY ts DESC, id DESC LIMIT ?6"
        );
        let db = self.db();
        let mut st = db.prepare(&sql)?;
        let rows = st.query_map(
            params![root_id, before, cts, cid, prefix, limit as i64],
            Version::from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The files that existed under `prefix` at time `at`, with the version each one had.
    pub fn tree_at(&self, root_id: i64, prefix: &str, at: i64) -> Result<Vec<FileAt>> {
        let sql = format!(
            "SELECT {VERSION_COLS} FROM (
                SELECT *, ROW_NUMBER() OVER (PARTITION BY path ORDER BY ts DESC, id DESC) AS rn
                FROM versions
                WHERE root_id = ?1 AND ts <= ?2
                  AND (?3 = '' OR path = ?3 OR substr(path, 1, length(?3) + 1) = ?3 || '/')
             ) WHERE rn = 1 AND kind != 2 ORDER BY path"
        );
        let db = self.db();
        let mut st = db.prepare(&sql)?;
        let rows = st.query_map(params![root_id, at, prefix], Version::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Counts of changes in `buckets` equal slices of `[from, to)`. The initial snapshot is left out.
    pub fn timeline(&self, root_id: i64, from: i64, to: i64, buckets: usize) -> Result<Vec<Bucket>> {
        let buckets = buckets.clamp(1, 2000) as i64;
        let width = ((to - from) / buckets).max(1);
        let mut out: Vec<Bucket> = (0..buckets)
            .map(|i| Bucket {
                start: from + i * width,
                end: from + (i + 1) * width,
                ..Default::default()
            })
            .collect();
        let db = self.db();
        let mut st = db.prepare(
            "SELECT (ts - ?2) / ?4 AS b, kind, COUNT(*) FROM versions
             WHERE root_id = ?1 AND origin != 1 AND ts >= ?2 AND ts < ?3
             GROUP BY b, kind",
        )?;
        let rows = st.query_map(params![root_id, from, to, width], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?;
        for row in rows {
            let (b, kind, n) = row?;
            if let Some(slot) = out.get_mut(b as usize) {
                match Kind::from_code(kind) {
                    Kind::Add => slot.added += n,
                    Kind::Mod => slot.modified += n,
                    Kind::Del => slot.deleted += n,
                }
            }
        }
        Ok(out)
    }

    /// Bursts since `since`: runs of live changes with gaps of at most `gap_ms`, touching at
    /// least `min_files` distinct files.
    pub fn bursts(&self, root_id: i64, since: i64, min_files: usize, gap_ms: i64) -> Result<Vec<Burst>> {
        let rows: Vec<(String, i64, Kind)> = {
            let db = self.db();
            let mut st = db.prepare(
                "SELECT path, ts, kind FROM versions WHERE root_id = ?1 AND origin = 0 AND ts >= ?2 ORDER BY ts, id",
            )?;
            st.query_map(params![root_id, since], |r| {
                Ok((r.get(0)?, r.get(1)?, Kind::from_code(r.get(2)?)))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::new();
        let mut cur: Vec<(String, i64, Kind)> = Vec::new();
        let mut flush = |cur: &mut Vec<(String, i64, Kind)>| {
            if let Some(b) = make_burst(root_id, cur, min_files) {
                out.push(b);
            }
            cur.clear();
        };
        for row in rows {
            if cur.last().is_some_and(|last| row.1 - last.1 > gap_ms) {
                flush(&mut cur);
            }
            cur.push(row);
        }
        flush(&mut cur);
        Ok(out)
    }

    /// Lines added and removed by one version, compared with the one before it.
    pub fn diff_version(&self, id: i64) -> Result<Diff> {
        let v = self.version(id)?;
        let prev = self.previous(&v)?;
        let old = match prev.as_ref().and_then(|p| p.hash.as_deref()) {
            Some(h) => self.blob(h)?,
            None => Vec::new(),
        };
        let new = match v.hash.as_deref() {
            Some(h) => self.blob(h)?,
            None => Vec::new(),
        };
        Ok(diff::diff(&old, &new, 3))
    }

    /// The difference between a stored version and what is on disk now.
    pub fn diff_with_disk(&self, id: i64) -> Result<Diff> {
        let v = self.version(id)?;
        let root = self.root(v.root_id)?;
        let old = match v.hash.as_deref() {
            Some(h) => self.blob(h)?,
            None => Vec::new(),
        };
        let now = fs::read(absolute(root.dir(), &v.path)).unwrap_or_default();
        Ok(diff::diff(&old, &now, 3))
    }

    pub fn stats(&self) -> Result<Stats> {
        let db = self.db();
        let roots = db.query_row("SELECT COUNT(*) FROM roots", [], |r| r.get(0))?;
        let versions = db.query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))?;
        let (blobs, stored_bytes) = db.query_row("SELECT COUNT(*), COALESCE(SUM(stored), 0) FROM blobs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        let original_bytes = db.query_row("SELECT COALESCE(SUM(size), 0) FROM versions", [], |r| r.get(0))?;
        Ok(Stats {
            roots,
            versions,
            blobs,
            stored_bytes,
            original_bytes,
        })
    }

    // ---- restore --------------------------------------------------------------------------

    /// Work out what it takes to put `scope` back the way it was at `at`. The current state is
    /// recorded first, so whatever the restore overwrites stays in the history.
    pub fn plan_restore(&self, root_id: i64, at: i64, scope: &Scope) -> Result<RestorePlan> {
        let root = self.root(root_id)?;
        match scope {
            Scope::All => {
                self.scan(&root, "", Origin::CatchUp)?;
            }
            Scope::Prefix(p) => {
                self.record(&root, p, Origin::CatchUp)?;
            }
            Scope::Paths(ps) => {
                for p in ps {
                    self.record(&root, p, Origin::CatchUp)?;
                }
            }
        }
        let then: HashMap<String, Version> = self
            .tree_at(root_id, scope.prefix(), at)?
            .into_iter()
            .filter(|v| scope.contains(&v.path))
            .map(|v| (v.path.clone(), v))
            .collect();
        let now: HashMap<String, Version> = self
            .tree_at(root_id, scope.prefix(), i64::MAX)?
            .into_iter()
            .filter(|v| scope.contains(&v.path))
            .map(|v| (v.path.clone(), v))
            .collect();
        let mut write: Vec<RestoreItem> = then
            .values()
            .filter(|t| now.get(&t.path).is_none_or(|n| n.hash != t.hash || n.mode != t.mode))
            .filter_map(|t| {
                Some(RestoreItem {
                    path: t.path.clone(),
                    hash: t.hash.clone()?,
                    size: t.size,
                    mode: t.mode,
                    exists: now.contains_key(&t.path),
                })
            })
            .collect();
        let mut delete: Vec<String> = now.keys().filter(|p| !then.contains_key(*p)).cloned().collect();
        write.sort_by(|a, b| a.path.cmp(&b.path));
        delete.sort();
        Ok(RestorePlan {
            root_id,
            at,
            write,
            delete,
        })
    }

    /// Carry out a plan. Every file written or removed is recorded as a new version, so the
    /// restore shows up in the history and can itself be undone.
    pub fn apply_restore(&self, plan: &RestorePlan) -> Result<RestoreReport> {
        let root = self.root(plan.root_id)?;
        let mut report = RestoreReport::default();
        for item in &plan.write {
            let abs = absolute(root.dir(), &item.path);
            let result = self.blob(&item.hash).and_then(|bytes| {
                write_atomically(&abs, &bytes)?;
                set_mode(&abs, item.mode);
                Ok(())
            });
            match result {
                Ok(()) => {
                    report.written += 1;
                    self.record(&root, &item.path, Origin::Restore)?;
                }
                Err(e) => report.failed.push((item.path.clone(), format!("{e:#}"))),
            }
        }
        for path in &plan.delete {
            let abs = absolute(root.dir(), path);
            match fs::remove_file(&abs) {
                Ok(()) => {
                    report.deleted += 1;
                    self.record(&root, path, Origin::Restore)?;
                    remove_empty_parents(root.dir(), &abs);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => report.failed.push((path.clone(), e.to_string())),
            }
        }
        Ok(report)
    }
}

fn make_burst(root_id: i64, rows: &[(String, i64, Kind)], min_files: usize) -> Option<Burst> {
    let mut first_kind: HashMap<&str, Kind> = HashMap::new();
    let mut last_kind: HashMap<&str, Kind> = HashMap::new();
    let mut paths = Vec::new();
    for (p, _, k) in rows {
        if !first_kind.contains_key(p.as_str()) {
            first_kind.insert(p, *k);
            paths.push(p.clone());
        }
        last_kind.insert(p, *k);
    }
    if paths.len() < min_files {
        return None;
    }
    // Classify each file by its net effect over the burst.
    let (mut added, mut modified, mut deleted) = (0, 0, 0);
    for p in &paths {
        match (first_kind[p.as_str()], last_kind[p.as_str()]) {
            (Kind::Add, Kind::Del) => {}
            (Kind::Add, _) => added += 1,
            (_, Kind::Del) => deleted += 1,
            _ => modified += 1,
        }
    }
    Some(Burst {
        root_id,
        start: rows[0].1,
        end: rows[rows.len() - 1].1,
        added,
        modified,
        deleted,
        paths,
    })
}

fn canonical_or_parent(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    // A path that no longer exists: canonicalize the nearest ancestor that does.
    let mut tail = Vec::new();
    let mut cur = path.to_path_buf();
    while let Some(name) = cur.file_name().map(|n| n.to_os_string()) {
        tail.push(name);
        cur.pop();
        if let Ok(mut p) = cur.canonicalize() {
            for t in tail.iter().rev() {
                p.push(t);
            }
            return p;
        }
    }
    path.to_path_buf()
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.clessidra-tmp", std::process::id()));
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

fn mtime_ms(m: &fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn mode_of(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(_: &fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if mode != 0 {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) {}
