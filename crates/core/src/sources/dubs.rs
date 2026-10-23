//! Dub (voice-over team) name normalization: "AniDUB", "AniDub Online" and "anidub.com"
//! all become the key `anidub` with the display name from `dubs.json`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::extensions::types::Dub;

const DOMAIN_SUFFIXES: [&str; 11] = [
    ".com", ".tv", ".ru", ".org", ".net", ".me", ".top", ".online", ".club", ".su", ".fun",
];

pub fn normalize_key(name: &str) -> String {
    let mut s = name.trim().to_lowercase();
    for suffix in DOMAIN_SUFFIXES {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.to_string();
            break;
        }
    }
    let key: String = s.chars().filter(|c| c.is_alphanumeric()).collect();
    if key.is_empty() {
        "unknown".into()
    } else {
        key
    }
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct DubTable {
    /// normalized alias -> (key, display name)
    by_alias: HashMap<String, (String, String)>,
}

impl DubTable {
    pub fn from_json(json: &str) -> Self {
        let entries: HashMap<String, Entry> = match serde_json::from_str(json) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("dubs.json is invalid: {e}");
                return Self::default();
            }
        };
        let mut by_alias = HashMap::new();
        for (key, entry) in entries {
            let value = (key.clone(), entry.name.clone());
            by_alias.insert(normalize_key(&key), value.clone());
            by_alias.insert(normalize_key(&entry.name), value.clone());
            for alias in &entry.aliases {
                by_alias.insert(normalize_key(alias), value.clone());
            }
        }
        Self { by_alias }
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|s| Self::from_json(&s))
            .unwrap_or_default()
    }

    /// Returns `(key, display name)`. Unknown dubs keep their own name.
    pub fn canonical(&self, raw: &str) -> (String, String) {
        let key = normalize_key(raw);
        match self.by_alias.get(&key) {
            Some(v) => v.clone(),
            None => (key, raw.trim().to_string()),
        }
    }

    /// Like `canonical`, but subtitles by a team never merge with its voice-over: they get
    /// the key `<team>-sub` (a hyphen never appears in name-derived keys).
    pub fn canonical_dub(&self, dub: &Dub) -> (String, String) {
        let (key, display) = self.canonical(&dub.name);
        if dub.kind != "subtitles" {
            return (key, display);
        }
        (format!("{key}{SUB_SUFFIX}"), subtitled(display))
    }

    /// Display name for a key made by `canonical_dub`.
    pub fn display_name(&self, key: &str) -> String {
        match key.strip_suffix(SUB_SUFFIX) {
            Some(team) => subtitled(self.canonical(team).1),
            None => self.canonical(key).1,
        }
    }
}

const SUB_SUFFIX: &str = "-sub";

fn subtitled(display: String) -> String {
    if display.to_lowercase().contains("субтитр") {
        display
    } else {
        format!("{display} (субтитры)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_dub_names() {
        assert_eq!(normalize_key("AniDUB"), "anidub");
        assert_eq!(normalize_key("Anidub.com"), "anidub");
        assert_eq!(normalize_key("  Студийная Банда "), "студийнаябанда");
        assert_eq!(normalize_key("SHIZA Project"), "shizaproject");
        assert_eq!(normalize_key("  "), "unknown");
    }

    #[test]
    fn subtitles_get_their_own_key() {
        use crate::extensions::types::Dub;
        let t = DubTable::from_json(r#"{"anidub":{"name":"AniDUB","aliases":["anidub"]}}"#);
        let dub = |kind: &str| Dub {
            id: "1".into(),
            name: "AniDub".into(),
            kind: kind.into(),
        };
        assert_eq!(
            t.canonical_dub(&dub("voice")),
            ("anidub".into(), "AniDUB".into())
        );
        assert_eq!(
            t.canonical_dub(&dub("subtitles")),
            ("anidub-sub".into(), "AniDUB (субтитры)".into())
        );
        assert_eq!(t.display_name("anidub-sub"), "AniDUB (субтитры)");
        assert_eq!(t.display_name("anidub"), "AniDUB");
    }

    #[test]
    fn canonical_uses_aliases() {
        let t = DubTable::from_json(
            r#"{"anidub":{"name":"AniDUB","aliases":["anidub","anidubonline"]}}"#,
        );
        assert_eq!(
            t.canonical("AniDub Online"),
            ("anidub".to_string(), "AniDUB".to_string())
        );
        assert_eq!(
            t.canonical("anidub.com"),
            ("anidub".to_string(), "AniDUB".to_string())
        );
        assert_eq!(
            t.canonical("Неизвестные Ребята"),
            (
                "неизвестныеребята".to_string(),
                "Неизвестные Ребята".to_string()
            )
        );
    }

    #[test]
    fn broken_table_is_empty_not_fatal() {
        let t = DubTable::from_json("{not json");
        assert_eq!(
            t.canonical("AniDUB"),
            ("anidub".to_string(), "AniDUB".to_string())
        );
    }
}
