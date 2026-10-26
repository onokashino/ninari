//! Drives mpv over its JSON IPC socket: mpv as a process (in an app window by `--wid`, or in a
//! window of its own), or libmpv the app runs inside its window. The app draws its own controls
//! above the video; this module loads episodes through the stream proxy, saves progress, streams
//! player state to the UI, plays the next episode and applies Anime4K. What it needs from the
//! app around it is a [`PlayerHost`].

pub mod audio;
mod gate;
pub mod ipc;
mod quality;
mod quiet;
pub mod session;
pub mod thumbs;
pub mod upscale;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::catalog::aniskip::{self, AniSkip};
use crate::catalog::jikan::{skippable, Jikan};
use crate::catalog::Catalog;
use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::extensions::types::{SkipTimes, Stream};
use crate::library::Library;
use crate::net::{FetchRequest, HttpClient};
use crate::proxy::StreamProxy;
use crate::sources::{ResolvedStream, Resolver};
use audio::{AudioFx, AudioTrack};
use gate::PlayGate;
use ipc::{Ipc, MpvEvent};
use session::{Action, Playback};
use thumbs::{parse_playlist, prefix_bytes, sample, Playlist, Thumb, ThumbsEvent, Url};
use upscale::{by_key, key_of, Tier, Upscale, FLAVORS, METHODS, MODES};

pub const TIER_SETTING: &str = "player.anime4k";
pub const UPSCALER_SETTING: &str = "player.upscaler";
pub const MODE_SETTING: &str = "player.anime4k_mode";
pub const FLAVOR_SETTING: &str = "player.artcnn_flavor";
pub const VOLUME_SETTING: &str = "player.volume";
pub const MUTED_SETTING: &str = "player.muted";
pub const FIT_SETTING: &str = "player.fit";
pub const QUALITY_SETTING: &str = "player.quality";
pub const AUDIO_SETTING: &str = "player.audio";
pub const SPEED_SETTING: &str = "player.speed";
pub const SKIP_OPENING_SETTING: &str = "player.skip_opening";
pub const SKIP_ENDING_SETTING: &str = "player.skip_ending";
/// Going on by itself passes over fillers and recaps (marks from Jikan).
pub const SKIP_FILLERS_SETTING: &str = "player.skip_fillers";
/// Playback speeds on offer.
const SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// The offered speed nearest to `v`.
pub fn clamp_speed(v: f64) -> f64 {
    if !v.is_finite() {
        return 1.0;
    }
    SPEEDS
        .into_iter()
        .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
        .unwrap_or(1.0)
}
/// Audio delay per dub: some dubs are out of sync with the picture.
pub const AUDIO_DELAY_SETTING: &str = "player.audio_delay";
/// Seek previews of this many recently played episodes stay in the cache.
const THUMBS_KEEP: usize = 40;

/// Player preferences that survive restarts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPrefs {
    pub volume: f64,
    pub muted: bool,
    /// Crop the video to fill the window instead of showing black bars.
    pub fill: bool,
    /// Highest quality to play (a slow connection); `None` plays the best available.
    pub quality_cap: Option<u32>,
    pub audio: AudioFx,
    pub speed: f64,
    /// Jump over openings and endings that the source has timings for.
    pub skip_opening: bool,
    pub skip_ending: bool,
    pub skip_fillers: bool,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self {
            volume: 100.0,
            muted: false,
            fill: false,
            quality_cap: None,
            audio: AudioFx::default(),
            speed: 1.0,
            skip_opening: false,
            skip_ending: false,
            skip_fillers: false,
        }
    }
}

impl PlayerPrefs {
    /// Reads the preferences through `get`, a settings lookup; broken values fall back to defaults.
    pub fn from_settings(get: impl Fn(&str) -> Option<serde_json::Value>) -> Self {
        let d = Self::default();
        let flag = |k: &str| get(k).and_then(|v| v.as_bool());
        Self {
            volume: get(VOLUME_SETTING)
                .and_then(|v| v.as_f64())
                .filter(|v| (0.0..=130.0).contains(v))
                .unwrap_or(d.volume),
            muted: flag(MUTED_SETTING).unwrap_or(d.muted),
            fill: get(FIT_SETTING)
                .and_then(|v| v.as_str().map(|s| s == "fill"))
                .unwrap_or(d.fill),
            quality_cap: get(QUALITY_SETTING)
                .and_then(|v| v.as_u64())
                .and_then(|q| u32::try_from(q).ok())
                .filter(|&q| q > 0),
            audio: AudioFx::from_value(get(AUDIO_SETTING)),
            speed: get(SPEED_SETTING)
                .and_then(|v| v.as_f64())
                .filter(|v| SPEEDS.contains(v))
                .unwrap_or(d.speed),
            skip_opening: flag(SKIP_OPENING_SETTING).unwrap_or(d.skip_opening),
            skip_ending: flag(SKIP_ENDING_SETTING).unwrap_or(d.skip_ending),
            skip_fillers: flag(SKIP_FILLERS_SETTING).unwrap_or(d.skip_fillers),
        }
    }

    pub fn mpv_args(&self) -> Vec<String> {
        let mut args = vec![
            format!("--volume={}", self.volume),
            format!("--mute={}", if self.muted { "yes" } else { "no" }),
            format!("--panscan={}", if self.fill { "1" } else { "0" }),
            format!("--speed={}", self.speed),
        ];
        let af = audio::filter_chain(&self.audio);
        if !af.is_empty() {
            args.push(format!("--af={af}"));
        }
        args
    }
}

const OBSERVED: [&str; 10] = [
    "time-pos",
    "duration",
    "pause",
    "height",
    "volume",
    "mute",
    "paused-for-cache",
    "demuxer-cache-time",
    "track-list",
    "aid",
];

/// What the player tells the app's player screen.
pub enum PlayerEvent {
    State(PlayerState),
    Thumbs(ThumbsEvent),
    Progress(ProgressEvent),
    Error {
        kind: String,
        message: String,
    },
    /// Playback stopped: the screen goes back.
    Stopped,
    /// An opening or ending was jumped over; `from` lets the user go back to it.
    Skipped {
        part: &'static str,
        from: f64,
    },
    /// The last available episode played to its end.
    Finished {
        shikimori_id: i64,
        episode: i64,
    },
}

/// libmpv the app runs inside its window, drawing under the controls.
pub trait Rendered: Send {
    fn stop(self: Box<Self>);
}

/// What the player needs from the app around it.
pub trait PlayerHost: Send + Sync + 'static {
    fn emit(&self, event: PlayerEvent);
    /// The screen's height in pixels, to pick Anime4K for it.
    fn screen_height(&self) -> u32;
    /// The mpv program: for playing in a window and for seek previews.
    fn mpv_program(&self) -> PathBuf;
    /// The folder with mpv.conf and the Anime4K shaders.
    fn mpv_config_dir(&self) -> PathBuf;
    /// The native handle of window `label` to embed mpv into with `--wid`, where that works.
    fn window_handle(&self, _label: &str) -> Result<Option<isize>, AppError> {
        Ok(None)
    }
    /// Whether the app can run libmpv inside its window.
    fn can_render(&self) -> bool {
        false
    }
    /// Starts libmpv inside window `label` with these options.
    fn start_rendered(
        &self,
        _label: &str,
        _options: Vec<(String, String)>,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Rendered>, AppError>> + Send>> {
        Box::pin(async { Err(AppError::new("Player", "libmpv недоступен")) })
    }
    /// An episode was watched to the end: the title is synced with the account.
    fn watched(&self, _shikimori_id: i64) {}
}

/// "--key=value" (as passed to the mpv program) into libmpv's (key, value).
pub fn option_pair(arg: &str) -> Option<(&str, &str)> {
    let arg = arg.strip_prefix("--")?;
    Some(arg.split_once('=').unwrap_or((arg, "yes")))
}

/// The options of an mpv.conf for libmpv; the output and the graphics API are libmpv's own.
/// mpv's `~~/` is its config folder, which libmpv started without `config=yes` does not know
/// (it turns `~~/x` into `x`): paths under it are given in full.
fn in_config_dir(dir: &std::path::Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|p| match p.strip_prefix("~~/") {
            Some(rest) => dir
                .join(rest)
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_string(),
            None => p.clone(),
        })
        .collect()
}

