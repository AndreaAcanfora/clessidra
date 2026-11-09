//! The engine behind clessidra.
//!
//! A [`Store`] keeps every version of every file under the folders it watches: an SQLite index of
//! versions plus a directory of zstd-compressed, content-addressed blobs (identical content is
//! stored once). The [`watch`] module turns filesystem events into versions; [`Store::plan_restore`]
//! and [`Store::apply_restore`] put a folder, a file or a burst of changes back the way they were
//! at any recorded moment. A restore is itself recorded, so it can be undone like anything else.

mod diff;
mod filter;
mod paths;
mod prune;
mod store;
pub mod time;
pub mod watch;

pub use diff::{Diff, DiffLine, DiffLineKind};
pub use filter::{DEFAULT_IGNORES, Filter};
pub use prune::PruneReport;
pub use store::{
    Bucket, Burst, FileAt, Kind, Origin, RestoreItem, RestorePlan, RestoreReport, Root, Scope, Settings, Stats, Store,
    Version,
};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
