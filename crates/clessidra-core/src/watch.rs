//! The watcher: turns filesystem events into versions.
//!
//! Events are coalesced per path: a path is recorded once it has been quiet for a moment, or
//! after a few seconds at the latest if something keeps writing to it. On start, and whenever
//! the platform says it dropped events, each folder is rescanned against its history.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use notify::{RecursiveMode, Watcher};

use crate::filter::IGNORE_FILE;
use crate::now_ms;
use crate::paths::relative;
use crate::store::{Burst, Origin, Root, Store, Version};

/// A path is recorded after this long without new events...
const QUIET: Duration = Duration::from_millis(400);
/// ...or after this long at most, for files that never stop changing.
const MAX_WAIT: Duration = Duration::from_secs(3);
const TICK: Duration = Duration::from_millis(150);
/// Bursts: at least this many files, with no gap longer than this between changes.
pub const BURST_MIN_FILES: usize = 10;
pub const BURST_GAP_MS: i64 = 2_000;
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub enum Event {
    /// New versions were recorded in one folder.
    Recorded { root_id: i64, versions: Vec<Version> },
    /// A burst of changes just finished.
    Burst(Burst),
    /// Something went wrong; the watcher keeps going.
    Error(String),
}

enum Msg {
    Fs(notify::Result<notify::Event>),
    Reload,
    Stop,
}

/// A running watcher. Dropping it stops the thread.
pub struct Daemon {
    tx: Sender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl Daemon {
    /// Start watching every folder in the store. Only one daemon may run per store; a second one
    /// (another copy of the app, or `clessidra daemon` next to it) fails here.
    pub fn start(store: Arc<Store>, on_event: impl Fn(Event) + Send + 'static) -> Result<Daemon> {
        let lock = File::create(store.home().join("daemon.lock"))?;
        if lock.try_lock().is_err() {
            bail!("another clessidra is already watching (the app or `clessidra daemon`)");
        }
        let (tx, rx) = mpsc::channel();
        let fs_tx = tx.clone();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = fs_tx.send(Msg::Fs(res));
        })
        .context("starting the filesystem watcher")?;
        let handle = std::thread::Builder::new()
            .name("clessidra-watch".into())
            .spawn(move || {
                let mut w = Worker {
                    store,
                    on_event: Box::new(on_event),
                    watcher,
                    watched: HashMap::new(),
                    pending: HashMap::new(),
                    rescan: HashSet::new(),
                    notified: HashSet::new(),
                    live_until: None,
                    _lock: lock,
                };
                w.sync_roots();
                let mut last_prune = Instant::now();
                loop {
                    match rx.recv_timeout(TICK) {
                        Ok(Msg::Fs(Ok(ev))) => w.on_fs(ev),
                        Ok(Msg::Fs(Err(e))) => (w.on_event)(Event::Error(e.to_string())),
                        Ok(Msg::Reload) => w.sync_roots(),
                        Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    w.flush(false);
                    if last_prune.elapsed() > PRUNE_EVERY {
                        last_prune = Instant::now();
                        if let Err(e) = w.store.prune() {
                            (w.on_event)(Event::Error(format!("prune: {e:#}")));
                        }
                    }
                }
                w.flush(true);
            })?;
        Ok(Daemon {
            tx,
            handle: Some(handle),
        })
    }