pub fn config_options(conf: &str) -> Vec<(String, String)> {
    conf.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| !matches!(k.trim(), "vo" | "gpu-api"))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub shikimori_id: i64,
    pub episode: i64,
    pub position: f64,
    pub duration: f64,
    pub watched: bool,
}

/// Everything the player screen needs to draw its controls.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub shikimori_id: i64,
    pub episode: i64,
    pub title: String,
    pub dub_key: String,
    pub dub_name: String,
    pub quality: Option<u32>,
    pub extension_id: String,
    pub position: f64,
    pub duration: f64,
    pub cached_until: f64,
    pub paused: bool,
    pub buffering: bool,
    pub volume: f64,
    pub muted: bool,
    pub upscale: bool,
    /// How the picture is upscaled when it is: the method, mode, tier and flavor chosen.
    pub upscale_cfg: Upscale,
    /// What runs right now, "Anime4K A+A, HQ"; `None` while off.
    pub upscale_label: Option<String>,
    pub opening: Option<(f64, f64)>,
    pub ending: Option<(f64, f64)>,
    pub prev: Option<i64>,
    pub next: Option<i64>,
    pub fill: bool,
    /// Qualities the current source offers, best first.
    pub qualities: Vec<u32>,
    /// The quality the user capped playback at; `None` is "auto".
    pub quality_cap: Option<u32>,
    pub audio_fx: AudioFx,
    /// Seconds the sound is shifted by for this dub; positive plays it later.
    pub audio_delay: f64,
    /// Audio tracks to choose from; empty when the file has just one.
    pub audio_tracks: Vec<AudioTrack>,
    pub audio_track: Option<i64>,
    pub speed: f64,
    pub skip_opening: bool,
    pub skip_ending: bool,
    pub skip_fillers: bool,
}

struct Current {
    /// The mpv process this playback runs in; events from older processes are ignored.
    session: u64,
    playback: Playback,
    token: String,
    title: String,
    dub_name: String,
    quality: Option<u32>,
    /// Every stream of the source, best first; `url` is the one playing.
    streams: Vec<Stream>,
    url: String,
    skip: Option<SkipTimes>,
    shaders_on: bool,
    /// The shaders given to mpv, to put back after a look at the original.
    shaders: Vec<String>,
    upscale: Upscale,
    auto_checked: bool,
    prefetched: Option<(i64, ResolvedStream)>,
    audio_delay: f64,
    audio_tracks: Vec<AudioTrack>,
    audio_track: Option<i64>,
}

impl Current {
    fn snapshot(&self, prefs: PlayerPrefs) -> PlayerState {
        let p = &self.playback;
        PlayerState {
            shikimori_id: p.shikimori_id,
            episode: p.episode,
            title: self.title.clone(),
            dub_key: p.dub_key.clone(),
            dub_name: self.dub_name.clone(),
            quality: self.quality,
            extension_id: p.extension_id.clone(),
            position: p.position,
            duration: p.duration,
            cached_until: p.cached_until,
            paused: p.paused,
            buffering: p.buffering,
            volume: p.volume,
            muted: p.muted,
            upscale: self.shaders_on,
            upscale_cfg: self.upscale,
            upscale_label: self
                .shaders_on
                .then(|| upscale::label(&self.upscale, p.video_height)),
            opening: self.skip.as_ref().and_then(|s| s.opening),
            ending: self.skip.as_ref().and_then(|s| s.ending),
            prev: p.prev(),
            next: p.next_in_order(),
            fill: prefs.fill,
            qualities: quality::offered(&self.streams),
            quality_cap: prefs.quality_cap,
            audio_fx: prefs.audio,
            audio_delay: self.audio_delay,
            audio_tracks: self.audio_tracks.clone(),
            audio_track: self.audio_track,
            speed: prefs.speed,
            skip_opening: prefs.skip_opening,
            skip_ending: prefs.skip_ending,
            skip_fillers: prefs.skip_fillers,
        }
    }
}

struct Running {
    ipc: Arc<Ipc>,
    closed: Arc<AtomicBool>,
    /// Label of the window mpv is embedded into.
    host: String,
    session: u64,
}

/// Where mpv's picture goes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    /// Into an app window by its handle (Windows), under its controls.
    Window(isize),
    /// libmpv inside the app drawing under its controls.
    Rendered,
    /// A window of its own, with mpv's usual keys and on-screen controls.
    Own,
}

/// mpv under the app's controls draws nothing of its own and ignores keys and the mouse: the app
/// draws all controls. No script runs either: a leftover one in the config folder (uosc) would
/// paint its idle screen over the app.
fn embedded_args(target: Target, pipe: &str, config_dir: &str) -> Vec<String> {
    let mut args = vec![
        "--idle=yes".into(),
        "--terminal=no".into(),
        "--load-scripts=no".into(),
        format!("--input-ipc-server={pipe}"),
        format!("--config-dir={config_dir}"),
    ];
    // libmpv's output can only start once the window gives it somewhere to draw.
    if target != Target::Rendered {
        args.push("--force-window=yes".into());
    }
    if let Target::Window(wid) = target {
        args.extend([
            format!("--wid={wid}"),
            "--gpu-api=d3d11".into(),
            // Clicks and the pointer over the video belong to the app's controls above it.
            "--input-cursor-passthrough=yes".into(),
        ]);
    }
    if target != Target::Own {
        args.extend(
            [
                "--input-default-bindings=no",
                "--input-vo-keyboard=no",
                "--input-cursor=no",
                "--cursor-autohide=no",
                "--osc=no",
                "--osd-level=0",
            ]
            .map(String::from),
        );
    }
    args
}

/// A running mpv: a process, or libmpv drawing into a window.
enum MpvRun {
    Process(tokio::process::Child),
    Rendered(Box<dyn Rendered>),
}

impl MpvRun {
    fn kill(self) {
        match self {
            MpvRun::Process(mut child) => {
                let _ = child.start_kill();
            }
            MpvRun::Rendered(r) => r.stop(),
        }
    }
}

/// The mpv program with no console window of its own on Windows.
fn mpv_process(program: &Path) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// `loadfile` options: the window title and where to start.
fn load_options(title: &str, episode: i64, start: f64) -> String {
    let media_title = format!("{title} · Серия {episode}");
    let options = format!("force-media-title=%{}%{media_title}", media_title.len());
    if start > 1.0 {
        format!("start={start:.1},{options}")
    } else {
        options
    }
}

/// A range to skip when skipping is on.
fn auto_range(on: bool, range: Option<(f64, f64)>) -> Option<(f64, f64)> {
    range.filter(|_| on)
}

