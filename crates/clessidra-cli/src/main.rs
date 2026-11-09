use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use clessidra_core::time::{ago, local_time, parse_moment};
use clessidra_core::watch::{BURST_GAP_MS, BURST_MIN_FILES, Daemon, Event};
use clessidra_core::{DiffLineKind, Kind, RestorePlan, Root, Scope, Store, Version, now_ms};

#[derive(Parser)]
#[command(name = "clessidra", version, about = "Undo for your whole disk.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start keeping the history of a folder (takes a first snapshot of every file).
    Add { dir: PathBuf },
    /// Stop watching a folder and forget its history.
    Remove {
        dir: PathBuf,
        #[arg(short, long)]
        yes: bool,
    },
    /// Watched folders.
    List,
    /// Watch every folder in the foreground (the desktop app does this for you).
    Daemon,
    /// What changed, newest first.
    Log {
        /// A file or folder inside a watched folder (default: the current directory).
        path: Option<PathBuf>,
        /// Only changes newer than this (`10m`, `2h`, `14:30`, `2026-11-10 14:30`).
        #[arg(long)]
        since: Option<String>,
        #[arg(short = 'n', long, default_value_t = 40)]
        limit: usize,
    },
    /// Every recorded version of one file.
    Versions { file: PathBuf },
    /// Print a file as it was at some moment.
    Show {
        file: PathBuf,
        #[arg(long, default_value = "now")]
        at: String,
    },
    /// Compare a file as it was at some moment with how it is now.
    Diff {
        file: PathBuf,
        #[arg(long)]
        at: String,
    },
    /// Put a file or folder back the way it was at some moment.
    Restore {
        path: PathBuf,
        #[arg(long)]
        at: String,
        /// Show what would change and stop.
        #[arg(long)]
        dry_run: bool,
        #[arg(short, long)]
        yes: bool,
    },
    /// Undo the most recent burst of changes (many files touched within seconds).
    Undo {
        path: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(short, long)]
        yes: bool,
    },
    /// Recent bursts of changes.
    Bursts {
        path: Option<PathBuf>,
        #[arg(long, default_value = "1d")]
        since: String,
    },
    /// Store size, settings and whether a watcher is running.
    Status,
    /// Thin out and drop old history now (the watcher also does this every hour).
    Prune,
    /// Change retention settings.
    Config {
        #[arg(long)]
        max_store_mb: Option<i64>,
        #[arg(long)]
        keep_days: Option<i64>,
        #[arg(long)]
        max_file_mb: Option<i64>,
    },
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("{} {e:#}", paint("31;1", "error:"));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let store = Store::open_default()?;
    let now = now_ms();
    match cli.cmd {
        Cmd::Add { dir } => {
            let root = store.add_root(&dir)?;
            eprint!("taking a first snapshot of {} ... ", root.path);
            let n = store.snapshot(&root)?;
            eprintln!("{n} files");
            if !watcher_running(&store) {
                println!("Nothing is watching yet: open the clessidra app, or run `clessidra daemon`.");
            }
        }
        Cmd::Remove { dir, yes } => {
            let (root, _) = locate(&store, &dir)?;
            if !yes && !confirm(&format!("Forget the whole history of {}?", root.path))? {
                return Ok(());
            }
            store.remove_root(root.id)?;
            println!("removed {}", root.path);
        }
        Cmd::List => {
            for r in store.roots()? {
                let last = r
                    .last_change
                    .map(|t| ago(t, now))
                    .unwrap_or_else(|| "no changes yet".into());
                let paused = if r.paused {
                    paint("33", " (paused)")
                } else {
                    String::new()
                };
                println!("{}{paused}  {}", r.path, paint("2", &last));
            }
        }
        Cmd::Daemon => daemon(store)?,
        Cmd::Log { path, since, limit } => {
            let (root, prefix) = locate(&store, &path_or_cwd(path)?)?;
            let since = since.map(|s| parse_moment(&s, now)).transpose()?.unwrap_or(i64::MIN);
            let events = store.events(root.id, &prefix, i64::MAX, None, limit)?;
            for v in events.iter().filter(|v| v.ts >= since) {
                print_event(v, now);
            }
        }
        Cmd::Versions { file } => {
            let (root, rel) = locate(&store, &file)?;
            for v in store.file_versions(root.id, &rel)? {
                let size = if v.kind == Kind::Del {
                    String::new()
                } else {
                    human_bytes(v.size)
                };
                println!(
                    "{}  {}  {:>9}  {}",
                    local_time(v.ts),
                    kind_tag(v.kind),
                    size,
                    paint("2", &format!("{:?}", v.origin).to_lowercase())
                );
            }
        }
        Cmd::Show { file, at } => {
            let (root, rel) = locate(&store, &file)?;
            let at = parse_moment(&at, now)?;
            let v = version_at(&store, &root, &rel, at)?;
            std::io::stdout().write_all(&store.blob(v.hash.as_deref().unwrap_or_default())?)?;
        }
        Cmd::Diff { file, at } => {
            let (root, rel) = locate(&store, &file)?;
            let v = version_at(&store, &root, &rel, parse_moment(&at, now)?)?;
            let d = store.diff_with_disk(v.id)?;
            if !d.text {
                bail!("{rel} is not a text file");
            }
            println!("{} {rel} @ {}", paint("31", "---"), local_time(v.ts));
            println!("{} {rel} now", paint("32", "+++"));
            for l in &d.lines {
                match l.kind {
                    DiffLineKind::Context => println!(" {}", l.text),
                    DiffLineKind::Added => println!("{}", paint("32", &format!("+{}", l.text))),
                    DiffLineKind::Removed => println!("{}", paint("31", &format!("-{}", l.text))),
                    DiffLineKind::Gap => println!("{}", paint("36", "@@")),
                }
            }
        }
        Cmd::Restore { path, at, dry_run, yes } => {
            let (root, prefix) = locate(&store, &path)?;
            let at = parse_moment(&at, now)?;
            let scope = if prefix.is_empty() {
                Scope::All
            } else {
                Scope::Prefix(prefix)
            };
            let plan = store.plan_restore(root.id, at, &scope)?;
            println!("Rewinding {} to {} ({}):", path.display(), local_time(at), ago(at, now));
            execute(&store, &plan, dry_run, yes)?;
        }
        Cmd::Undo { path, dry_run, yes } => {
            let (root, _) = locate(&store, &path_or_cwd(path)?)?;
            let bursts = store.bursts(root.id, now - 7 * 86_400_000, BURST_MIN_FILES, BURST_GAP_MS)?;
            let Some(b) = bursts.last() else {
                bail!("no burst of changes in {} in the last week", root.path);
            };
            println!(
                "Undoing {} files changed {} ({} added, {} modified, {} deleted):",
                b.paths.len(),
                ago(b.end, now),
                b.added,
                b.modified,
                b.deleted
            );
            let plan = store.plan_restore(root.id, b.start - 1, &Scope::Paths(b.paths.clone()))?;
            execute(&store, &plan, dry_run, yes)?;
        }
        Cmd::Bursts { path, since } => {
            let (root, _) = locate(&store, &path_or_cwd(path)?)?;
            let since = parse_moment(&since, now)?;
            for b in store.bursts(root.id, since, BURST_MIN_FILES, BURST_GAP_MS)? {
                println!(
                    "{}  {:>5} files in {:>4}s  {}  {}  {}",
                    local_time(b.start),
                    b.paths.len(),
                    (b.end - b.start) / 1000,
                    paint("32", &format!("+{}", b.added)),
                    paint("33", &format!("~{}", b.modified)),
                    paint("31", &format!("-{}", b.deleted)),
                );
            }
        }
        Cmd::Status => {
            let s = store.stats()?;
            let cfg = store.settings();
            println!("store      {}", store.home().display());
            println!(
                "watcher    {}",
                if watcher_running(&store) {
                    "running"
                } else {
                    "not running"
                }
            );
            println!("folders    {}", s.roots);
            println!("versions   {}", s.versions);
            println!(
                "size       {} on disk for {} of versions",
                human_bytes(s.stored_bytes),
                human_bytes(s.original_bytes)
            );
            println!(
                "retention  {} days, up to {} MB, files up to {} MB",
                cfg.keep_days, cfg.max_store_mb, cfg.max_file_mb
            );
        }
        Cmd::Prune => {
            let r = store.prune()?;
            println!(
                "thinned {}, expired {}, evicted {}; freed {}",
                r.thinned,
                r.expired,
                r.evicted,
                human_bytes(r.bytes_freed)
            );
        }
        Cmd::Config {
            max_store_mb,
            keep_days,
            max_file_mb,
        } => {
            let mut s = store.settings();
            s.max_store_mb = max_store_mb.unwrap_or(s.max_store_mb);
            s.keep_days = keep_days.unwrap_or(s.keep_days);
            s.max_file_mb = max_file_mb.unwrap_or(s.max_file_mb);
            store.set_settings(s.clone())?;
            println!(
                "keep {} days, up to {} MB, files up to {} MB",
                s.keep_days, s.max_store_mb, s.max_file_mb
            );
        }
    }
    Ok(())
}

