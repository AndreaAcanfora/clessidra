use std::fs;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use clessidra_core::watch::{Daemon, Event};
use clessidra_core::{Kind, Origin, Scope, Settings, Store, now_ms};

struct Fixture {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    store: Store,
    dir: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let dir = work.path().canonicalize().unwrap();
    Fixture {
        _dirs: (home, work),
        store,
        dir,
    }
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn read(dir: &Path, rel: &str) -> Option<String> {
    fs::read_to_string(dir.join(rel)).ok()
}

/// Versions are timestamped in milliseconds; keep consecutive steps apart.
fn tick() {
    std::thread::sleep(Duration::from_millis(5));
}

#[test]
fn rewinds_a_folder_and_the_rewind_can_be_undone() {
    let f = fixture();
    write(&f.dir, "notes.md", "one");
    write(&f.dir, "src/main.rs", "fn main() {}");
    let root = f.store.add_root(&f.dir).unwrap();
    assert_eq!(f.store.snapshot(&root).unwrap(), 2);
    tick();
    let before = now_ms();
    tick();

    // An over-eager tool rewrites one file, deletes a folder and leaves junk behind.
    write(&f.dir, "notes.md", "two");
    fs::remove_dir_all(f.dir.join("src")).unwrap();
    write(&f.dir, "junk/out.txt", "junk");
    for rel in ["notes.md", "src", "junk"] {
        f.store.record(&root, rel, Origin::Live).unwrap();
    }
    let kinds: Vec<Kind> = f
        .store
        .events(root.id, "", i64::MAX, None, 10)
        .unwrap()
        .iter()
        .map(|v| v.kind)
        .collect();
    assert_eq!(kinds.len(), 3);
    assert!(kinds.contains(&Kind::Del) && kinds.contains(&Kind::Mod) && kinds.contains(&Kind::Add));
    tick();
    let after = now_ms();

    let plan = f.store.plan_restore(root.id, before, &Scope::All).unwrap();
    assert_eq!(
        plan.write.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(),
        ["notes.md", "src/main.rs"]
    );
    assert_eq!(plan.delete, ["junk/out.txt"]);
    let report = f.store.apply_restore(&plan).unwrap();
    assert_eq!((report.written, report.deleted, report.failed.len()), (2, 1, 0));
    assert_eq!(read(&f.dir, "notes.md").as_deref(), Some("one"));
    assert_eq!(read(&f.dir, "src/main.rs").as_deref(), Some("fn main() {}"));
    assert!(
        !f.dir.join("junk").exists(),
        "empty folders left by the restore are removed"
    );

    // The restore is in the history too, so it can be rewound in turn.
    tick();
    let plan = f.store.plan_restore(root.id, after, &Scope::All).unwrap();
    f.store.apply_restore(&plan).unwrap();
    assert_eq!(read(&f.dir, "notes.md").as_deref(), Some("two"));
    assert!(!f.dir.join("src").exists());
    assert_eq!(read(&f.dir, "junk/out.txt").as_deref(), Some("junk"));
}

#[test]
fn restoring_one_file_leaves_the_rest_alone() {
    let f = fixture();
    write(&f.dir, "a.txt", "a1");
    write(&f.dir, "b.txt", "b1");
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    tick();
    let t = now_ms();
    tick();
    write(&f.dir, "a.txt", "a2");
    write(&f.dir, "b.txt", "b2");
    let plan = f
        .store
        .plan_restore(root.id, t, &Scope::Prefix("a.txt".into()))
        .unwrap();
    assert_eq!(plan.write.len(), 1);
    f.store.apply_restore(&plan).unwrap();
    assert_eq!(read(&f.dir, "a.txt").as_deref(), Some("a1"));
    assert_eq!(read(&f.dir, "b.txt").as_deref(), Some("b2"));
    // b.txt's new content was picked up by the plan's catch-up only for the scope; record it now.
    f.store.record(&root, "b.txt", Origin::Live).unwrap();
    let versions = f.store.file_versions(root.id, "a.txt").unwrap();
    assert_eq!(
        versions.len(),
        3,
        "baseline, the overwrite found while planning, the restore"
    );
    assert_eq!(versions[0].origin, Origin::Restore);
}

#[test]
fn a_burst_of_deletions_is_detected_and_undone() {
    let f = fixture();
    for i in 0..30 {
        write(&f.dir, &format!("photos/{i:02}.jpg"), &format!("photo {i}"));
    }
    write(&f.dir, "keep.txt", "unrelated");
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    assert!(
        f.store.bursts(root.id, 0, 10, 2000).unwrap().is_empty(),
        "the initial snapshot is not a burst"
    );
    tick();

    for i in 0..30 {
        fs::remove_file(f.dir.join(format!("photos/{i:02}.jpg"))).unwrap();
        f.store
            .record(&root, &format!("photos/{i:02}.jpg"), Origin::Live)
            .unwrap();
    }
    let bursts = f.store.bursts(root.id, 0, 10, 2000).unwrap();
    assert_eq!(bursts.len(), 1);
    let b = &bursts[0];
    assert_eq!((b.added, b.modified, b.deleted), (0, 0, 30));

    // Something else changes after the burst; undoing the burst must not touch it.
    tick();
    write(&f.dir, "keep.txt", "edited later");
    let plan = f
        .store
        .plan_restore(root.id, b.start - 1, &Scope::Paths(b.paths.clone()))
        .unwrap();
    assert_eq!((plan.write.len(), plan.delete.len()), (30, 0));
    f.store.apply_restore(&plan).unwrap();
    assert_eq!(read(&f.dir, "photos/07.jpg").as_deref(), Some("photo 7"));
    assert_eq!(read(&f.dir, "keep.txt").as_deref(), Some("edited later"));
}

#[test]
fn identical_content_is_stored_once() {
    let f = fixture();
    let big = "x".repeat(100_000);
    for i in 0..5 {
        write(&f.dir, &format!("copy{i}.txt"), &big);
    }
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    let s = f.store.stats().unwrap();
    assert_eq!((s.versions, s.blobs), (5, 1));
    assert_eq!(s.original_bytes, 500_000);
    assert!(s.stored_bytes < 1_000, "compressed: {}", s.stored_bytes);
}

#[test]
fn unchanged_and_ignored_files_create_no_versions() {
    let f = fixture();
    write(&f.dir, "a.txt", "same");
    write(&f.dir, "node_modules/pkg/index.js", "module.exports = 1");
    write(&f.dir, ".git/HEAD", "ref: refs/heads/main");
    let root = f.store.add_root(&f.dir).unwrap();
    assert_eq!(f.store.snapshot(&root).unwrap(), 1);
    // Rewriting the same bytes (a new mtime) is not a change.
    tick();
    write(&f.dir, "a.txt", "same");
    assert!(f.store.record(&root, "a.txt", Origin::Live).unwrap().is_empty());
    assert!(
        f.store
            .record(&root, "node_modules/pkg/index.js", Origin::Live)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tree_at_shows_the_folder_as_it_was() {
    let f = fixture();
    write(&f.dir, "a.txt", "1");
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    tick();
    let t1 = now_ms();
    tick();
    write(&f.dir, "b.txt", "2");
    fs::remove_file(f.dir.join("a.txt")).unwrap();
    f.store.catch_up(&root).unwrap();
    let then: Vec<String> = f
        .store
        .tree_at(root.id, "", t1)
        .unwrap()
        .into_iter()
        .map(|v| v.path)
        .collect();
    let now: Vec<String> = f
        .store
        .tree_at(root.id, "", i64::MAX)
        .unwrap()
        .into_iter()
        .map(|v| v.path)
        .collect();
    assert_eq!(then, ["a.txt"]);
    assert_eq!(now, ["b.txt"]);
}

#[test]
fn diffs_between_versions() {
    let f = fixture();
    write(&f.dir, "a.txt", "alpha\nbeta\n");
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    tick();
    write(&f.dir, "a.txt", "alpha\ngamma\n");
    let v = f.store.record(&root, "a.txt", Origin::Live).unwrap().remove(0);
    let d = f.store.diff_version(v.id).unwrap();
    assert_eq!((d.added, d.removed), (1, 1));
}

#[test]
fn overlapping_and_bogus_roots_are_refused() {
    let f = fixture();
    fs::create_dir_all(f.dir.join("inner")).unwrap();
    f.store.add_root(&f.dir).unwrap();
    assert!(f.store.add_root(&f.dir.join("inner")).is_err());
    assert!(f.store.add_root(&f.dir.join("missing")).is_err());
    assert!(f.store.add_root(f.store.home()).is_err());
}

#[cfg(unix)]
#[test]
fn restores_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    write(&f.dir, "run.sh", "#!/bin/sh\necho hi\n");
    fs::set_permissions(f.dir.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    tick();
    let t = now_ms();
    tick();
    fs::remove_file(f.dir.join("run.sh")).unwrap();
    let plan = f.store.plan_restore(root.id, t, &Scope::All).unwrap();
    f.store.apply_restore(&plan).unwrap();
    let mode = fs::metadata(f.dir.join("run.sh")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn pruning_thins_old_history_and_keeps_current_files() {
    let f = fixture();
    write(&f.dir, "a.txt", "v0");
    let root = f.store.add_root(&f.dir).unwrap();
    f.store.snapshot(&root).unwrap();
    for i in 1..=20 {
        tick();
        write(&f.dir, "a.txt", &format!("v{i}"));
        f.store.record(&root, "a.txt", Origin::Live).unwrap();
    }
    assert_eq!(f.store.file_versions(root.id, "a.txt").unwrap().len(), 21);

    // Two days later every edit falls in the same hour: one survives, plus the current version.
    let report = f.store.prune_at(now_ms() + 2 * 86_400_000).unwrap();
    assert_eq!(report.thinned, 19);
    let left = f.store.file_versions(root.id, "a.txt").unwrap();
    assert_eq!(left.len(), 2);
    assert_eq!(f.store.blob(left[0].hash.as_deref().unwrap()).unwrap(), b"v20");

    // Past keep_days only the current version is left.
    f.store
        .set_settings(Settings {
            keep_days: 1,
            ..Settings::default()
        })
        .unwrap();
    f.store.prune_at(now_ms() + 3 * 86_400_000).unwrap();
    assert_eq!(f.store.file_versions(root.id, "a.txt").unwrap().len(), 1);
    assert_eq!(f.store.stats().unwrap().blobs, 1);
}

#[test]
fn the_watcher_records_changes_as_they_happen() {
    let f = fixture();
    write(&f.dir, "a.txt", "1");
    let store = Arc::new(f.store);
    let root = store.add_root(&f.dir).unwrap();
    store.snapshot(&root).unwrap();

    let (tx, rx) = mpsc::channel();
    let daemon = Daemon::start(store.clone(), move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    assert!(Daemon::start(store.clone(), |_| {}).is_err(), "one watcher per store");
    std::thread::sleep(Duration::from_millis(300)); // let the platform watcher settle

    write(&f.dir, "a.txt", "2");
    write(&f.dir, "new/b.txt", "b");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while seen.len() < 2 && std::time::Instant::now() < deadline {
        if let Ok(Event::Recorded { versions, .. }) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.extend(versions.into_iter().map(|v| (v.path, v.kind)));
        }
    }
    daemon.stop();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        seen,
        [("a.txt".to_string(), Kind::Mod), ("new/b.txt".to_string(), Kind::Add)]
    );
}
