#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AsyncApp, Bounds, Global, KeyBinding, QuitMode, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowOptions, prelude::*, px, size,
};
use gpui_platform::application;
use ninari_app::assets::{Assets, load_fonts};
use ninari_app::nav::{Back, Forward, Route};
use ninari_app::shell::{self, AppMsg};
use ninari_app::updates;
use ninari_app::{core::Core, root::Root};
use ninari_core::watch::Watcher;
use serde_json::json;

/// The window, while it is open: closing to the tray leaves none.
struct MainWindow(Option<WindowHandle<Root>>);

impl Global for MainWindow {}

fn main_window(cx: &App) -> Option<WindowHandle<Root>> {
    cx.try_global::<MainWindow>().and_then(|w| w.0)
}

fn open_window(core: &Arc<Core>, autoplay: bool, page: Option<Route>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let core = core.clone();
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // The player's video lies under the window and shows through it.
                window_background: WindowBackgroundAppearance::Transparent,
                app_id: Some("ninari".into()),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("ninari".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut root = Root::new(core, autoplay, window, cx);
                    if let Some(page) = page {
                        root.open(page, cx);
                    }
                    root
                })
            },
        )
        .unwrap();
    cx.set_global(MainWindow(Some(window)));
}

/// Brings the window up, opening it again when it was closed to the tray.
fn show(core: &Arc<Core>, cx: &mut App) {
    match main_window(cx) {
        Some(w) => {
            let _ = w.update(cx, |_, window, _| window.activate_window());
        }
        None => open_window(core, false, None, cx),
    }
}

fn on_msg(msg: AppMsg, core: &Arc<Core>, cx: &mut App) {
    match msg {
        AppMsg::Show => show(core, cx),
        AppMsg::Quit => {
            core.player.shutdown();
            cx.quit();
        }
        AppMsg::Link(link) => {
            show(core, cx);
            if let (Some(repo), Some(w)) = (shell::plugin_repo(&link), main_window(cx)) {
                let _ = w.update(cx, |root, window, cx| root.offer_repo(repo, window, cx));
            }
        }
    }
}

/// The player's and the devices' messages go to whichever window is open now.
fn pump(core: &Arc<Core>, cx: &mut App) {
    if let Some(mut rx) = core.player_rx.lock().unwrap().take() {
        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(msg) = rx.recv().await {
                cx.update(|cx| {
                    if let Some(w) = main_window(cx) {
                        let _ = w.update(cx, |root, window, cx| root.on_host(msg, window, cx));
                    }
                });
            }
        })
        .detach();
    }
    // A sync with another device brought titles or progress in: the lists show them.
    if let Some(mut rx) = core.devices_rx.lock().unwrap().take() {
        cx.spawn(async move |cx: &mut AsyncApp| {
            while rx.recv().await.is_some() {
                cx.update(|cx| {
                    if let Some(w) = main_window(cx) {
                        let _ = w.update(cx, |root, _, cx| root.library_changed(cx));
                    }
                });
            }
        })
        .detach();
    }
}

/// The window closed: the app stays in the tray when the user keeps it there and there is a
/// tray to come back from; the episode playing stops, nobody watches a closed window.
fn on_closed(core: &Arc<Core>, cx: &mut App) {
    if !cx.windows().is_empty() {
        return;
    }
    cx.set_global(MainWindow(None));
    let keep = core
        .setting::<bool>(shell::CLOSE_TO_TRAY_SETTING)
        .unwrap_or(true);
    if !keep || !shell::tray_up() {
        // What was playing keeps its place.
        core.player.shutdown();
        cx.quit();
        return;
    }
    let player = core.player.clone();
    core.rt.spawn(async move { player.stop().await });
    if core.setting::<bool>(shell::TRAY_HINT_SETTING) != Some(true) {
        shell::notify(
            "ninari работает в трее",
            "Пришлю уведомление, когда выйдет новая серия. Выйти можно из меню значка в трее.",
        );
        core.save_setting(shell::TRAY_HINT_SETTING, json!(true));
    }
}

