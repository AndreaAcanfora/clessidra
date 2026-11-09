//! The desktop app: a window over the store, a tray icon, and the watcher running in the
//! background for as long as the app is open (closing the window only hides it).

use std::sync::{Arc, Mutex};

use clessidra_core::watch::{BURST_GAP_MS, BURST_MIN_FILES, Daemon, Event};
use clessidra_core::{
    Bucket, Burst, Diff, FileAt, RestorePlan, RestoreReport, Root, Scope, Settings, Stats, Store, Version, now_ms,
};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

struct App {
    store: Arc<Store>,
    daemon: Mutex<Option<Daemon>>,
    /// Why the watcher is not running, if it is not.
    daemon_error: Mutex<Option<String>>,
}

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Run blocking store work off the main thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[derive(Serialize)]
struct Status {
    watching: bool,
    error: Option<String>,
    stats: Stats,
    home: String,
}

#[tauri::command]
async fn status(app: State<'_, App>) -> Res<Status> {
    let store = app.store.clone();
    let stats = blocking(move || store.stats()).await?;
    Ok(Status {
        watching: app.daemon.lock().unwrap().is_some(),
        error: app.daemon_error.lock().unwrap().clone(),
        stats,
        home: app.store.home().display().to_string(),
    })
}

#[tauri::command]
async fn roots(app: State<'_, App>) -> Res<Vec<Root>> {
    let store = app.store.clone();
    blocking(move || store.roots()).await
}

#[tauri::command]
async fn add_root(handle: AppHandle, app: State<'_, App>, path: String) -> Res<Root> {
    let store = app.store.clone();
    let root = blocking({
        let store = store.clone();
        move || store.add_root(std::path::Path::new(&path))
    })
    .await?;
    // The first snapshot can take a while on a big folder; the window hears when it is done.
    let r = root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = store.snapshot(&r).map_err(err);
        let _ = handle.emit("snapshot-done", (r.id, result));
        if let Some(d) = handle.state::<App>().daemon.lock().unwrap().as_ref() {
            d.reload();
        }
    });
    Ok(root)
}

#[tauri::command]
async fn remove_root(app: State<'_, App>, id: i64) -> Res<()> {
    let store = app.store.clone();
    blocking(move || store.remove_root(id)).await?;
    reload(&app);
    Ok(())
}

#[tauri::command]
async fn set_paused(app: State<'_, App>, id: i64, paused: bool) -> Res<()> {
    let store = app.store.clone();
    blocking(move || store.set_paused(id, paused)).await?;
    reload(&app);
    Ok(())
}

fn reload(app: &App) {
    if let Some(d) = app.daemon.lock().unwrap().as_ref() {
        d.reload();
    }
}

#[tauri::command]
async fn timeline(app: State<'_, App>, root_id: i64, from: i64, to: i64, buckets: usize) -> Res<Vec<Bucket>> {
    let store = app.store.clone();
    blocking(move || store.timeline(root_id, from, to, buckets)).await
}

#[tauri::command]
async fn events(
    app: State<'_, App>,
    root_id: i64,
    prefix: String,
    before: i64,
    cursor: Option<(i64, i64)>,
    limit: usize,
) -> Res<Vec<Version>> {
    let store = app.store.clone();
    blocking(move || store.events(root_id, &prefix, before, cursor, limit)).await
}

#[tauri::command]
async fn tree_at(app: State<'_, App>, root_id: i64, at: i64) -> Res<Vec<FileAt>> {
    let store = app.store.clone();
    blocking(move || store.tree_at(root_id, "", at)).await
}

#[tauri::command]
async fn file_versions(app: State<'_, App>, root_id: i64, path: String) -> Res<Vec<Version>> {
    let store = app.store.clone();
    blocking(move || store.file_versions(root_id, &path)).await
}

#[tauri::command]
async fn diff_version(app: State<'_, App>, id: i64) -> Res<Diff> {
    let store = app.store.clone();
    blocking(move || store.diff_version(id)).await
}

#[tauri::command]
async fn diff_with_disk(app: State<'_, App>, id: i64) -> Res<Diff> {
    let store = app.store.clone();
    blocking(move || store.diff_with_disk(id)).await
}

#[derive(Serialize)]
struct Preview {
    /// None for binary content.
    text: Option<String>,
    truncated: bool,
    size: usize,
}

