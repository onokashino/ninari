//! The built-in extensions registry: what it offers, merged with what is installed.

use serde::{Deserialize, Serialize};

/// Official registry. Change it when the extensions repository is published elsewhere;
/// the `extensions.registry` setting overrides it (useful for development).
pub const DEFAULT_REGISTRY: &str =
    "https://raw.githubusercontent.com/onokashino/ninari/master/extensions/dist/index.json";
pub const REGISTRY_SETTING: &str = "extensions.registry";

pub fn registry_url(setting: Option<String>) -> String {
    setting
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY.to_string())
}

/// One extension as listed in a repository `index.json`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct IndexEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub domains: Vec<String>,
    /// Hosts besides `domains` the extension may fetch (players, CDNs).
    #[serde(default, rename = "allowedHosts")]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub domains: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub installed_version: Option<String>,
    pub enabled: bool,
    pub update_available: bool,
}

/// Registry entries in index order, each marked with its install state.
pub fn merge(index: &[IndexEntry], installed: &[Installed]) -> Vec<RegistryEntry> {
    index
        .iter()
        .map(|e| {
            let inst = installed.iter().find(|i| i.id == e.id);
            RegistryEntry {
                id: e.id.clone(),
                name: if e.name.is_empty() {
                    e.id.clone()
                } else {
                    e.name.clone()
                },
                version: e.version.clone(),
                domains: e.domains.clone(),
                allowed_hosts: e.allowed_hosts.clone(),
                installed_version: inst.map(|i| i.version.clone()),
                enabled: inst.is_some_and(|i| i.enabled),
                update_available: inst
                    .is_some_and(|i| !e.version.is_empty() && i.version != e.version),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, version: &str) -> IndexEntry {
        IndexEntry {
            id: id.into(),
            name: id.to_uppercase(),
            version: version.into(),
            domains: vec![format!("https://{id}.example")],
            allowed_hosts: vec![],
        }
    }

    #[test]
    fn merges_index_with_installed_extensions() {
        let index = vec![
            entry("anilibria", "0.2.0"),
            entry("yummy", "0.1.0"),
            entry("kodik", "1.0.0"),
        ];
        let installed = vec![
            Installed {
                id: "anilibria".into(),
                version: "0.1.0".into(),
                enabled: true,
            },
            Installed {
                id: "kodik".into(),
                version: "1.0.0".into(),
                enabled: false,
            },
        ];
        let merged = merge(&index, &installed);
        let row = |id: &str| merged.iter().find(|e| e.id == id).unwrap();
        assert_eq!(row("anilibria").installed_version.as_deref(), Some("0.1.0"));
        assert!(row("anilibria").update_available);
        assert!(row("anilibria").enabled);
        assert_eq!(row("yummy").installed_version, None);
        assert!(!row("yummy").update_available);
        assert!(!row("kodik").enabled);
        assert!(!row("kodik").update_available);
        assert_eq!(
            merged.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["anilibria", "yummy", "kodik"]
        );
    }

    #[test]
    fn the_hosts_an_extension_talks_to_reach_the_registry_screen() {
        let index: Vec<IndexEntry> = serde_json::from_str(
            r#"[{ "id": "yummy", "domains": ["https://api.yani.tv"],
                  "allowedHosts": ["*.kodikplayer.com", "video.sibnet.ru"] }]"#,
        )
        .unwrap();
        let merged = merge(&index, &[]);
        assert_eq!(
            merged[0].allowed_hosts,
            ["*.kodikplayer.com", "video.sibnet.ru"]
        );
        let json = serde_json::to_value(&merged[0]).unwrap();
        assert_eq!(json["allowedHosts"][1], "video.sibnet.ru");
    }

    #[test]
    fn registry_url_prefers_the_setting() {
        assert_eq!(registry_url(None), DEFAULT_REGISTRY);
        assert_eq!(registry_url(Some(" ".into())), DEFAULT_REGISTRY);
        assert_eq!(
            registry_url(Some("D:/ext/index.json".into())),
            "D:/ext/index.json"
        );
    }
}