/// CachyOS's ananicy-cpp runs bun and node, and with them what `bun gpui` starts, at idle CPU
/// and disk priority: the UI then stutters whenever anything else is busy. Done before any
/// thread starts, so every thread inherits the normal priority.
#[cfg(all(debug_assertions, target_os = "linux"))]
fn leave_idle_priority() {
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    const IOPRIO_BEST_EFFORT_4: libc::c_long = (2 << 13) | 4;
    unsafe {
        if libc::sched_getscheduler(0) == libc::SCHED_IDLE {
            let param = libc::sched_param { sched_priority: 0 };
            libc::sched_setscheduler(0, libc::SCHED_OTHER, &param);
            libc::setpriority(libc::PRIO_PROCESS, 0, 0);
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                0,
                IOPRIO_BEST_EFFORT_4,
            );
        }
    }
}

/// The log goes to the terminal and to ninari.log in the cache folder, kept until the next start:
/// a release on Windows has no terminal, and a failed start must leave a trace somewhere.
fn start_log() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let dir = ninari_app::core::cache_dir();
    let file =
        std::fs::create_dir_all(&dir).and_then(|_| std::fs::File::create(dir.join("ninari.log")));
    let log = tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false);
    match file {
        Ok(file) => log
            .with_writer(std::io::stderr.and(std::sync::Mutex::new(file)))
            .init(),
        Err(_) => log.init(),
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("{info}");
        default(info);
    }));
}

/// `ninari` opens the home screen; `--continue` starts the title on its continue card,
/// `--page <name>` opens a page by its `Route::parse` name ("title:61990"), and an
/// `ninari://` link is handled, by the instance already running when there is one.
fn main() -> anyhow::Result<()> {
    run().inspect_err(|e| tracing::error!("{e:#}"))
}

fn run() -> anyhow::Result<()> {
    #[cfg(all(debug_assertions, target_os = "linux"))]
    leave_idle_priority();
    start_log();
    let args: Vec<String> = std::env::args().collect();
    let link = args
        .iter()
        .find(|a| a.starts_with(&format!("{}://", shell::SCHEME)))
        .cloned();
    if shell::hand_over(link.as_deref()) {
        return Ok(());
    }
    let autoplay = args.iter().any(|a| a == "--continue");
    let page = args
        .iter()
        .position(|a| a == "--page")
        .and_then(|i| args.get(i + 1))
        .and_then(|name| Route::parse(name));
    // Lives as long as the app: the stream proxy and the sources' requests run on it.
    let rt = tokio::runtime::Runtime::new()?;
    let core = Core::open(&rt)?;
    // The download of an update installed by now.
    let _ = std::fs::remove_dir_all(ninari_app::updates::folder());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    shell::listen(rt.handle(), tx.clone());
    shell::tray(rt.handle(), tx);
    shell::register_scheme();
    Watcher {
        notify: Arc::new(shell::notify),
        db: core.db.clone(),
        library: core.library.clone(),
        catalog: core.catalog.clone(),
        resolver: core.resolver.clone(),
    }
    .spawn(rt.handle());
    application().with_assets(Assets).run(move |cx: &mut App| {
        if let Err(e) = load_fonts(cx) {
            tracing::warn!("fonts: {e:#}");
        }
        core.install(cx);
        // The app outlives its window in the tray; on_closed quits when it should.
        cx.set_quit_mode(QuitMode::Explicit);
        ninari_app::text_input::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("alt-left", Back, None),
            KeyBinding::new("alt-right", Forward, None),
        ]);
        open_window(&core, autoplay, page, cx);
        pump(&core, cx);
        let shell_core = core.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(msg) = rx.recv().await {
                cx.update(|cx| on_msg(msg, &shell_core, cx));
            }
        })
        .detach();
        if let Some(link) = link {
            on_msg(AppMsg::Link(link), &core, cx);
        }
        // A newer version, looked for a little after start unless the user turned that off.
        let update_core = core.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_secs(20))
                .await;
            if update_core.setting::<bool>(updates::AUTO_UPDATE_SETTING) == Some(false) {
                return;
            }
            match updates::check(&update_core).await {
                Ok(Some(info)) => cx.update(|cx| {
                    if let Some(w) = main_window(cx) {
                        let _ = w.update(cx, |root, _, cx| root.offer_update(info, cx));
                    }
                }),
                Ok(None) => {}
                Err(e) => tracing::info!("update check failed: {e:#}"),
            }
        })
        .detach();
        let closed_core = core.clone();
        cx.on_window_closed(move |cx, _| on_closed(&closed_core, cx))
            .detach();
    });
    Ok(())
}
