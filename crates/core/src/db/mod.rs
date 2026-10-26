use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

pub type Db = Arc<Mutex<Connection>>;

const SCHEMA_V1: &str = r#"
CREATE TABLE titles (
    shikimori_id INTEGER PRIMARY KEY,
    data_json TEXT NOT NULL,
    palette_json TEXT,
    poster_path TEXT,
    fetched_at INTEGER NOT NULL
);
CREATE TABLE library (
    shikimori_id INTEGER PRIMARY KEY,
    status TEXT NOT NULL,
    chosen_dub_key TEXT,
    notify INTEGER NOT NULL DEFAULT 1,
    added_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE episode_progress (
    shikimori_id INTEGER NOT NULL,
    episode INTEGER NOT NULL,
    position_sec REAL NOT NULL,
    duration_sec REAL NOT NULL,
    watched INTEGER NOT NULL DEFAULT 0,
    dub_key TEXT,
    extension_id TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (shikimori_id, episode)
);
CREATE TABLE mappings (
    shikimori_id INTEGER NOT NULL,
    extension_id TEXT NOT NULL,
    match_json TEXT NOT NULL,
    candidates_json TEXT NOT NULL DEFAULT '[]',
    confidence TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    episode_offset INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (shikimori_id, extension_id)
);
CREATE TABLE episode_index (
    shikimori_id INTEGER NOT NULL,
    extension_id TEXT NOT NULL,
    episode INTEGER NOT NULL,
    dub_key TEXT NOT NULL,
    max_quality INTEGER,
    first_seen_at INTEGER NOT NULL,
    PRIMARY KEY (shikimori_id, extension_id, episode, dub_key)
);
CREATE TABLE extension_health (
    extension_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    ok INTEGER NOT NULL,
    error_kind TEXT
);
CREATE INDEX extension_health_by_ext ON extension_health (extension_id, ts);
CREATE TABLE extension_repos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    urls_json TEXT NOT NULL,
    added_at INTEGER NOT NULL
);
CREATE TABLE extensions (
    id TEXT PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES extension_repos(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0,
    installed_at INTEGER NOT NULL
);
CREATE TABLE extension_kv (
    extension_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    expires_at INTEGER,
    PRIMARY KEY (extension_id, key)
);
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);
"#;

/// Cover art from AniList. Posters downloaded so far came from Shikimori; clearing their paths
/// fetches them again, now from AniList, which has larger ones.
const SCHEMA_V2: &str = r#"
CREATE TABLE anilist_art (
    mal_id INTEGER PRIMARY KEY,
    cover_url TEXT,
    banner_url TEXT,
    color TEXT,
    fetched_at INTEGER NOT NULL
);
ALTER TABLE titles ADD COLUMN banner_path TEXT;
UPDATE titles SET poster_path = NULL;
"#;

/// Devices paired for syncing on the local network. `sent_until` is this device's clock, and
/// `received_until` the peer's: each side asks the other for changes by the other's own clock.
const SCHEMA_V3: &str = r#"
CREATE TABLE peers (
    device_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    key BLOB NOT NULL,
    sent_until INTEGER NOT NULL DEFAULT 0,
    received_until INTEGER NOT NULL DEFAULT 0,
    last_sync INTEGER,
    paired_at INTEGER NOT NULL
);
"#;

/// Filler and recap marks of episodes from Jikan, per MyAnimeList (= Shikimori) id.
const SCHEMA_V4: &str = r#"
CREATE TABLE jikan_episodes (
    mal_id INTEGER PRIMARY KEY,
    marks_json TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);
"#;

/// The user's own score (1-10) and note for a title.
const SCHEMA_V5: &str = r#"
ALTER TABLE library ADD COLUMN score INTEGER;
ALTER TABLE library ADD COLUMN note TEXT;
"#;

/// Dub names as the sources spell them, by key: the key alone has lost its case and spaces.
const SCHEMA_V6: &str = r#"
CREATE TABLE dub_names (
    key TEXT PRIMARY KEY,
    name TEXT NOT NULL
);
"#;

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(SCHEMA_V1),
        M::up(SCHEMA_V2),
        M::up(SCHEMA_V3),
        M::up(SCHEMA_V4),
        M::up(SCHEMA_V5),
        M::up(SCHEMA_V6),
    ])
}

fn prepare(mut conn: Connection) -> anyhow::Result<Connection> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

pub fn open(path: &Path) -> anyhow::Result<Connection> {
    // On the first start nothing of the app's folder exists yet.
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    prepare(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> anyhow::Result<Connection> {
    prepare(Connection::open_in_memory()?)
}

pub fn get_setting(conn: &Connection, key: &str) -> anyhow::Result<Option<serde_json::Value>> {
    use rusqlite::OptionalExtension;
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        rusqlite::params![key, value.to_string()],
    )?;
    Ok(())
}

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get_setting(&conn, "player.anime4k").unwrap(), None);
        set_setting(&conn, "player.anime4k", &serde_json::json!("hq")).unwrap();
        set_setting(&conn, "player.anime4k", &serde_json::json!("off")).unwrap();
        assert_eq!(
            get_setting(&conn, "player.anime4k").unwrap(),
            Some(serde_json::json!("off"))
        );
    }

    #[test]
    fn migrations_are_valid() {
        assert!(migrations().validate().is_ok());
    }

    #[test]
    fn open_creates_schema() {
        let conn = open_in_memory().unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for t in [
            "dub_names",
            "episode_index",
            "episode_progress",
            "extension_health",
            "extension_kv",
            "extension_repos",
            "extensions",
            "library",
            "mappings",
            "settings",
            "titles",
        ] {
            assert!(tables.iter().any(|x| x == t), "missing table {t}");
        }
    }
}
