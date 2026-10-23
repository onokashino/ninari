//! The GPUI app around ninari_core's player. The player's events come to the window's thread
//! through a channel, and when it wants mpv drawn in the window, libmpv starts there and draws into
//! a Wayland surface under the window (see video); mpv's IPC socket does the rest.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ninari_core::account::Account;
use ninari_core::error::AppError;
use ninari_core::player::{PlayerEvent, PlayerHost, Rendered};
use tokio::sync::{mpsc, oneshot};

pub enum HostMsg {
    Event(PlayerEvent),
    /// Start libmpv with these options and say when it draws.
    Start(Vec<(String, String)>, oneshot::Sender<Result<(), AppError>>),
    /// The player is done with libmpv.
    Stop,
}

pub struct GpuiHost {
    tx: mpsc::UnboundedSender<HostMsg>,
    screen_height: AtomicU32,
    account: Arc<Account>,
}

impl GpuiHost {
    pub fn new(account: Arc<Account>) -> (Arc<Self>, mpsc::UnboundedReceiver<HostMsg>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let host = Arc::new(Self {
            tx,
            screen_height: AtomicU32::new(1080),
            account,
        });
        (host, rx)
    }

    /// The display the window is on, in device pixels, for Anime4K.
    pub fn set_screen_height(&self, height: u32) {
        self.screen_height.store(height.max(1), Ordering::Relaxed);
    }
}

fn gone() -> AppError {
    AppError::new("Player", "окно закрылось")
}

/// `name` in the program's own folder.
fn beside_program(name: &str) -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(name))
}

impl PlayerHost for GpuiHost {
    fn emit(&self, event: PlayerEvent) {
        let _ = self.tx.send(HostMsg::Event(event));
    }

    fn screen_height(&self) -> u32 {
        self.screen_height.load(Ordering::Relaxed)
    }

    /// The mpv a release carries beside the program (scripts/package.sh); otherwise the
    /// system's, or on Windows the one scripts/fetch-mpv.ps1 downloads.
    fn mpv_program(&self) -> PathBuf {
        if let Some(own) = beside_program("mpv").filter(|p| p.is_file()) {
            return own;
        }
        if cfg!(windows) {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/binaries/mpv-x86_64-pc-windows-msvc.exe"
            ))
        } else {
            "mpv".into()
        }
    }

    /// Beside the program in a release (scripts/package.sh puts it there); the repository's
    /// copy, with the shaders from scripts/fetch-shaders.sh, while developing.
    fn mpv_config_dir(&self) -> PathBuf {
        beside_program("mpv-config")
            .filter(|dir| dir.is_dir())
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/mpv-config")))
    }

    /// Episodes watched go to the list on Shikimori too.
    fn watched(&self, shikimori_id: i64) {
        self.account.push(shikimori_id);
    }

    fn can_render(&self) -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    }

    fn start_rendered(
        &self,
        _label: &str,
        options: Vec<(String, String)>,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Rendered>, AppError>> + Send>> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            tx.send(HostMsg::Start(options, reply))
                .map_err(|_| gone())?;
            answer.await.unwrap_or_else(|_| Err(gone()))?;
            Ok(Box::new(Drawn(tx)) as Box<dyn Rendered>)
        })
    }
}

/// libmpv drawing in the window; dropping it there stops it.
struct Drawn(mpsc::UnboundedSender<HostMsg>);

impl Rendered for Drawn {
    fn stop(self: Box<Self>) {
        let _ = self.0.send(HostMsg::Stop);
    }
}
