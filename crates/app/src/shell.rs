//! The app around its window: one instance at a time, `ninari://` links, the tray icon,
//! notifications, and the window opening again after it was closed to the tray.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc::UnboundedSender;

/// What the tray, a second launch or a link asks of the app.
pub enum AppMsg {
    Show,
    Quit,
    Link(String),
}

pub const SCHEME: &str = "ninari";
/// Whether closing the window keeps the app in the tray; on unless the user turns it off.
pub const CLOSE_TO_TRAY_SETTING: &str = "app.close_to_tray";
/// Set once the user was told the app keeps running in the tray.
pub const TRAY_HINT_SETTING: &str = "app.tray_hint_shown";

static TRAY_UP: AtomicBool = AtomicBool::new(false);

/// Whether the tray icon is on screen: without it a closed window could not come back.
pub fn tray_up() -> bool {
    TRAY_UP.load(Ordering::Relaxed)
}

/// The repository a plugins link offers (`ninari://plugins?repo=<index.json URL>`): an
/// https URL of a `.json` index, nothing else. Nothing is installed from a link alone.
pub fn plugin_repo(link: &str) -> Option<String> {
    let link = url::Url::parse(link).ok()?;
    if link.scheme() != SCHEME || link.host_str() != Some("plugins") {
        return None;
    }
    let repo = link
        .query_pairs()
        .find(|(k, _)| k == "repo")
        .map(|(_, v)| v.into_owned())?;
    let parsed = url::Url::parse(&repo).ok()?;
    (parsed.scheme() == "https" && parsed.path().ends_with(".json")).then_some(repo)
}

/// Shows a notification. On a thread of its own: the D-Bus call blocks, and blocking inside the
/// core's runtime is not allowed.
pub fn notify(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let icon = icon_file();
        let mut note = notify_rust::Notification::new();
        note.appname("ninari").summary(&title).body(&body);
        // Windows names a notification by the app id of the Start menu's shortcut (scripts/ninari.iss).
        #[cfg(windows)]
        note.app_id(APP_ID);
        if let Some(icon) = &icon {
            note.icon(&icon.to_string_lossy());
        }
        if let Err(e) = note.show() {
            tracing::warn!("cannot show a notification: {e}");
        }
    });
}

/// The Windows app id, the same as the installer gives the Start menu shortcut.
#[cfg(windows)]
pub const APP_ID: &str = "onokashino.ninari";

/// The app's stamp as a file, for notifications to show.
fn icon_file() -> Option<PathBuf> {
    const ICON: &[u8] = include_bytes!("../assets/icon.png");
    let path = crate::core::cache_dir().join("icon.png");
    // Written again when the stamp changed in a new version.
    if std::fs::read(&path).ok().as_deref() != Some(ICON) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, ICON).ok()?;
    }
    Some(path)
}

#[cfg(unix)]
fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ninari.sock")
}

/// Hands a link, or a wish to see the window, to an instance already running. True when there
/// was one: this launch then has nothing else to do.
#[cfg(unix)]
pub fn hand_over(link: Option<&str>) -> bool {
    use std::io::Write;
    std::os::unix::net::UnixStream::connect(socket_path())
        .and_then(|mut s| writeln!(s, "{}", link.unwrap_or("show")))
        .is_ok()
}

/// One pipe per user: another user's ninari on the same machine is an instance of its own.
#[cfg(windows)]
fn pipe_name() -> String {
    format!(
        r"\\.\pipe\ninari-{}",
        std::env::var("USERNAME").unwrap_or_default()
    )
}

#[cfg(windows)]
pub fn hand_over(link: Option<&str>) -> bool {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .open(pipe_name())
        .and_then(|mut p| writeln!(p, "{}", link.unwrap_or("show")))
        .is_ok()
}

/// What a later launch asked for.
fn message(line: &str) -> AppMsg {
    let line = line.trim();
    if line.starts_with(&format!("{SCHEME}://")) {
        AppMsg::Link(line.to_string())
    } else {
        AppMsg::Show
    }
}

/// Listens for later launches: each one sends a line, a link or "show".
#[cfg(unix)]
pub fn listen(rt: &tokio::runtime::Handle, tx: UnboundedSender<AppMsg>) {
    use tokio::io::AsyncBufReadExt;
    let path = socket_path();
    // Left by an instance that did not exit cleanly: nobody answered on it just now.
    let _ = std::fs::remove_file(&path);
    rt.spawn(async move {
        let listener = match tokio::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("single instance socket: {e}");
                return;
            }
        };
        while let Ok((stream, _)) = listener.accept().await {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            if let Ok(Some(line)) = lines.next_line().await
                && tx.send(message(&line)).is_err()
            {
                break;
            }
        }
    });
}

#[cfg(windows)]
pub fn listen(rt: &tokio::runtime::Handle, tx: UnboundedSender<AppMsg>) {
    use tokio::io::AsyncBufReadExt;
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = pipe_name();
    rt.spawn(async move {
        let mut server = match ServerOptions::new().first_pipe_instance(true).create(&name) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("single instance pipe: {e}");
                return;
            }
        };
        // Each launch takes the pipe it connected to; the next one waits on a fresh one.
        while server.connect().await.is_ok() {
            let Ok(next) = ServerOptions::new().create(&name) else {
                break;
            };
            let client = std::mem::replace(&mut server, next);
            let mut lines = tokio::io::BufReader::new(client).lines();
            if let Ok(Some(line)) = lines.next_line().await
                && tx.send(message(&line)).is_err()
            {
                break;
            }
        }
    });
}

