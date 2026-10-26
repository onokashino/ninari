//! One core for the whole app: the database, catalog, library, the sources with their plugins and
//! the stream proxy, on a tokio runtime of their own.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ninari_core::account::Account;
use ninari_core::catalog::anilist::AniList;
use ninari_core::catalog::aniskip::AniSkip;
use ninari_core::catalog::calendar::Schedule;
use ninari_core::catalog::details::Details;
use ninari_core::catalog::jikan::Jikan;
use ninari_core::catalog::posters::Posters;
use ninari_core::catalog::{Catalog, SHIKIMORI_DOMAINS};
use ninari_core::db::{self, Db};
use ninari_core::devices::{self, Devices, lan::Lan};
use ninari_core::error::AppError;
use ninari_core::extensions::host::ExtensionHost;
use ninari_core::library::Library;
use ninari_core::net::HttpClient;
use ninari_core::player::{Player, PlayerDeps};
use ninari_core::proxy::StreamProxy;
use ninari_core::sources::Resolver;
use ninari_core::sources::dubs::DubTable;

use crate::playback::{GpuiHost, HostMsg};

const USER_AGENT: &str = concat!(
    "ninari/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/onokashino/ninari)"
);

pub struct Core {
    pub rt: tokio::runtime::Handle,
    pub db: Db,
    pub catalog: Arc<Catalog>,
    pub posters: Arc<Posters>,
    pub host: Arc<ExtensionHost>,
    pub library: Arc<Library>,
    pub resolver: Arc<Resolver>,
    pub schedule: Arc<Schedule>,
    pub details: Arc<Details>,
    pub jikan: Arc<Jikan>,
    /// For pictures the core does not keep itself, such as video thumbnails.
    pub http: HttpClient,
    pub proxy: Arc<StreamProxy>,
    /// The player controller, drawing through `player_host`.
    pub player: Arc<Player>,
    pub player_host: Arc<GpuiHost>,
    /// Shikimori sign-in; library changes go to the list there.
    pub account: Arc<Account>,
    /// Syncing with the user's other devices on the local network.
    pub devices: Arc<Devices>,
    pub lan: Arc<Lan>,
    /// A sync brought something in; taken once by the window.
    pub devices_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>>,
    /// The player host's messages, taken once by the window.
    pub player_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<HostMsg>>>,
}

/// What other devices call this one: the computer's name.
fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Компьютер".into())
}

struct Installed(Arc<Core>);

impl gpui::Global for Installed {}

/// The folders the app's data and cache folders sit in: XDG's on Linux, AppData on Windows.
fn roots() -> (PathBuf, PathBuf) {
    let var = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .unwrap_or_default()
    };
    if cfg!(windows) {
        (var("APPDATA"), var("LOCALAPPDATA"))
    } else {
        let home = var("HOME");
        (home.join(".local/share"), home.join(".cache"))
    }
}

const NAME: &str = "com.onokashino.ninari";
const OLD_NAME: &str = "com.onokashino.anitsair";

/// Where the database and caches live.
pub fn data_dir() -> PathBuf {
    roots().0.join(NAME)
}

pub fn cache_dir() -> PathBuf {
    roots().1.join(NAME)
}

/// The app was called anitsair: its folders and database take the new name once, so the
/// library, progress and settings carry over.
fn move_old_name((data, cache): (PathBuf, PathBuf)) {
    let rename = |old: PathBuf, new: PathBuf| {
        if old.exists()
            && !new.exists()
            && let Err(e) = std::fs::rename(&old, &new)
        {
            tracing::warn!("moving {} to {}: {e}", old.display(), new.display());
        }
    };
    rename(data.join(OLD_NAME), data.join(NAME));
    rename(cache.join(OLD_NAME), cache.join(NAME));
    for part in ["", "-wal", "-shm"] {
        rename(
            data.join(NAME).join(format!("anitsair.db{part}")),
            data.join(NAME).join(format!("ninari.db{part}")),
        );
    }
}

