//! New versions from GitHub Releases: checked a little after start and on demand. The release
//! page opens in the browser.
//! No download and install yet; that comes with packaging the GPUI app.

use std::sync::Arc;

use ninari_core::error::AppError;
use serde_json::Value;

use crate::core::Core;

const LATEST: &str = "https://github.com/onokashino/ninari/releases/latest/download/latest.json";
pub const RELEASES: &str = "https://github.com/onokashino/ninari/releases/latest";
/// Whether to look for a new version on start; on unless the user turns it off.
pub const AUTO_UPDATE_SETTING: &str = "app.auto_update";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
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
            notes: latest["notes"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
            version: version.trim_start_matches('v').to_string(),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