fn daemon(store: Store) -> Result<()> {
    let store = Arc::new(store);
    let roots = store.roots()?;
    if roots.is_empty() {
        bail!("no folders to watch yet; add one with `clessidra add <dir>`");
    }
    for r in &roots {
        eprintln!("watching {}", r.path);
    }
    let names: std::collections::HashMap<i64, String> = roots.into_iter().map(|r| (r.id, r.path)).collect();
    let _daemon = Daemon::start(store, move |e| match e {
        Event::Recorded { versions, .. } => {
            let now = now_ms();
            for v in &versions {
                print_event(v, now);
            }
        }
        Event::Burst(b) => eprintln!(
            "{} {} files changed in {} ({} deleted) - `clessidra undo` puts them back",
            paint("33;1", "burst:"),
            b.paths.len(),
            names.get(&b.root_id).map(String::as_str).unwrap_or("?"),
            b.deleted
        ),
        Event::Error(e) => eprintln!("{} {e}", paint("31", "warning:")),
    })?;
    loop {
        std::thread::park();
    }
}

fn execute(store: &Store, plan: &RestorePlan, dry_run: bool, yes: bool) -> Result<()> {
    if plan.is_empty() {
        println!("  nothing to do: it already looks like that");
        return Ok(());
    }
    const SHOW: usize = 30;
    for w in plan.write.iter().take(SHOW) {
        let verb = if w.exists {
            paint("33", "rewrite")
        } else {
            paint("32", "bring back")
        };
        println!("  {verb:>10}  {}", w.path);
    }
    for d in plan.delete.iter().take(SHOW.saturating_sub(plan.write.len())) {
        println!("  {:>10}  {d}", paint("31", "remove"));
    }
    let shown = plan.write.len().min(SHOW) + plan.delete.len().min(SHOW.saturating_sub(plan.write.len()));
    let total = plan.write.len() + plan.delete.len();
    if total > shown {
        println!("  ... and {} more", total - shown);
    }
    if dry_run {
        return Ok(());
    }
    if !yes && !confirm(&format!("Apply these {total} changes? (they can be undone too)"))? {
        return Ok(());
    }
    let r = store.apply_restore(plan)?;
    println!("wrote {}, removed {}", r.written, r.deleted);
    for (p, e) in &r.failed {
        eprintln!("{} {p}: {e}", paint("31", "failed:"));
    }
    if !r.failed.is_empty() {
        bail!("{} files could not be restored", r.failed.len());
    }
    Ok(())
}