/// The tray icon: a click brings the window back, its menu opens or quits the app.
#[cfg(target_os = "linux")]
pub fn tray(rt: &tokio::runtime::Handle, tx: UnboundedSender<AppMsg>) {
    use ksni::TrayMethods;
    use std::sync::OnceLock;

    struct Tray(UnboundedSender<AppMsg>);

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "ninari".into()
        }
        fn title(&self) -> String {
            "ninari".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            icon().into_iter().collect()
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.0.send(AppMsg::Show);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            vec![
                ksni::menu::StandardItem {
                    label: "Открыть ninari".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.0.send(AppMsg::Show);
                    }),
                    ..Default::default()
                }
                .into(),
                ksni::menu::StandardItem {
                    label: "Выход".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.0.send(AppMsg::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    /// The app's stamp as ARGB32 in network order, as the tray protocol wants it.
    fn icon() -> Option<ksni::Icon> {
        let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
            .ok()?
            .into_rgba8();
        let (width, height) = img.dimensions();
        let data = img
            .pixels()
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect();
        Some(ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        })
    }

    static HANDLE: OnceLock<ksni::Handle<Tray>> = OnceLock::new();
    rt.spawn(async move {
        match Tray(tx).spawn().await {
            Ok(handle) => {
                let _ = HANDLE.set(handle);
                TRAY_UP.store(true, Ordering::Relaxed);
            }
            // No tray host (a desktop without a status bar): closing the window quits.
            Err(e) => tracing::info!("no tray icon: {e}"),
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub fn tray(_rt: &tokio::runtime::Handle, _tx: UnboundedSender<AppMsg>) {
    // The Windows tray goes with the Windows build.
}

/// Makes `ninari://` links open this program: a desktop entry naming it, and the scheme's default
/// handler.
#[cfg(target_os = "linux")]
pub fn register_scheme() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    else {
        return;
    };
    // The app was called anitsair: its handler for anitsair:// links goes.
    let _ = std::fs::remove_file(data.join("applications/anitsair-handler.desktop"));
    let file = data.join("applications/ninari-handler.desktop");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=ninari\nTerminal=false\nNoDisplay=true\n\
         MimeType=x-scheme-handler/{SCHEME}\nExec=\"{}\" %u\n",
        exe.display()
    );
    if std::fs::read_to_string(&file).ok().as_deref() == Some(entry.as_str()) {
        return;
    }
    std::thread::spawn(move || {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&file, entry) {
            tracing::warn!("cannot register {SCHEME}:// links: {e}");
            return;
        }
        let _ = std::process::Command::new("xdg-mime")
            .args([
                "default",
                "ninari-handler.desktop",
                &format!("x-scheme-handler/{SCHEME}"),
            ])
            .status();
    });
}

/// The same on Windows: the scheme under the user's own classes, so no admin rights are asked;
/// and the app id with its name and icon, which notifications show even without the installer's
/// shortcut.
#[cfg(windows)]
pub fn register_scheme() {
    use std::os::windows::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let scheme = format!(r"HKCU\Software\Classes\{SCHEME}");
    let app = format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}");
    let open = format!("\"{}\" \"%1\"", exe.display());
    let icon = icon_file()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    std::thread::spawn(move || {
        let entries: [(String, &[&str]); 5] = [
            (scheme.clone(), &["/ve", "/d", "URL:ninari"]),
            (scheme.clone(), &["/v", "URL Protocol", "/d", ""]),
            (
                format!(r"{scheme}\shell\open\command"),
                &["/ve", "/d", &open],
            ),
            (app.clone(), &["/v", "DisplayName", "/d", "ninari"]),
            (app, &["/v", "IconUri", "/d", &icon]),
        ];
        for (key, args) in entries {
            let done = std::process::Command::new("reg")
                .arg("add")
                .arg(&key)
                .args(args)
                .arg("/f")
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .status();
            if !done.as_ref().is_ok_and(|s| s.success()) {
                tracing::warn!("cannot register {key}: {done:?}");
                return;
            }
        }
    });
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn register_scheme() {}

#[cfg(test)]
mod tests {
    use super::*;

    const GITHUB: &str =
        "https://raw.githubusercontent.com/onokashino/ninari/master/extensions/dist/index.json";

    #[test]
    fn a_plugins_link_names_a_repository() {
        let encoded: String = url::form_urlencoded::byte_serialize(GITHUB.as_bytes()).collect();
        assert_eq!(
            plugin_repo(&format!("ninari://plugins?repo={encoded}")).as_deref(),
            Some(GITHUB)
        );
        assert_eq!(
            plugin_repo(&format!("ninari://plugins/?repo={GITHUB}")).as_deref(),
            Some(GITHUB)
        );
    }

    #[test]
    fn anything_else_is_ignored() {
        for bad in [
            "ninari://plugins",
            "ninari://plugins?repo=",
            "ninari://other?repo=https://example.org/index.json",
            "https://plugins?repo=https://example.org/index.json",
            "ninari://plugins?repo=http://example.org/index.json",
            "ninari://plugins?repo=file:///C:/index.json",
            "ninari://plugins?repo=https://example.org/plugin.js",
            "not a url",
        ] {
            assert_eq!(plugin_repo(bad), None, "{bad}");
        }
    }
}