    /// Pick up folders that were added, removed, paused or resumed.
    pub fn reload(&self) {
        let _ = self.tx.send(Msg::Reload);
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Worker {
    store: Arc<Store>,
    on_event: Box<dyn Fn(Event) + Send>,
    watcher: notify::RecommendedWatcher,
    watched: HashMap<i64, Root>,
    pending: HashMap<(i64, String), (Instant, Instant)>,
    rescan: HashSet<i64>,
    notified: HashSet<(i64, i64)>,
    /// Until when to look for bursts (some live change happened recently).
    live_until: Option<Instant>,
    _lock: File,
}

impl Worker {
    fn sync_roots(&mut self) {
        let roots = match self.store.roots() {
            Ok(r) => r,
            Err(e) => return (self.on_event)(Event::Error(format!("{e:#}"))),
        };
        let active: HashMap<i64, Root> = roots.into_iter().filter(|r| !r.paused).map(|r| (r.id, r)).collect();
        let gone: Vec<i64> = self
            .watched
            .keys()
            .filter(|id| !active.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            if let Some(r) = self.watched.remove(&id) {
                let _ = self.watcher.unwatch(r.dir());
            }
            self.pending.retain(|(rid, _), _| *rid != id);
        }
        for (id, root) in active {
            if self.watched.contains_key(&id) {
                continue;
            }
            if let Err(e) = self.watcher.watch(root.dir(), RecursiveMode::Recursive) {
                (self.on_event)(Event::Error(format!("{}: {e}", root.path)));
                continue;
            }
            // Bursts from before this start were either reported already or are old news.
            if let Ok(old) = self
                .store
                .bursts(id, now_ms() - 10 * 60_000, BURST_MIN_FILES, BURST_GAP_MS)
            {
                self.notified.extend(old.iter().map(|b| (id, b.start)));
            }
            self.watched.insert(id, root);
            self.rescan.insert(id);
        }
    }

    fn root_of(&self, path: &PathBuf) -> Option<(&Root, String)> {
        self.watched
            .values()
            .filter_map(|r| relative(r.dir(), path).map(|rel| (r, rel)))
            .max_by_key(|(r, _)| r.path.len())
    }

    fn on_fs(&mut self, ev: notify::Event) {
        if ev.need_rescan() {
            self.rescan.extend(self.watched.keys().copied());
            return;
        }
        if matches!(ev.kind, notify::EventKind::Access(_)) {
            return;
        }
        let now = Instant::now();
        for p in &ev.paths {
            let Some((root, rel)) = self.root_of(p) else { continue };
            let root_id = root.id;
            if rel == IGNORE_FILE {
                self.store.invalidate_filter(root_id);
            }
            let ignored = self.store.filter(root).ignores(p, p.is_dir());
            if rel == IGNORE_FILE {
                self.rescan.insert(root_id);
            }
            if ignored {
                continue;
            }
            self.pending
                .entry((root_id, rel))
                .and_modify(|e| e.1 = now)
                .or_insert((now, now));
        }
    }

    fn flush(&mut self, all: bool) {
        for id in std::mem::take(&mut self.rescan) {
            let Some(root) = self.watched.get(&id).cloned() else {
                continue;
            };
            self.pending.retain(|(rid, _), _| *rid != id);
            match self.store.catch_up(&root) {
                Ok(v) if !v.is_empty() => (self.on_event)(Event::Recorded {
                    root_id: id,
                    versions: v,
                }),
                Ok(_) => {}
                Err(e) => (self.on_event)(Event::Error(format!("{}: {e:#}", root.path))),
            }
        }
        let ready: Vec<(i64, String)> = self
            .pending
            .iter()
            .filter(|(_, (first, last))| all || last.elapsed() >= QUIET || first.elapsed() >= MAX_WAIT)
            .map(|(k, _)| k.clone())
            .collect();
        let mut by_root: HashMap<i64, Vec<Version>> = HashMap::new();
        for key in ready {
            self.pending.remove(&key);
            let (root_id, rel) = key;
            let Some(root) = self.watched.get(&root_id) else {
                continue;
            };
            match self.store.record(root, &rel, Origin::Live) {
                Ok(v) => by_root.entry(root_id).or_default().extend(v),
                Err(e) => (self.on_event)(Event::Error(format!("{rel}: {e:#}"))),
            }
        }
        for (root_id, versions) in by_root {
            if !versions.is_empty() {
                self.live_until = Some(Instant::now() + Duration::from_secs(10));
                (self.on_event)(Event::Recorded { root_id, versions });
            }
        }
        if self.live_until.is_some_and(|t| Instant::now() < t) {
            self.check_bursts();
        }
    }

    /// Report each burst once, when it has gone quiet.
    fn check_bursts(&mut self) {
        let now = now_ms();
        for &root_id in self.watched.keys() {
            let Ok(bursts) = self
                .store
                .bursts(root_id, now - 10 * 60_000, BURST_MIN_FILES, BURST_GAP_MS)
            else {
                continue;
            };
            for b in bursts {
                if now - b.end > BURST_GAP_MS && self.notified.insert((root_id, b.start)) {
                    (self.on_event)(Event::Burst(b));
                }
            }
        }
    }
}
