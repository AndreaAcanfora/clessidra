//! Keeping the store small. Recent history is kept in full; older history is thinned out (one
//! version per file per hour after a day, per day after a week), dropped after `keep_days`, and
//! evicted oldest-first if the store still exceeds `max_store_mb`. The current version of every
//! file is never dropped.

use std::collections::{HashMap, HashSet};
use std::fs;

use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

use crate::now_ms;
use crate::store::Store;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

#[derive(Serialize, Clone, Debug, Default)]
pub struct PruneReport {
    pub thinned: usize,
    pub expired: usize,
    pub evicted: usize,
    pub blobs_removed: usize,
    pub bytes_freed: i64,
}

impl Store {
    pub fn prune(&self) -> Result<PruneReport> {
        self.prune_at(now_ms())
    }

    pub fn prune_at(&self, now: i64) -> Result<PruneReport> {
        let settings = self.settings();
        let mut report = PruneReport::default();
        let current = self.current_ids()?;

        // Thin out and expire.
        let old: Vec<(i64, i64, String, i64, i64)> = {
            let db = self.db();
            let mut st = db.prepare(
                "SELECT id, root_id, path, ts, kind FROM versions WHERE ts < ?1 ORDER BY root_id, path, ts DESC, id DESC",
            )?;
            st.query_map([now - DAY], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let expiry = now - settings.keep_days * DAY;
        let mut kept_slot: HashSet<(i64, String, i64)> = HashSet::new();
        let mut doomed_thin = Vec::new();
        let mut doomed_expired = Vec::new();
        for (id, root_id, path, ts, kind) in old {
            if current.contains(&id) {
                continue;
            }
            if ts < expiry {
                doomed_expired.push(id);
                continue;
            }
            if kind == 2 {
                continue; // deletions are tiny and keep the timeline honest
            }
            let slot = if now - ts > 7 * DAY { ts / DAY } else { ts / HOUR };
            // Rows come newest first, so the first one seen in a slot is the one kept.
            if !kept_slot.insert((root_id, path, slot)) {
                doomed_thin.push(id);
            }
        }
        report.thinned = self.delete_versions(&doomed_thin)?;
        report.expired = self.delete_versions(&doomed_expired)?;
        let (n, bytes) = self.gc()?;
        report.blobs_removed += n;
        report.bytes_freed += bytes;

        // Evict the oldest history until the store fits.
        let cap = settings.max_store_mb * 1024 * 1024;
        loop {
            let stored: i64 = self
                .db()
                .query_row("SELECT COALESCE(SUM(stored), 0) FROM blobs", [], |r| r.get(0))?;
            if stored <= cap {
                break;
            }
            let batch: Vec<i64> = {
                let db = self.db();
                let mut st = db.prepare("SELECT id FROM versions ORDER BY ts, id LIMIT 2000")?;
                st.query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<i64>>>()?
                    .into_iter()
                    .filter(|id| !current.contains(id))
                    .take(200)
                    .collect()
            };
            if batch.is_empty() {
                break;
            }
            report.evicted += self.delete_versions(&batch)?;
            let (n, bytes) = self.gc()?;
            report.blobs_removed += n;
            report.bytes_freed += bytes;
        }
        Ok(report)
    }

    /// The id of the newest version of every path: these are never pruned.
    fn current_ids(&self) -> Result<HashSet<i64>> {
        let db = self.db();
        let mut st = db.prepare(
            "SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (PARTITION BY root_id, path ORDER BY ts DESC, id DESC) AS rn FROM versions
             ) WHERE rn = 1",
        )?;
        Ok(st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    fn delete_versions(&self, ids: &[i64]) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut n = 0;
        {
            let mut st = tx.prepare("DELETE FROM versions WHERE id = ?1")?;
            for id in ids {
                n += st.execute(params![id])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Remove content no version refers to any more. Returns (blobs removed, bytes freed).
    pub(crate) fn gc(&self) -> Result<(usize, i64)> {
        let db = self.db();
        let gone: Vec<(String, i64)> = {
            let mut st = db.prepare(
                "DELETE FROM blobs WHERE NOT EXISTS (SELECT 1 FROM versions v WHERE v.hash = blobs.hash)
                 RETURNING hash, stored",
            )?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        // Files are removed while the index is still locked; see `record_file`.
        let mut by_dir: HashMap<String, usize> = HashMap::new();
        let mut bytes = 0;
        for (hash, stored) in &gone {
            let _ = fs::remove_file(self.blob_path(hash));
            *by_dir.entry(hash[..2].to_string()).or_default() += 1;
            bytes += stored;
        }
        for dir in by_dir.keys() {
            let _ = fs::remove_dir(self.home().join("objects").join(dir));
        }
        Ok((gone.len(), bytes))
    }
}