/// The watched folder containing `path` and `path` relative to it.
fn locate(store: &Store, path: &Path) -> Result<(Root, String)> {
    let abs = std::path::absolute(path)?;
    store
        .root_for(&abs)?
        .with_context(|| format!("{} is not inside a watched folder", abs.display()))
}

fn path_or_cwd(p: Option<PathBuf>) -> Result<PathBuf> {
    match p {
        Some(p) => Ok(p),
        None => Ok(std::env::current_dir()?),
    }
}

fn version_at(store: &Store, root: &Root, rel: &str, at: i64) -> Result<Version> {
    let v = store
        .file_versions(root.id, rel)?
        .into_iter()
        .find(|v| v.ts <= at)
        .with_context(|| format!("no record of {rel} at {}", local_time(at)))?;
    if v.kind == Kind::Del {
        bail!(
            "{rel} did not exist at {} (deleted {})",
            local_time(at),
            local_time(v.ts)
        );
    }
    Ok(v)
}

fn watcher_running(store: &Store) -> bool {
    std::fs::File::open(store.home().join("daemon.lock")).is_ok_and(|f| f.try_lock_shared().is_err())
}

fn print_event(v: &Version, now: i64) {
    println!("{:>9}  {}  {}", paint("2", &ago(v.ts, now)), kind_tag(v.kind), v.path);
}

fn kind_tag(k: Kind) -> String {
    match k {
        Kind::Add => paint("32", "added   "),
        Kind::Mod => paint("33", "modified"),
        Kind::Del => paint("31", "deleted "),
    }
}

fn human_bytes(n: i64) -> String {
    let n = n as f64;
    match n {
        n if n < 1024.0 => format!("{n} B"),
        n if n < 1024.0 * 1024.0 => format!("{:.1} KB", n / 1024.0),
        n if n < 1024.0 * 1024.0 * 1024.0 => format!("{:.1} MB", n / 1024.0 / 1024.0),
        n => format!("{:.2} GB", n / 1024.0 / 1024.0 / 1024.0),
    }
}

fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("refusing to change files without confirmation; pass --yes");
    }
    eprint!("{question} [y/N] ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn paint(code: &str, s: &str) -> String {
    if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}
