//! New versions from GitHub Releases: checked a little after start and on demand, then
//! downloaded, checked against the checksum latest.json gives, installed over this copy and
//! started again. A build from the repository only points at the release page.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Duration;

use gpui::{App, Global};
use ninari_core::error::AppError;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::core::{Core, cache_dir};

const LATEST: &str = "https://github.com/onokashino/ninari/releases/latest/download/latest.json";
pub const RELEASES: &str = "https://github.com/onokashino/ninari/releases/latest";
/// Whether to look for a new version on start; on unless the user turns it off.
pub const AUTO_UPDATE_SETTING: &str = "app.auto_update";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    /// The package for this system, when the release has one.
    pub package: Option<Package>,
}

/// A file of the release: the Linux archive or the Windows installer.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub url: String,
    pub sha256: String,
}

/// This build's key among latest.json's platforms: "linux-x86_64", "windows-x86_64".
fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn package(latest: &Value) -> Option<Package> {
    let entry = &latest["platforms"][platform()];
    Some(Package {
        url: entry["url"].as_str()?.to_string(),
        sha256: entry["sha256"].as_str()?.to_lowercase(),
    })
}

fn parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map_while(|p| p.parse().ok())
        .collect()
}

/// Whether `latest` is a later version than `current`, by its numbers.
pub fn newer(latest: &str, current: &str) -> bool {
    parts(latest) > parts(current)
}

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::new("Update", e.to_string())
}

/// The release newer than this build, or None when this one is the latest.
pub fn check(core: &Arc<Core>) -> impl Future<Output = anyhow::Result<Option<UpdateInfo>>> + use<> {
    core.run(|c| async move {
        let reply = c.http.inner().get(LATEST).send().await.map_err(fail)?;
        // Nothing released yet: nothing newer either.
        if reply.status() == 404 {
            return Ok(None);
        }
        let latest: Value = reply
            .error_for_status()
            .map_err(fail)?
            .json()
            .await
            .map_err(fail)?;
        let version = latest["version"].as_str().unwrap_or_default().to_string();
        Ok(newer(&version, CURRENT).then(|| UpdateInfo {
            package: package(&latest),
            notes: latest["notes"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
            version: version.trim_start_matches('v').to_string(),
        }))
    })
}

/// What installing an update is doing, for every window to show.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Progress {
    #[default]
    Idle,
    /// The share downloaded, 0 to 1; 0 while the size is unknown.
    Downloading(f32),
    Installing,
    Failed(String),
}

#[derive(Default)]
struct State(Progress);

impl Global for State {}

pub fn progress(cx: &App) -> Progress {
    cx.try_global::<State>()
        .map(|s| s.0.clone())
        .unwrap_or_default()
}

fn set(progress: Progress, cx: &mut App) {
    cx.default_global::<State>().0 = progress;
    cx.refresh_windows();
}

/// Whether this copy can replace itself: a release (mpv-config lies beside the program) with a
/// package for this system. A build from the repository is updated with git instead.
pub fn can_install(info: &UpdateInfo) -> bool {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("mpv-config")));
    info.package.is_some() && beside.is_some_and(|dir| dir.is_dir())
}

/// Where downloads wait to be installed; cleared on the next start.
pub fn folder() -> PathBuf {
    cache_dir().join("update")
}

/// Downloads, checks and installs `info`'s package, then starts the new version and quits.
/// Anything wrong leaves this version as it is and says why.
pub fn install(info: &UpdateInfo, cx: &mut App) {
    let Some(package) = info.package.clone().filter(|_| can_install(info)) else {
        cx.open_url(RELEASES);
        return;
    };
    if matches!(
        progress(cx),
        Progress::Downloading(_) | Progress::Installing
    ) {
        return;
    }
    set(Progress::Downloading(0.), cx);
    let got = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let downloaded = Arc::new(AtomicBool::new(false));
    let job = {
        let (got, total, downloaded) = (got.clone(), total.clone(), downloaded.clone());
        Core::get(cx).run(move |c| async move {
            let file = download(&c, &package, &got, &total).await?;
            downloaded.store(true, Relaxed);
            apply(&file).await
        })
    };
    // The share downloaded, a few times a second, while it lasts.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let shown = match (downloaded.load(Relaxed), total.load(Relaxed)) {
                (true, _) => Progress::Installing,
                (false, 0) => Progress::Downloading(0.),
                (false, all) => Progress::Downloading(got.load(Relaxed) as f32 / all as f32),
            };
            let busy = cx.update(|cx| {
                let busy = matches!(progress(cx), Progress::Downloading(_));
                if busy {
                    set(shown, cx);
                }
                busy
            });
            if !busy {
                break;
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| match job.await {
        Ok(program) => cx.update(|cx| {
            set(Progress::Installing, cx);
            restart(program.as_deref());
            cx.quit();
        }),
        Err(e) => {
            tracing::warn!("update: {e:#}");
            cx.update(|cx| set(Progress::Failed(format!("{e:#}")), cx));
        }
    })
    .detach();
}