#[tauri::command]
async fn preview(app: State<'_, App>, hash: String) -> Res<Preview> {
    const MAX: usize = 256 * 1024;
    let store = app.store.clone();
    blocking(move || {
        let bytes = store.blob(&hash)?;
        let size = bytes.len();
        let cut = &bytes[..size.min(MAX)];
        let text = if cut.contains(&0) {
            None
        } else {
            Some(String::from_utf8_lossy(cut).into_owned())
        };
        Ok(Preview {
            text,
            truncated: size > MAX,
            size,
        })
    })
    .await
}

#[tauri::command]
async fn bursts(app: State<'_, App>, root_id: i64, since: i64) -> Res<Vec<Burst>> {
    let store = app.store.clone();
    blocking(move || store.bursts(root_id, since, BURST_MIN_FILES, BURST_GAP_MS)).await
}

#[tauri::command]
async fn plan_restore(app: State<'_, App>, root_id: i64, at: i64, scope: Scope) -> Res<RestorePlan> {
    let store = app.store.clone();
    blocking(move || store.plan_restore(root_id, at, &scope)).await
}

#[tauri::command]
async fn apply_restore(app: State<'_, App>, plan: RestorePlan) -> Res<RestoreReport> {
    let store = app.store.clone();
    blocking(move || store.apply_restore(&plan)).await
}

#[tauri::command]
fn settings(app: State<'_, App>) -> Settings {
    app.store.settings()
}

#[tauri::command]
fn set_settings(app: State<'_, App>, settings: Settings) -> Res<()> {
    app.store.set_settings(settings).map_err(err)
}

#[tauri::command]
async fn reveal(app: State<'_, App>, root_id: i64, path: String) -> Res<String> {
    let store = app.store.clone();
    blocking(move || {
        let root = store.root(root_id)?;
        let mut p = root.dir().to_path_buf();
        p.extend(path.split('/').filter(|s| !s.is_empty()));
        Ok(p.display().to_string())
    })
    .await
}

fn show_window(handle: &AppHandle) {
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn start_daemon(handle: &AppHandle) {
    let app = handle.state::<App>();
    let h = handle.clone();
    let names = Arc::new(Mutex::new(std::collections::HashMap::<i64, String>::new()));
    let result = Daemon::start(app.store.clone(), move |e| match e {
        Event::Recorded { root_id, versions } => {
            let _ = h.emit("recorded", (root_id, versions.len()));
        }
        Event::Burst(b) => {
            let _ = h.emit("burst", &b);
            if b.deleted >= 5 || b.paths.len() >= 25 {
                let folder = {
                    let mut names = names.lock().unwrap();
                    if !names.contains_key(&b.root_id)
                        && let Ok(r) = h.state::<App>().store.root(b.root_id)
                    {
                        let name = r.dir().file_name().map(|n| n.to_string_lossy().into_owned());
                        names.insert(b.root_id, name.unwrap_or(r.path));
                    }
                    names.get(&b.root_id).cloned().unwrap_or_default()
                };
                let title = if b.deleted > 0 {
                    format!("{} files deleted in {folder}", b.deleted)
                } else {
                    format!("{} files changed in {folder}", b.paths.len())
                };
                let _ = h
                    .notification()
                    .builder()
                    .title(title)
                    .body("Open clessidra to undo it in one click.")
                    .show();
            }
        }
        Event::Error(e) => log::warn!("{e}"),
    });
    match result {
        Ok(d) => {
            *app.daemon.lock().unwrap() = Some(d);
            *app.daemon_error.lock().unwrap() = None;
        }
        Err(e) => *app.daemon_error.lock().unwrap() = Some(format!("{e:#}")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store = match Store::open_default() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("clessidra: cannot open the store: {e:#}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .manage(App {
            store,
            daemon: Mutex::new(None),
            daemon_error: Mutex::new(None),
        })
        .setup(|app| {
            let handle = app.handle().clone();
            start_daemon(&handle);

            let open = MenuItem::with_id(app, "open", "Open clessidra", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit (stop keeping history)", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true)
                .tooltip("clessidra")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;

            if std::env::args().any(|a| a == "--background")
                && let Some(w) = app.get_webview_window("main")
            {
                let _ = w.hide();
            }
            let _ = handle.emit("started", now_ms());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the history going; quitting is in the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            status,
            roots,
            add_root,
            remove_root,
            set_paused,
            timeline,
            events,
            tree_at,
            file_versions,
            diff_version,
            diff_with_disk,
            preview,
            bursts,
            plan_restore,
            apply_restore,
            settings,
            set_settings,
            reveal,
        ])
        .build(tauri::generate_context!())
        .expect("error while building clessidra")
        .run(|app, event| {
            // macOS: clicking the dock icon brings the window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_window(app);
            }
            let _ = (app, event);
        });
}