fn io_err(e: std::io::Error) -> AppError {
    AppError::new("Player", format!("mpv: {e}"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Services the player works with.
pub struct PlayerDeps {
    pub catalog: Arc<Catalog>,
    pub resolver: Arc<Resolver>,
    pub proxy: Arc<StreamProxy>,
    pub library: Arc<Library>,
    pub db: Db,
    pub http: HttpClient,
    pub cache_dir: PathBuf,
    pub jikan: Arc<Jikan>,
    pub aniskip: Arc<AniSkip>,
}

pub struct Player {
    app: Arc<dyn PlayerHost>,
    catalog: Arc<Catalog>,
    resolver: Arc<Resolver>,
    proxy: Arc<StreamProxy>,
    library: Arc<Library>,
    db: Db,
    running: tokio::sync::Mutex<Option<Running>>,
    child: Mutex<Option<MpvRun>>,
    current: Mutex<Option<Current>>,
    prefs: Mutex<Option<PlayerPrefs>>,
    http: HttpClient,
    cache_dir: PathBuf,
    thumbs: Mutex<Option<(ThumbsEvent, Arc<AtomicBool>)>>,
    /// Window the next playback should appear in ("main" or the separate "player" window).
    host: Mutex<String>,
    sessions: AtomicU64,
    gate: PlayGate,
    /// Streams looked up ahead for the episode on screen, by dub: the pointer resting on a dub
    /// in the player's list starts one, so the switch finds it ready.
    warm: Mutex<
        std::collections::HashMap<
            (i64, i64, String),
            Arc<tokio::sync::OnceCell<Option<ResolvedStream>>>,
        >,
    >,
    jikan: Arc<Jikan>,
    aniskip: Arc<AniSkip>,
}

impl Player {
    pub fn new(app: Arc<dyn PlayerHost>, deps: PlayerDeps) -> Arc<Self> {
        Arc::new(Self {
            app,
            catalog: deps.catalog,
            resolver: deps.resolver,
            proxy: deps.proxy,
            library: deps.library,
            db: deps.db,
            running: tokio::sync::Mutex::new(None),
            child: Mutex::new(None),
            current: Mutex::new(None),
            prefs: Mutex::new(None),
            http: deps.http,
            cache_dir: deps.cache_dir,
            thumbs: Mutex::new(None),
            host: Mutex::new("main".into()),
            sessions: AtomicU64::new(0),
            gate: PlayGate::default(),
            warm: Mutex::default(),
            jikan: deps.jikan,
            aniskip: deps.aniskip,
        })
    }

    fn with_current<T>(&self, f: impl FnOnce(&mut Current) -> T) -> Option<T> {
        self.current.lock().unwrap().as_mut().map(f)
    }

    pub fn state(&self) -> Option<PlayerState> {
        let prefs = self.prefs();
        self.with_current(|c| c.snapshot(prefs))
    }

    fn emit_state(&self, force: bool) {
        let prefs = self.prefs();
        let snapshot = self
            .with_current(|c| {
                if force {
                    c.playback.mark_dirty();
                }
                c.playback.should_emit(now_ms()).then(|| c.snapshot(prefs))
            })
            .flatten();
        if let Some(s) = snapshot {
            self.app.emit(PlayerEvent::State(s));
        }
    }

    fn prefs(&self) -> PlayerPrefs {
        let mut cached = self.prefs.lock().unwrap();
        if let Some(p) = *cached {
            return p;
        }
        let conn = self.db.lock().unwrap();
        let p = PlayerPrefs::from_settings(|k| crate::db::get_setting(&conn, k).ok().flatten());
        *cached = Some(p);
        p
    }

    fn update_prefs(&self, f: impl FnOnce(&mut PlayerPrefs)) {
        let before = self.prefs();
        let mut after = before;
        f(&mut after);
        if after == before {
            return;
        }
        *self.prefs.lock().unwrap() = Some(after);
        let conn = self.db.lock().unwrap();
        let fit = if after.fill { "fill" } else { "fit" };
        for (k, v) in [
            (VOLUME_SETTING, json!(after.volume)),
            (MUTED_SETTING, json!(after.muted)),
            (FIT_SETTING, json!(fit)),
            (
                QUALITY_SETTING,
                after.quality_cap.map_or(json!("auto"), |q| json!(q)),
            ),
            (AUDIO_SETTING, json!(after.audio)),
            (SPEED_SETTING, json!(after.speed)),
            (SKIP_OPENING_SETTING, json!(after.skip_opening)),
            (SKIP_ENDING_SETTING, json!(after.skip_ending)),
            (SKIP_FILLERS_SETTING, json!(after.skip_fillers)),
        ] {
            if let Err(e) = crate::db::set_setting(&conn, k, &v) {
                tracing::warn!("cannot save {k}: {e}");
            }
        }
    }

    /// The upscaler as the settings say; read at each episode's start, since the settings
    /// page writes them too.
    fn upscale_prefs(&self) -> Upscale {
        let conn = self.db.lock().unwrap();
        let get = |k: &str| -> String {
            crate::db::get_setting(&conn, k)
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        let d = Upscale::default();
        Upscale {
            method: by_key(&METHODS, &get(UPSCALER_SETTING)).unwrap_or(d.method),
            mode: by_key(&MODES, &get(MODE_SETTING)).unwrap_or(d.mode),
            tier: Tier::parse(&get(TIER_SETTING)),
            flavor: by_key(&FLAVORS, &get(FLAVOR_SETTING)).unwrap_or(d.flavor),
        }
    }

    fn save_upscale(&self, u: &Upscale) {
        let conn = self.db.lock().unwrap();
        for (k, v) in [
            (UPSCALER_SETTING, key_of(&METHODS, u.method)),
            (MODE_SETTING, key_of(&MODES, u.mode)),
            (TIER_SETTING, u.tier.key()),
            (FLAVOR_SETTING, key_of(&FLAVORS, u.flavor)),
        ] {
            if let Err(e) = crate::db::set_setting(&conn, k, &json!(v)) {
                tracing::warn!("cannot save {k}: {e}");
            }
        }
    }

    fn audio_delay_for(&self, dub_key: &str) -> f64 {
        let conn = self.db.lock().unwrap();
        let saved = crate::db::get_setting(&conn, AUDIO_DELAY_SETTING)
            .ok()
            .flatten();
        audio::delay_for(saved.as_ref(), dub_key)
    }

    fn save_audio_delay(&self, dub_key: &str, delay: f64) {
        let conn = self.db.lock().unwrap();
        let saved = crate::db::get_setting(&conn, AUDIO_DELAY_SETTING)
            .ok()
            .flatten();
        let next = audio::remember_delay(saved, dub_key, delay);
        if let Err(e) = crate::db::set_setting(&conn, AUDIO_DELAY_SETTING, &next) {
            tracing::warn!("cannot save {AUDIO_DELAY_SETTING}: {e}");
        }
    }

    /// Switches sound effects on or off while playing; they also apply to later episodes.
    pub async fn set_audio(&self, fx: AudioFx) -> Result<(), AppError> {
        self.update_prefs(|p| p.audio = fx);
        if let Some(ipc) = self.ipc().await {
            ipc.command(json!(["set_property", "af", audio::filter_chain(&fx)]))
                .await
                .map_err(io_err)?;
        }
        self.emit_state(true);
        Ok(())
    }

    fn screen_height(&self) -> u32 {
        self.app.screen_height()
    }

    /// Makes the next playback appear in the given window.
    pub fn set_host(&self, label: &str) {
        *self.host.lock().unwrap() = label.to_string();
    }

    pub fn host(&self) -> String {
        self.host.lock().unwrap().clone()
    }

    async fn ensure_running(self: &Arc<Self>) -> Result<(Arc<Ipc>, u64), AppError> {
        let host = self.host();
        let mut guard = self.running.lock().await;
        if let Some(r) = guard.as_ref() {
            if !r.closed.load(Ordering::SeqCst) && r.host == host {
                return Ok((r.ipc.clone(), r.session));
            }
            // mpv is embedded into another window: it cannot move, so it is restarted.
            r.closed.store(true, Ordering::SeqCst);
        }
        if let Some(old) = self.child.lock().unwrap().take() {
            old.kill();
        }
        let session = self.sessions.fetch_add(1, Ordering::SeqCst) + 1;
        let name = format!("ninari-{}", hex::encode(rand::random::<[u8; 8]>()));
        #[cfg(windows)]
        let pipe = format!(r"\\.\pipe\{name}");
        #[cfg(unix)]
        let pipe = std::env::temp_dir()
            .join(format!("{name}.sock"))
            .to_string_lossy()
            .into_owned();
        let config_path = self.app.mpv_config_dir();
        let config_dir = config_path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        let rendered = self.app.can_render();
        let target = match self.app.window_handle(&host)? {
            Some(wid) => Target::Window(wid),
            None if rendered => Target::Rendered,
            None => Target::Own,
        };
        let args: Vec<String> = embedded_args(target, &pipe, &config_dir)
            .into_iter()
            .chain(self.prefs().mpv_args())
            .collect();
        let run = match target {
            Target::Rendered => {
                let conf =
                    std::fs::read_to_string(config_path.join("mpv.conf")).unwrap_or_default();
                let options = config_options(&conf)
                    .into_iter()
                    .chain(args.iter().filter_map(|a| {
                        option_pair(a).map(|(k, v)| (k.to_string(), v.to_string()))
                    }))
                    .collect();
                MpvRun::Rendered(self.app.start_rendered(&host, options).await?)
            }
            _ => {
                // Its output is not read: nothing must fill a pipe and stall it.
                let child = mpv_process(&self.app.mpv_program())
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|e| {
                        AppError::new("Player", format!("не удалось запустить mpv: {e}"))
                    })?;
                MpvRun::Process(child)
            }
        };
        *self.child.lock().unwrap() = Some(run);
        let (ipc, events) = Ipc::connect(&pipe, Duration::from_secs(10))
            .await
            .map_err(io_err)?;
        for (i, name) in OBSERVED.iter().enumerate() {
            ipc.observe(i as u64 + 1, name).await.map_err(io_err)?;
        }
        let closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(
            self.clone()
                .event_loop(ipc.clone(), events, closed.clone(), session),
        );
        *guard = Some(Running {
            ipc: ipc.clone(),
            closed,
            host,
            session,
        });
        Ok((ipc, session))
    }

    async fn ipc(&self) -> Option<Arc<Ipc>> {
        let guard = self.running.lock().await;
        guard
            .as_ref()
            .filter(|r| !r.closed.load(Ordering::SeqCst))
            .map(|r| r.ipc.clone())
    }

    /// Plays an episode. Without `dub_key` the remembered dub of the title is used, or the best one.
    pub async fn play(
        self: &Arc<Self>,
        id: i64,
        episode: i64,
        dub_key: Option<String>,
        start: Option<f64>,
    ) -> Result<(), AppError> {
        let ticket = self.gate.begin();
        let dub_key = match dub_key {
            Some(k) => k,
            None => match self.library.get_entry(id)?.and_then(|e| e.chosen_dub_key) {
                Some(k) => k,
                None => self.resolver.default_dub(id).await?.ok_or_else(|| {
                    AppError::new("NoSource", "Ни одно расширение не нашло этот тайтл")
                })?,
            },
        };
        let prefetched = self
            .with_current(|c| {
                let same = c.playback.shikimori_id == id && c.playback.dub_key == dub_key;
                c.prefetched.take().filter(|(n, _)| same && *n == episode)
            })
            .flatten();
        let began = std::time::Instant::now();
        let warm = self
            .warm
            .lock()
            .unwrap()
            .remove(&(id, episode, dub_key.clone()));
        let warmed = match warm {
            // Still looking it up: wait for that rather than start over.
            Some(cell) => cell.get_or_init(|| async { None }).await.clone(),
            None => None,
        };
        let resolved = match prefetched.map(|(_, r)| r).or(warmed) {
            Some(r) => r,
            None => self.resolver.resolve(id, episode, &dub_key).await?,
        };
        // Looked up for another episode: of no use any more.
        self.warm
            .lock()
            .unwrap()
            .retain(|(i, e, _), _| (*i, *e) == (id, episode));
        // Episodes opened from the panel, N/P or autoplay pick up where they were left.
        let start = match start {
            Some(s) => s,
            None => {
                let progress = self.library.progress(id)?;
                crate::library::resume_position(progress.iter().find(|p| p.episode == episode))
            }
        };
        let found = began.elapsed();
        let result = self
            .start(ticket, id, episode, dub_key, resolved, start)
            .await;
        tracing::info!(
            "episode {episode} of {id}: stream found in {} ms, loading after {} ms",
            found.as_millis(),
            began.elapsed().as_millis()
        );
        result
    }

    async fn start(
        self: &Arc<Self>,
        ticket: u64,
        id: i64,
        episode: i64,
        dub_key: String,
        resolved: ResolvedStream,
        start: f64,
    ) -> Result<(), AppError> {
        // None of these waits on another: together they take as long as the slowest.
        let (anime, running, in_dub, skip) = tokio::join!(
            self.catalog.get(id),
            self.ensure_running(),
            self.resolver.episodes_in_dub(id, &dub_key),
            self.resolver.skip_for(id, episode),
        );
        let (anime, (ipc, session)) = (anime?, running?);
        let available = in_dub?.into_iter().max().unwrap_or(episode).max(episode);
        let final_episode = (anime.status.as_deref() == Some("released") && anime.episodes > 0)
            .then_some(anime.episodes);
        let ending_start = skip.as_ref().and_then(|s| s.ending).map(|(from, _)| from);
        let Some(_loading) = self.gate.commit(ticket).await else {
            return Ok(());
        };
        let streams: Vec<Stream> = std::iter::once(resolved.stream.clone())
            .chain(resolved.alternatives.iter().cloned())
            .collect();
        let chosen = streams[quality::pick(&streams, self.prefs().quality_cap)].clone();
        let local = self.proxy.open_session(&chosen.url, chosen.headers.clone());
        let token = StreamProxy::token_of(&local).unwrap_or_default();

        let playback = Playback::new(
            id,
            episode,
            dub_key.clone(),
            resolved.extension_id.clone(),
            available,
            final_episode,
            ending_start,
        )
        .resume_at(start)
        .auto_skip(
            auto_range(
                self.prefs().skip_opening,
                skip.as_ref().and_then(|s| s.opening),
            ),
            auto_range(
                self.prefs().skip_ending,
                skip.as_ref().and_then(|s| s.ending),
            ),
        );
        // mpv lives on between episodes with the volume it had; a new episode starts from the
        // saved one, which mpv is told again below, so the slider and the sound agree.
        let prefs = self.prefs();
        let mut playback = playback;
        playback.volume = prefs.volume;
        playback.muted = prefs.muted;
        let title = anime.russian.clone().unwrap_or_else(|| anime.name.clone());
        let audio_delay = self.audio_delay_for(&dub_key);
        let previous = self.current.lock().unwrap().replace(Current {
            session,
            playback,
            token,
            title: title.clone(),
            dub_name: resolved.dub_name.clone(),
            quality: chosen.quality,
            streams,
            url: chosen.url,
            skip,
            shaders_on: false,
            shaders: Vec::new(),
            upscale: self.upscale_prefs(),
            auto_checked: false,
            prefetched: None,
            audio_delay,
            audio_tracks: Vec::new(),
            audio_track: None,
        });
        if let Some(prev) = previous {
            self.save(&prev.playback);
            self.proxy.close_session(&prev.token);
        }
        self.refresh_fillers(id, anime.status.as_deref() != Some("released"));

        let options = load_options(&title, episode, start);
        ipc.command(json!(["set_property", "pause", false]))
            .await
            .map_err(io_err)?;
        ipc.command(json!(["set_property", "audio-delay", audio_delay]))
            .await
            .map_err(io_err)?;
        ipc.command(json!(["set_property", "volume", prefs.volume]))
            .await
            .map_err(io_err)?;
        ipc.command(json!(["set_property", "mute", prefs.muted]))
            .await
            .map_err(io_err)?;
        ipc.command(json!(["loadfile", local, "replace", -1, options]))
            .await
            .map_err(io_err)?;
        self.library.set_dub(id, &dub_key)?;
        self.emit_state(true);
        self.start_thumbs(id, episode, &dub_key, &resolved);
        Ok(())
    }

    pub fn thumbnails(&self) -> Option<ThumbsEvent> {
        self.thumbs
            .lock()
            .unwrap()
            .as_ref()
            .map(|(ev, _)| ev.clone())
    }

    fn start_thumbs(
        self: &Arc<Self>,
        id: i64,
        episode: i64,
        dub_key: &str,
        resolved: &ResolvedStream,
    ) {
        let lightest = std::iter::once(&resolved.stream)
            .chain(resolved.alternatives.iter())
            .min_by_key(|s| s.quality.unwrap_or(u32::MAX))
            .cloned()
            .unwrap_or_else(|| resolved.stream.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let empty = ThumbsEvent {
            shikimori_id: id,
            episode,
            items: vec![],
        };
        if let Some((_, old)) = self.thumbs.lock().unwrap().replace((empty, cancel.clone())) {
            old.store(true, Ordering::SeqCst);
        }
        let safe_dub: String = dub_key.chars().filter(|c| c.is_alphanumeric()).collect();
        let dir = self
            .cache_dir
            .join("thumbs")
            .join(format!("{id}-{episode}-{safe_dub}"));
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me
                .generate_thumbs(id, episode, &lightest, &dir, &cancel)
                .await
            {
                tracing::info!("seek previews for {id}/{episode}: {e}");
            }
        });
    }

    fn publish_thumbs(&self, id: i64, episode: i64, mut items: Vec<Thumb>, cancel: &AtomicBool) {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        items.sort_by(|a, b| a.t.total_cmp(&b.t));
        let event = ThumbsEvent {
            shikimori_id: id,
            episode,
            items,
        };
        if let Some((ev, _)) = self.thumbs.lock().unwrap().as_mut() {
            if ev.shikimori_id == id && ev.episode == episode {
                *ev = event.clone();
            }
        }
        self.app.emit(PlayerEvent::Thumbs(event));
    }

    async fn get_public(
        &self,
        url: &Url,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<reqwest::Response, AppError> {
        let req = FetchRequest {
            url: url.to_string(),
            method: "GET".into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            body: None,
            timeout: None,
        };
        self.http
            .send_guarded(req, &|_| Ok(()))
            .await
            .map_err(|e| AppError::new("Blocked", e.message))
    }

    async fn fetch_text(
        &self,
        url: &Url,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<String, AppError> {
        self.get_public(url, headers)
            .await?
            .error_for_status()
            .map_err(|e| AppError::new("Blocked", e.to_string()))?
            .text()
            .await
            .map_err(|e| AppError::new("Blocked", e.to_string()))
    }

    async fn generate_thumbs(
        &self,
        id: i64,
        episode: i64,
        stream: &Stream,
        dir: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), AppError> {
        std::fs::create_dir_all(dir)?;
        thumbs::mark_used(dir);
        if let Some(root) = dir.parent().map(Path::to_path_buf) {
            tokio::task::spawn_blocking(move || thumbs::prune(&root, THUMBS_KEEP));
        }
        let list_file = dir.join("thumbs.json");
        if let Ok(json) = std::fs::read_to_string(&list_file) {
            if let Ok(items) = serde_json::from_str::<Vec<Thumb>>(&json) {
                self.publish_thumbs(id, episode, items, cancel);
                return Ok(());
            }
        }
        let mut url =
            Url::parse(&stream.url).map_err(|e| AppError::new("Player", e.to_string()))?;
        let mut body = self.fetch_text(&url, &stream.headers).await?;
        if let Playlist::Master(variants) = parse_playlist(&body, &url) {
            let Some((_, lowest)) = variants.into_iter().min_by_key(|(bw, _)| *bw) else {
                return Err(AppError::new("Player", "пустой master-плейлист"));
            };
            url = lowest;
            body = self.fetch_text(&url, &stream.headers).await?;
        }
        let Playlist::Media(segments) = parse_playlist(&body, &url) else {
            return Err(AppError::new("Player", "не удалось разобрать плейлист"));
        };
        let picks: Vec<(usize, thumbs::Segment)> = sample(&segments, 150)
            .into_iter()
            .map(|(i, s)| (i, s.clone()))
            .collect();
        let planned = picks.len();
        let limit = prefix_bytes(stream.quality);
        let items: Mutex<Vec<Thumb>> = Mutex::new(Vec::new());
        use futures_util::StreamExt;
        futures_util::stream::iter(picks)
            .for_each_concurrent(4, |(i, seg)| {
                let items = &items;
                async move {
                    if cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    match self.make_thumb(&seg, i, dir, limit, &stream.headers).await {
                        Ok(path) => {
                            let snapshot = {
                                let mut list = items.lock().unwrap();
                                list.push(Thumb { t: seg.start, path });
                                list.len().is_multiple_of(8).then(|| list.clone())
                            };
                            if let Some(list) = snapshot {
                                self.publish_thumbs(id, episode, list, cancel);
                            }
                        }
                        Err(e) => tracing::debug!("preview {i}: {e}"),
                    }
                }
            })
            .await;
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut done = items.into_inner().unwrap();
        done.sort_by(|a, b| a.t.total_cmp(&b.t));
        if thumbs::complete_enough(done.len(), planned) {
            std::fs::write(&list_file, serde_json::to_string(&done)?)?;
        }
        self.publish_thumbs(id, episode, done, cancel);
        Ok(())
    }

    async fn make_thumb(
        &self,
        seg: &thumbs::Segment,
        i: usize,
        dir: &Path,
        limit: usize,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<String, AppError> {
        let out = dir.join(format!("{i:05}.jpg"));
        if out.exists() {
            return Ok(out.to_string_lossy().into_owned());
        }
        let mut resp = self
            .get_public(&seg.url, headers)
            .await?
            .error_for_status()
            .map_err(|e| AppError::new("Blocked", e.to_string()))?;
        // Only the start of the segment is needed; dropping the response aborts the download.
        let mut buf = Vec::with_capacity(limit);
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| AppError::new("Blocked", e.to_string()))?
        {
            buf.extend_from_slice(&chunk);
            if buf.len() >= limit {
                break;
            }
        }
        drop(resp);
        let part = dir.join(format!("{i:05}.part.ts"));
        let tmp = dir.join(format!("{i:05}.out"));
        std::fs::write(&part, &buf)?;
        let _ = std::fs::remove_dir_all(&tmp);
        let output = quiet::run(
            &self.app.mpv_program(),
            vec![
                "--no-config".to_string(),
                "--terminal=no".into(),
                "--no-audio".into(),
                "--hwdec=no".into(),
                "--vo=image".into(),
                "--vo-image-format=jpg".into(),
                "--vo-image-jpeg-quality=72".into(),
                format!("--vo-image-outdir={}", tmp.display()),
                "--frames=1".into(),
                "--vf=scale=320:-2".into(),
                part.to_string_lossy().into_owned(),
            ],
        )
        .await
        .map_err(|e| AppError::new("Player", e.to_string()));
        let produced = tmp.join("00000001.jpg");
        let result = match output {
            Ok(_) if produced.exists() => std::fs::rename(&produced, &out)
                .map(|_| out.to_string_lossy().into_owned())
                .map_err(AppError::from),
            Ok(_) => Err(AppError::new("Player", "mpv не вытащил кадр")),
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    fn save(&self, p: &Playback) {
        let progress = p.progress(now_ts());
        if let Err(e) = self
            .library
            .save_progress(&progress, p.ending_start, p.final_episode)
        {
            tracing::warn!("cannot save progress: {e}");
        } else if progress.watched {
            self.app.watched(progress.shikimori_id);
        }
        self.app.emit(PlayerEvent::Progress(ProgressEvent {
            shikimori_id: progress.shikimori_id,
            episode: progress.episode,
            position: progress.position_sec,
            duration: progress.duration_sec,
            watched: progress.watched,
        }));
    }

    fn report(&self, e: AppError) {
        tracing::warn!("player: {e}");
        self.app.emit(PlayerEvent::Error {
            kind: e.kind.to_string(),
            message: e.message.clone(),
        });
    }

    /// Another episode of the title playing, in the same dub.
    async fn play_episode(self: &Arc<Self>, episode: i64) -> Result<(), AppError> {
        let Some((id, dub)) =
            self.with_current(|c| (c.playback.shikimori_id, c.playback.dub_key.clone()))
        else {
            return Ok(());
        };
        self.play(id, episode, Some(dub), None).await
    }

    /// Which episodes going on by itself passes over: what is cached at once, then fresh marks
    /// from Jikan when they come. Nothing when the user did not ask for it.
    fn refresh_fillers(self: &Arc<Self>, id: i64, ongoing: bool) {
        if !self.prefs().skip_fillers {
            self.with_current(|c| c.playback.set_skip_episodes(Default::default()));
            return;
        }
        if let Some(cached) = self.jikan.cached(id) {
            self.with_current(|c| {
                if c.playback.shikimori_id == id {
                    c.playback.set_skip_episodes(skippable(&cached.marks));
                }
            });
        }
        let me = self.clone();
        tokio::spawn(async move {
            let marks = me.jikan.marks(id, ongoing).await;
            let on = me.prefs().skip_fillers;
            me.with_current(|c| {
                if on && c.playback.shikimori_id == id {
                    c.playback.set_skip_episodes(skippable(&marks));
                }
            });
        });
    }

    async fn play_relative(self: &Arc<Self>, delta: i64) -> Result<(), AppError> {
        let Some((id, episode, dub)) = self.with_current(|c| {
            (
                c.playback.shikimori_id,
                c.playback.episode + delta,
                c.playback.dub_key.clone(),
            )
        }) else {
            return Ok(());
        };
        if episode < 1 {
            return Ok(());
        }
        self.play(id, episode, Some(dub), None).await
    }

    /// Switches the dub of the current episode and continues from the same moment.
    /// Starts looking up the episode on screen in `dub_key`, for a switch that may follow.
    pub fn warm_dub(self: &Arc<Self>, dub_key: String) {
        let Some((id, episode)) =
            self.with_current(|c| (c.playback.shikimori_id, c.playback.episode))
        else {
            return;
        };
        let cell = self
            .warm
            .lock()
            .unwrap()
            .entry((id, episode, dub_key.clone()))
            .or_default()
            .clone();
        if cell.initialized() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            cell.get_or_init(|| async {
                me.resolver
                    .resolve(id, episode, &dub_key)
                    .await
                    .map_err(|e| tracing::info!("looking up {dub_key} ahead: {}", e.message))
                    .ok()
            })
            .await;
        });
    }

    pub async fn set_dub(self: &Arc<Self>, dub_key: String) -> Result<(), AppError> {
        let Some((id, episode, position)) = self.with_current(|c| {
            (
                c.playback.shikimori_id,
                c.playback.episode,
                c.playback.position,
            )
        }) else {
            return Err(AppError::new("Player", "сейчас ничего не играет"));
        };
        self.play(id, episode, Some(dub_key), Some(position)).await
    }

    /// Stops playback if `label` is the window that shows it. A window that handed playback
    /// to another one must not stop it when its own player screen goes away.
    pub async fn stop_from(&self, label: &str) {
        if self.host() == label {
            self.stop().await;
        }
    }

    /// Stops playback (the user left the player screen). mpv stays alive for the next episode.
    pub async fn stop(&self) {
        let _loading = self.gate.cancel().await;
        if let Some((_, cancel)) = self.thumbs.lock().unwrap().take() {
            cancel.store(true, Ordering::SeqCst);
        }
        let last = self.current.lock().unwrap().take();
        if let Some(cur) = last {
            self.save(&cur.playback);
            self.proxy.close_session(&cur.token);
        }
        if let Some(ipc) = self.ipc().await {
            let _ = ipc.command(json!(["stop"])).await;
        }
        self.app.emit(PlayerEvent::Stopped);
    }

    /// Kills mpv when the app exits.
    pub fn shutdown(&self) {
        if let Some(cur) = self.current.lock().unwrap().take() {
            self.save(&cur.playback);
        }
        if let Some(child) = self.child.lock().unwrap().take() {
            child.kill();
        }
    }

    /// Remote-control actions from the player screen.
    pub async fn control(
        self: &Arc<Self>,
        action: &str,
        value: Option<f64>,
    ) -> Result<(), AppError> {
        let ipc = self
            .ipc()
            .await
            .ok_or_else(|| AppError::new("Player", "плеер не запущен"))?;
        let v = value.unwrap_or(0.0);
        let cmd = match action {
            "toggle_pause" => json!(["cycle", "pause"]),
            "seek" => json!(["seek", v.max(0.0), "absolute"]),
            "seek_by" => json!(["seek", v, "relative"]),
            "volume" => json!(["set_property", "volume", v.clamp(0.0, 130.0)]),
            "toggle_mute" => json!(["cycle", "mute"]),
            "toggle_fill" => {
                let fill = !self.prefs().fill;
                self.update_prefs(|p| p.fill = fill);
                ipc.command(json!([
                    "set_property",
                    "panscan",
                    if fill { 1.0 } else { 0.0 }
                ]))
                .await
                .map_err(io_err)?;
                self.emit_state(true);
                return Ok(());
            }
            "skip_opening" => {
                let end = self
                    .with_current(|c| c.skip.as_ref().and_then(|s| s.opening).map(|(_, e)| e))
                    .flatten()
                    .ok_or_else(|| AppError::new("Player", "у серии нет таймингов опенинга"))?;
                json!(["seek", end, "absolute"])
            }
            "next" => return self.play_relative(1).await,
            "prev" => return self.play_relative(-1).await,
            "toggle_upscale" => {
                self.toggle_upscale(&ipc).await;
                return Ok(());
            }
            // Tuning from the player's panel: each change shows at once and becomes the default.
            "upscale_method" | "upscale_mode" | "upscale_flavor" | "upscale_tier" => {
                let i = v.max(0.0) as usize;
                let Some(mut u) = self.with_current(|c| c.upscale) else {
                    return Ok(());
                };
                match action {
                    "upscale_method" => u.method = METHODS.get(i).map_or(u.method, |m| m.0),
                    "upscale_mode" => u.mode = MODES.get(i).map_or(u.mode, |m| m.0),
                    "upscale_flavor" => u.flavor = FLAVORS.get(i).map_or(u.flavor, |f| f.0),
                    _ => {
                        u.tier = [Tier::Off, Tier::Fast, Tier::Hq, Tier::Ultra]
                            .get(i)
                            .copied()
                            .unwrap_or(u.tier)
                    }
                }
                // Picking a method, mode or flavor means wanting to see it.
                if action != "upscale_tier" && u.tier == Tier::Off {
                    u.tier = Tier::Fast;
                }
                self.with_current(|c| c.upscale = u);
                self.save_upscale(&u);
                self.apply_upscale(&ipc, true).await;
                return Ok(());
            }
            // Held to see the original picture; let go to see the upscaled one again.
            "upscale_compare" => {
                let back = self
                    .with_current(|c| (v < 0.5 && c.shaders_on).then(|| c.shaders.clone()))
                    .flatten()
                    .unwrap_or_default();
                ipc.command(json!(["set_property", "glsl-shaders", back]))
                    .await
                    .map_err(io_err)?;
                return Ok(());
            }
            "quality" => {
                let cap = (v >= 1.0).then_some(v as u32);
                self.update_prefs(|p| p.quality_cap = cap);
                return self.switch_quality(&ipc, cap).await;
            }
            "audio_delay" => {
                let delay = audio::clamp_delay(v);
                let dub = self
                    .with_current(|c| {
                        c.audio_delay = delay;
                        c.playback.dub_key.clone()
                    })
                    .ok_or_else(|| AppError::new("Player", "сейчас ничего не играет"))?;
                self.save_audio_delay(&dub, delay);
                ipc.command(json!(["set_property", "audio-delay", delay]))
                    .await
                    .map_err(io_err)?;
                self.emit_state(true);
                return Ok(());
            }
            "audio_track" => json!(["set_property", "aid", v as i64]),
            "speed" => {
                let speed = clamp_speed(v);
                self.update_prefs(|p| p.speed = speed);
                ipc.command(json!(["set_property", "speed", speed]))
                    .await
                    .map_err(io_err)?;
                self.emit_state(true);
                return Ok(());
            }
            "toggle_skip_fillers" => {
                self.update_prefs(|p| p.skip_fillers = !p.skip_fillers);
                if let Some(id) = self.with_current(|c| c.playback.shikimori_id) {
                    self.refresh_fillers(id, false);
                }
                self.emit_state(true);
                return Ok(());
            }
            "toggle_skip_opening" | "toggle_skip_ending" => {
                let opening = action == "toggle_skip_opening";
                self.update_prefs(|p| {
                    if opening {
                        p.skip_opening = !p.skip_opening
                    } else {
                        p.skip_ending = !p.skip_ending
                    }
                });
                let prefs = self.prefs();
                self.with_current(|c| {
                    let skip = c.skip.clone().unwrap_or_default();
                    c.playback.set_auto_skip(
                        auto_range(prefs.skip_opening, skip.opening),
                        auto_range(prefs.skip_ending, skip.ending),
                    );
                });
                self.emit_state(true);
                return Ok(());
            }
            other => {
                return Err(AppError::new(
                    "BadRequest",
                    format!("неизвестное действие {other}"),
                ))
            }
        };
        ipc.command(cmd).await.map_err(io_err)
    }

    /// Plays the stream that fits `cap` from the same moment, if it is not the one playing.
    async fn switch_quality(&self, ipc: &Ipc, cap: Option<u32>) -> Result<(), AppError> {
        let Some(_loading) = self.gate.commit(self.gate.current()).await else {
            return Ok(());
        };
        let next = self
            .with_current(|c| {
                let s = c.streams.get(quality::pick(&c.streams, cap))?.clone();
                let title = c.title.clone();
                (s.url != c.url).then_some((s, title, c.playback.episode, c.playback.position))
            })
            .flatten();
        let Some((stream, title, episode, position)) = next else {
            self.emit_state(true);
            return Ok(());
        };
        let local = self.proxy.open_session(&stream.url, stream.headers.clone());
        let token = StreamProxy::token_of(&local).unwrap_or_default();
        let old = self.with_current(|c| {
            c.quality = stream.quality;
            c.url = stream.url.clone();
            std::mem::replace(&mut c.token, token)
        });
        if let Some(old) = old {
            self.proxy.close_session(&old);
        }
        ipc.command(json!([
            "loadfile",
            local,
            "replace",
            -1,
            load_options(&title, episode, position)
        ]))
        .await
        .map_err(io_err)?;
        self.emit_state(true);
        Ok(())
    }

    async fn prefetch(self: Arc<Self>, episode: i64) {
        let Some((id, dub)) =
            self.with_current(|c| (c.playback.shikimori_id, c.playback.dub_key.clone()))
        else {
            return;
        };
        match self.resolver.resolve(id, episode, &dub).await {
            Ok(r) => {
                self.with_current(|c| {
                    if c.playback.shikimori_id == id {
                        c.prefetched = Some((episode, r));
                    }
                });
            }
            Err(e) => tracing::info!("prefetch of episode {episode} failed: {e}"),
        }
    }

    async fn apply(self: &Arc<Self>, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Save => {
                    if let Some(p) = self.with_current(|c| c.playback.clone()) {
                        self.save(&p);
                    }
                }
                Action::Prefetch(n) => {
                    tokio::spawn(self.clone().prefetch(n));
                }
                Action::PlayNext(n) => {
                    let me = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = me.play_episode(n).await {
                            me.report(e);
                        }
                    });
                }
                Action::Skip { part, from, to } => {
                    if let Some(ipc) = self.ipc().await {
                        if let Err(e) = ipc.command(json!(["seek", to, "absolute"])).await {
                            tracing::warn!("cannot skip the {part}: {e}");
                            continue;
                        }
                        self.app.emit(PlayerEvent::Skipped { part, from });
                    }
                }
                Action::Finished => {
                    if let Some((id, episode)) =
                        self.with_current(|c| (c.playback.shikimori_id, c.playback.episode))
                    {
                        self.app.emit(PlayerEvent::Finished {
                            shikimori_id: id,
                            episode,
                        });
                    }
                }
            }
        }
    }

    /// An array rather than a joined string: mpv splits path lists on `;` only on Windows.
    async fn set_shaders(&self, ipc: &Ipc, shaders: Option<Vec<String>>) {
        let paths = shaders
            .as_deref()
            .map(|list| in_config_dir(&self.app.mpv_config_dir(), list))
            .unwrap_or_default();
        let cmd = json!(["set_property", "glsl-shaders", paths]);
        if let Err(e) = ipc.command(cmd).await {
            tracing::warn!("cannot set shaders: {e}");
            return;
        }
        self.with_current(|c| {
            c.shaders_on = shaders.is_some();
            c.shaders = paths;
        });
        self.emit_state(true);
    }

    /// The current episode's upscaler as configured; `force` runs it even on a source that
    /// already fills the screen, as a choice made by hand does.
    async fn apply_upscale(&self, ipc: &Ipc, force: bool) {
        let Some((u, height)) = self.with_current(|c| (c.upscale, c.playback.video_height)) else {
            return;
        };
        let screen = if force {
            self.screen_height().max(height.max(1) * 2)
        } else {
            self.screen_height()
        };
        self.set_shaders(ipc, upscale::shaders(height.max(1), screen, &u))
            .await;
    }

    async fn on_height(&self, ipc: &Ipc, height: u32) {
        let first = self.with_current(|c| {
            c.playback.video_height = height;
            let first = !c.auto_checked;
            c.auto_checked = true;
            first
        });
        if first != Some(true) {
            return;
        }
        // Also clears what the previous episode left, so the state says what mpv does.
        self.apply_upscale(ipc, false).await;
    }

    async fn toggle_upscale(&self, ipc: &Ipc) {
        let Some(on) = self.with_current(|c| c.shaders_on) else {
            return;
        };
        if on {
            self.set_shaders(ipc, None).await;
            return;
        }
        self.with_current(|c| {
            if c.upscale.tier == Tier::Off {
                c.upscale.tier = Tier::Fast;
            }
        });
        // A manual toggle upscales even when the source already matches the screen.
        self.apply_upscale(ipc, true).await;
    }

    /// Openings and endings the source does not know, from AniSkip, once the length is known:
    /// a mark fits only a video of the same length.
    fn fill_skips(self: &Arc<Self>) {
        let Some((id, episode, duration, missing)) = self.with_current(|c| {
            let s = c.skip.clone().unwrap_or_default();
            let p = &c.playback;
            (
                p.shikimori_id,
                p.episode,
                p.duration,
                s.opening.is_none() || s.ending.is_none(),
            )
        }) else {
            return;
        };
        if !missing {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let found = aniskip::pick(&me.aniskip.marks(id, episode).await, duration);
            let prefs = me.prefs();
            let changed = me
                .with_current(|c| {
                    if (c.playback.shikimori_id, c.playback.episode) != (id, episode) {
                        return false;
                    }
                    let s = c.skip.get_or_insert_with(Default::default);
                    let before = s.clone();
                    s.opening = s.opening.or(found.opening);
                    s.ending = s.ending.or(found.ending);
                    if *s == before {
                        return false;
                    }
                    let s = s.clone();
                    c.playback.ending_start =
                        c.playback.ending_start.or(s.ending.map(|(from, _)| from));
                    c.playback.set_auto_skip(
                        auto_range(prefs.skip_opening, s.opening),
                        auto_range(prefs.skip_ending, s.ending),
                    );
                    true
                })
                .unwrap_or(false);
            if changed {
                me.emit_state(true);
            }
        });
    }

    fn on_property(&self, name: &str, data: &serde_json::Value) -> bool {
        self.with_current(|c| {
            let p = &mut c.playback;
            match name {
                "duration" => p.duration = data.as_f64().unwrap_or(p.duration),
                "pause" => p.paused = data.as_bool().unwrap_or(p.paused),
                "volume" => p.volume = data.as_f64().unwrap_or(p.volume),
                "mute" => p.muted = data.as_bool().unwrap_or(p.muted),
                "paused-for-cache" => p.buffering = data.as_bool().unwrap_or(false),
                "track-list" => c.audio_tracks = audio::audio_tracks(data),
                "aid" => c.audio_track = data.as_i64(),
                "demuxer-cache-time" => {
                    p.cached_until = data.as_f64().unwrap_or(p.cached_until);
                    return false;
                }
                _ => return false,
            }
            p.mark_dirty();
            true
        })
        .unwrap_or(false)
    }

    /// Boxed with an explicit type: the loop can start playback, which may spawn a new loop.
    fn event_loop(
        self: Arc<Self>,
        ipc: Arc<Ipc>,
        mut events: UnboundedReceiver<MpvEvent>,
        closed: Arc<AtomicBool>,
        session: u64,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            while let Some(ev) = events.recv().await {
                let mine = self.with_current(|c| c.session) == Some(session);
                if !mine && !matches!(ev, MpvEvent::Shutdown) {
                    continue;
                }
                match ev {
                    MpvEvent::Property { name, data } => match name.as_str() {
                        "time-pos" => {
                            if let Some(pos) = data.as_f64() {
                                let actions = self
                                    .with_current(|c| {
                                        c.playback.buffering = false;
                                        c.playback.on_time(pos)
                                    })
                                    .unwrap_or_default();
                                self.apply(actions).await;
                                self.emit_state(false);
                            }
                        }
                        "height" => {
                            if let Some(h) = data.as_u64() {
                                self.on_height(&ipc, h as u32).await;
                            }
                        }
                        other => {
                            let changed = self.on_property(other, &data);
                            if other == "duration" && data.as_f64().is_some_and(|d| d > 0.0) {
                                self.fill_skips();
                            }
                            match (other, data.as_f64(), data.as_bool()) {
                                ("volume", Some(v), _) => {
                                    self.update_prefs(|p| p.volume = v.round())
                                }
                                ("mute", _, Some(m)) => self.update_prefs(|p| p.muted = m),
                                _ => {}
                            }
                            if other == "pause" && data.as_bool() == Some(true) {
                                self.apply(vec![Action::Save]).await;
                            }
                            if changed {
                                self.emit_state(false);
                            }
                        }
                    },
                    // "stop" also fires for the previous file when a new one replaces it: ignore it.
                    MpvEvent::EndFile { reason } => match reason.as_str() {
                        "eof" | "quit" => {
                            let actions = self
                                .with_current(|c| c.playback.on_end(&reason))
                                .unwrap_or_default();
                            self.apply(actions).await;
                        }
                        "error" => {
                            self.report(AppError::new("Player", "mpv не смог воспроизвести поток"))
                        }
                        _ => {}
                    },
                    MpvEvent::FileLoaded => self.emit_state(true),
                    MpvEvent::Shutdown => break,
                    _ => {}
                }
            }
            // Set beforehand only when the app replaced this mpv on purpose (another window).
            let replaced = closed.swap(true, Ordering::SeqCst);
            let last = {
                let mut cur = self.current.lock().unwrap();
                if cur.as_ref().is_some_and(|c| c.session == session) {
                    cur.take()
                } else {
                    None
                }
            };
            if let Some(cur) = last {
                self.save(&cur.playback);
                self.proxy.close_session(&cur.token);
                self.app.emit(PlayerEvent::Stopped);
                if !replaced {
                    self.report(AppError::new(
                        "Player",
                        "Плеер неожиданно закрылся. Открой серию ещё раз, прогресс сохранён.",
                    ));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpv_arguments_become_options() {
        assert_eq!(option_pair("--osd-level=0"), Some(("osd-level", "0")));
        assert_eq!(
            option_pair("--input-ipc-server=/tmp/a=b.sock"),
            Some(("input-ipc-server", "/tmp/a=b.sock"))
        );
        assert_eq!(option_pair("--idle"), Some(("idle", "yes")));
        assert_eq!(option_pair("idle"), None);
    }

    #[test]
    fn shaders_go_by_full_path() {
        let dir = std::path::Path::new("/opt/ninari/mpv-config");
        let list = vec![
            "~~/shaders/a.glsl".to_string(),
            "/elsewhere/b.glsl".to_string(),
        ];
        let full = in_config_dir(dir, &list);
        assert_eq!(full[0], dir.join("shaders/a.glsl").to_string_lossy());
        assert_eq!(full[1], "/elsewhere/b.glsl");
    }

    #[test]
    fn the_config_file_applies_except_the_windows_output() {
        let conf = "# note\nvo=gpu-next\ngpu-api=d3d11\nhwdec=auto-safe\n\ncache = yes\n";
        assert_eq!(
            config_options(conf),
            vec![
                ("hwdec".into(), "auto-safe".into()),
                ("cache".into(), "yes".into())
            ]
        );
    }

    #[test]
    fn saved_preferences_become_mpv_arguments() {
        let prefs = PlayerPrefs {
            volume: 42.0,
            muted: true,
            fill: true,
            quality_cap: Some(720),
            audio: AudioFx {
                voice: true,
                ..AudioFx::default()
            },
            speed: 1.5,
            skip_fillers: false,
            skip_opening: true,
            skip_ending: false,
        };
        assert_eq!(
            prefs.mpv_args(),
            vec![
                "--volume=42",
                "--mute=yes",
                "--panscan=1",
                "--speed=1.5",
                "--af=@fx:lavfi=[dialoguenhance,alimiter=limit=0.95:level=disabled]"
            ]
        );
        assert_eq!(
            PlayerPrefs::default().mpv_args(),
            vec!["--volume=100", "--mute=no", "--panscan=0", "--speed=1"]
        );
    }

    #[test]
    fn embedded_mpv_draws_nothing_of_its_own() {
        let args = embedded_args(Target::Window(42), "pipe", "cfg");
        for expected in [
            "--wid=42",
            "--osc=no",
            "--load-scripts=no",
            "--osd-level=0",
            "--gpu-api=d3d11",
            "--input-cursor-passthrough=yes",
        ] {
            assert!(args.iter().any(|a| a == expected), "{expected} missing");
        }
    }

    #[test]
    fn libmpv_under_the_controls_is_quiet_and_waits_for_its_surface() {
        let args = embedded_args(Target::Rendered, "pipe", "cfg");
        assert!(args.iter().any(|a| a == "--osc=no") && args.iter().any(|a| a == "--osd-level=0"));
        assert!(!args.iter().any(|a| a.starts_with("--wid")
            || a == "--force-window=yes"
            || a.starts_with("--gpu-api")));
    }

    #[test]
    fn mpv_in_its_own_window_keeps_its_controls() {
        let args = embedded_args(Target::Own, "pipe", "cfg");
        assert!(args.iter().any(|a| a == "--input-ipc-server=pipe"));
        assert!(!args.iter().any(|a| a.starts_with("--wid")
            || a == "--osc=no"
            || a == "--input-default-bindings=no"));
    }

    fn prefs_from(pairs: &[(&str, serde_json::Value)]) -> PlayerPrefs {
        let map: std::collections::HashMap<&str, serde_json::Value> =
            pairs.iter().cloned().collect();
        PlayerPrefs::from_settings(|k| map.get(k).cloned())
    }

    #[test]
    fn quality_cap_is_a_number_or_auto() {
        let cap = |v| prefs_from(&[(QUALITY_SETTING, v)]).quality_cap;
        assert_eq!(cap(json!(720)), Some(720));
        assert_eq!(cap(json!("auto")), None);
        assert_eq!(cap(json!(-5)), None);
        assert_eq!(prefs_from(&[]).quality_cap, None);
    }

    #[test]
    fn preferences_parse_from_settings_values() {
        let p = prefs_from(&[
            (VOLUME_SETTING, json!(35.5)),
            (MUTED_SETTING, json!(false)),
            (FIT_SETTING, json!("fill")),
            (QUALITY_SETTING, json!(480)),
            (AUDIO_SETTING, json!({ "night": true })),
            (SPEED_SETTING, json!(1.25)),
            (SKIP_OPENING_SETTING, json!(true)),
            (SKIP_ENDING_SETTING, json!(false)),
            (SKIP_FILLERS_SETTING, json!(true)),
        ]);
        assert_eq!(
            p,
            PlayerPrefs {
                volume: 35.5,
                muted: false,
                fill: true,
                quality_cap: Some(480),
                audio: AudioFx {
                    night: true,
                    ..AudioFx::default()
                },
                speed: 1.25,
                skip_opening: true,
                skip_ending: false,
                skip_fillers: true,
            }
        );
        let broken = prefs_from(&[
            (VOLUME_SETTING, json!("loud")),
            (FIT_SETTING, json!(3)),
            (AUDIO_SETTING, json!([1])),
            (SPEED_SETTING, json!(9)),
            (SKIP_OPENING_SETTING, json!("yes")),
        ]);
        assert_eq!(broken, PlayerPrefs::default());
    }

    #[test]
    fn speed_is_one_of_the_offered_steps() {
        assert_eq!(clamp_speed(1.3), 1.25);
        assert_eq!(clamp_speed(0.1), 0.5);
        assert_eq!(clamp_speed(3.0), 2.0);
        assert_eq!(clamp_speed(f64::NAN), 1.0);
    }
}