async fn download(
    c: &Core,
    package: &Package,
    got: &AtomicU64,
    total: &AtomicU64,
) -> Result<PathBuf, AppError> {
    let dir = folder();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let name = package.url.rsplit('/').next().unwrap_or("ninari-update");
    let path = dir.join(name);
    let mut reply = c
        .http
        .inner()
        .get(&package.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(fail)?;
    total.store(reply.content_length().unwrap_or(0), Relaxed);
    let mut file = std::fs::File::create(&path)?;
    let mut hash = Sha256::new();
    while let Some(chunk) = reply.chunk().await.map_err(fail)? {
        hash.update(&chunk);
        file.write_all(&chunk)?;
        got.fetch_add(chunk.len() as u64, Relaxed);
    }
    file.flush()?;
    let sum: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if sum != package.sha256 {
        return Err(fail(
            "скачанный файл повреждён, контрольная сумма не совпала",
        ));
    }
    Ok(path)
}

/// Linux: the archive unpacked and its install.sh run, which swaps the program folder. Returns
/// the program to start.
#[cfg(not(windows))]
async fn apply(file: &Path) -> Result<Option<PathBuf>, AppError> {
    let dir = file.with_extension("unpacked");
    std::fs::create_dir_all(&dir)?;
    run(tokio::process::Command::new("tar")
        .arg("xzf")
        .arg(file)
        .arg("-C")
        .arg(&dir))
    .await?;
    let unpacked = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("install.sh"))
        .find(|p| p.is_file())
        .ok_or_else(|| fail("в архиве нет install.sh"))?;
    run(tokio::process::Command::new("bash").arg(unpacked)).await?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| fail("нет HOME"))?;
    Ok(Some(data.join("ninari/ninari")))
}

/// Windows: the installer, started quietly; it waits for this copy to close, replaces it and
/// starts the new one itself.
#[cfg(windows)]
async fn apply(file: &Path) -> Result<Option<PathBuf>, AppError> {
    std::process::Command::new(file)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
        ])
        .spawn()?;
    Ok(None)
}

#[cfg(not(windows))]
async fn run(command: &mut tokio::process::Command) -> Result<(), AppError> {
    let out = command.output().await?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    Err(fail(format!("{:?}: {}", out.status, said.trim())))
}

/// Starts `program` once this process is gone, so it does not find this one still running and
/// hand itself over to it.
fn restart(program: Option<&Path>) {
    let Some(program) = program else {
        return;
    };
    #[cfg(unix)]
    {
        let wait = "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec \"$2\"";
        let started = std::process::Command::new("sh")
            .args(["-c", wait, "sh", &std::process::id().to_string()])
            .arg(program)
            .spawn();
        if let Err(e) = started {
            tracing::warn!("cannot start the new version: {e}");
        }
    }
    #[cfg(not(unix))]
    let _ = program;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_is_this_systems() {
        let latest: Value = serde_json::json!({
            "version": "0.2.0",
            "platforms": {
                platform(): {"url": "https://example.org/a", "sha256": "ABC"},
                "plan9-mips": {"url": "https://example.org/b", "sha256": "def"}
            }
        });
        assert_eq!(
            package(&latest),
            Some(Package {
                url: "https://example.org/a".into(),
                sha256: "abc".into()
            })
        );
        assert_eq!(package(&serde_json::json!({"version": "0.2.0"})), None);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v0.10.0", "0.9.3"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("", "0.1.0"));
    }
}