impl Core {
    /// Opens everything; `rt` must outlive the app (the proxy and requests run on it).
    pub fn open(rt: &tokio::runtime::Runtime) -> anyhow::Result<Arc<Self>> {
        move_old_name(roots());
        let db: Db = Arc::new(Mutex::new(db::open(&data_dir().join("ninari.db"))?));
        let http = HttpClient::new(USER_AGENT);
        let catalog = Arc::new(Catalog::new(
            http.clone(),
            db.clone(),
            SHIKIMORI_DOMAINS.iter().map(|d| d.to_string()).collect(),
        ));
        let anilist = AniList::new(http.clone(), db.clone());
        let posters = Arc::new(Posters::new(
            http.clone(),
            db.clone(),
            cache_dir().join("posters"),
            anilist.clone(),
        ));
        let schedule = Arc::new(Schedule::new(catalog.clone(), anilist.clone()));
        let details = Arc::new(Details::new(catalog.clone(), anilist, posters.clone()));
        let jikan = Jikan::new(http.clone(), db.clone());
        let host = Arc::new(ExtensionHost::new(
            http.clone(),
            db.clone(),
            data_dir().join("extensions"),
        ));
        rt.block_on(host.load_installed())
            .map_err(|e| anyhow::anyhow!("plugins: {}", e.message))?;
        let resolver = Arc::new(Resolver::new(
            catalog.clone(),
            host.clone(),
            db.clone(),
            DubTable::load(&host.dubs_path()),
        ));
        let proxy = Arc::new(
            rt.block_on(StreamProxy::start(http.clone()))
                .map_err(|e| anyhow::anyhow!("proxy: {}", e.message))?,
        );
        let library = Arc::new(Library::new(db.clone()));
        let account = Account::new(http.clone(), db.clone(), library.clone());
        let devices = Devices::new(db.clone(), library.clone(), device_name());
        let lan = Lan::new(devices.clone());
        let (devices_tx, devices_rx) = tokio::sync::mpsc::unbounded_channel();
        devices.set_on_change(move || {
            let _ = devices_tx.send(());
        });
        let lan_on = db::get_setting(&db.lock().unwrap(), devices::ENABLED_SETTING)
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if lan_on {
            let lan = lan.clone();
            rt.spawn(async move {
                if let Err(e) = lan.start().await {
                    tracing::warn!("local network sync did not start: {}", e.message);
                }
            });
        }
        let (player_host, player_rx) = GpuiHost::new(account.clone());
        let player = Player::new(
            player_host.clone(),
            PlayerDeps {
                catalog: catalog.clone(),
                resolver: resolver.clone(),
                proxy: proxy.clone(),
                library: library.clone(),
                db: db.clone(),
                http: http.clone(),
                cache_dir: cache_dir(),
                jikan: jikan.clone(),
                aniskip: AniSkip::new(http.clone()),
            },
        );
        Ok(Arc::new(Core {
            rt: rt.handle().clone(),
            library,
            db,
            catalog,
            posters,
            host,
            resolver,
            schedule,
            details,
            jikan,
            http: http.clone(),
            proxy,
            player,
            player_host,
            account,
            devices,
            lan,
            devices_rx: Mutex::new(Some(devices_rx)),
            player_rx: Mutex::new(Some(player_rx)),
        }))
    }

    /// Makes the core reachable from every view.
    pub fn install(self: &Arc<Self>, cx: &mut gpui::App) {
        cx.set_global(Installed(self.clone()));
    }

    pub fn get(cx: &gpui::App) -> Arc<Core> {
        cx.global::<Installed>().0.clone()
    }

    /// A setting as it is saved, `None` when it never was.
    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let saved = db::get_setting(&self.db.lock().unwrap(), key)
            .ok()
            .flatten()?;
        serde_json::from_value(saved).ok()
    }

    /// Saves a setting in the background.
    pub fn save_setting(self: &Arc<Self>, key: &'static str, value: serde_json::Value) {
        let db = self.db.clone();
        self.rt.spawn(async move {
            if let Err(e) = db::set_setting(&db.lock().unwrap(), key, &value) {
                tracing::warn!("setting {key}: {e:#}");
            }
        });
    }

    /// Runs `f` on the core's runtime; a GPUI task awaits the result.
    pub fn run<T, F, G>(
        self: &Arc<Self>,
        f: G,
    ) -> impl Future<Output = anyhow::Result<T>> + use<T, F, G>
    where
        G: FnOnce(Arc<Core>) -> F,
        T: Send + 'static,
        F: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        let task = self.rt.spawn(f(self.clone()));
        async move { Ok(task.await??) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_names_data_moves_once() {
        let home = std::env::temp_dir().join(format!("ninari-home-{}", std::process::id()));
        let roots = (home.join("data"), home.join("cache"));
        let old = roots.0.join(OLD_NAME);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("anitsair.db"), "library").unwrap();
        std::fs::write(old.join("anitsair.db-wal"), "log").unwrap();
        std::fs::create_dir_all(roots.1.join(OLD_NAME).join("posters")).unwrap();

        move_old_name(roots.clone());
        let data = roots.0.join(NAME);
        assert_eq!(
            std::fs::read_to_string(data.join("ninari.db")).unwrap(),
            "library"
        );
        assert!(data.join("ninari.db-wal").exists() && !old.exists());
        assert!(roots.1.join(NAME).join("posters").is_dir());

        // Run again, and with an old folder back: what is there now stays.
        std::fs::create_dir_all(&old).unwrap();
        move_old_name(roots);
        assert_eq!(
            std::fs::read_to_string(data.join("ninari.db")).unwrap(),
            "library"
        );
        std::fs::remove_dir_all(&home).unwrap();
    }
}
