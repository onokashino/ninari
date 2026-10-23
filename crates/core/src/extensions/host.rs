//! Installed extensions: installation from a repository `index.json` (URL or local path),
//! sha256 verification and loading into QuickJS runtimes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::registry::{IndexEntry, Installed};
use super::runtime::{JsExtension, KvStore};
use super::types::Manifest;
use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::net::HttpClient;

#[derive(Deserialize)]
struct RepoIndex {
    extensions: Vec<RepoEntry>,
    dubs: Option<String>,
}

#[derive(Deserialize)]
struct RepoListing {
    extensions: Vec<IndexEntry>,
}

#[derive(Deserialize)]
struct RepoEntry {
    id: String,
    file: String,
    sha256: String,
}

fn is_url(loc: &str) -> bool {
    loc.starts_with("http://") || loc.starts_with("https://")
}

/// Ids become file names in the extensions folder, so only `[a-z0-9][a-z0-9_-]{0,63}` passes.
fn check_id(id: &str) -> Result<(), AppError> {
    let mut chars = id.chars();
    let ok = id.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(AppError::new(
            "Extension",
            format!("недопустимый id расширения: {id:?}"),
        ))
    }
}

fn resolve_relative(index_loc: &str, file: &str) -> Result<String, AppError> {
    if is_url(index_loc) {
        let base = url::Url::parse(index_loc)
            .map_err(|e| AppError::new("Extension", format!("bad url: {e}")))?;
        let joined = base
            .join(file)
            .map_err(|e| AppError::new("Extension", format!("bad path {file}: {e}")))?;
        if !is_url(joined.as_str()) {
            return Err(AppError::new(
                "Extension",
                format!("{file}: репозиторий из сети может ссылаться только на http(s)"),
            ));
        }
        return Ok(joined.to_string());
    }
    let parent = Path::new(index_loc).parent().unwrap_or(Path::new("."));
    Ok(parent.join(file).to_string_lossy().into_owned())
}

pub struct ExtensionHost {
    http: HttpClient,
    pub(crate) db: Db,
    dir: PathBuf,
    loaded: RwLock<Vec<Arc<JsExtension>>>,
}

impl ExtensionHost {
    /// `dir` holds installed scripts and `dubs.json`.
    pub fn new(http: HttpClient, db: Db, dir: PathBuf) -> Self {
        Self {
            http,
            db,
            dir,
            loaded: RwLock::new(Vec::new()),
        }
    }

    pub fn dubs_path(&self) -> PathBuf {
        self.dir.join("dubs.json")
    }

    pub fn all(&self) -> Vec<Arc<JsExtension>> {
        self.loaded.read().unwrap().clone()
    }

    pub fn list(&self) -> Vec<Manifest> {
        self.all().iter().map(|e| e.manifest().clone()).collect()
    }

    #[cfg(test)]
    pub(crate) fn replace_loaded(&self, extensions: Vec<Arc<JsExtension>>) {
        *self.loaded.write().unwrap() = extensions;
    }

    pub async fn load_installed(&self) -> Result<(), AppError> {
        let ids: Vec<String> = {
            let conn = self.db.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id FROM extensions WHERE enabled = 1 ORDER BY position, installed_at",
            )?;
            let rows = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            rows
        };
        let mut loaded = Vec::new();
        for id in ids {
            if let Err(e) = check_id(&id) {
                tracing::warn!("{}", e.message);
                continue;
            }
            let path = self.dir.join(format!("{id}.js"));
            let source = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("extension {id}: cannot read {}: {e}", path.display());
                    continue;
                }
            };
            match JsExtension::load(
                &source,
                self.http.clone(),
                KvStore::new(self.db.clone(), id.clone()),
            )
            .await
            {
                Ok(ext) => loaded.push(Arc::new(ext)),
                Err(e) => tracing::warn!("extension {id} failed to load: {e}"),
            }
        }
        *self.loaded.write().unwrap() = loaded;
        Ok(())
    }

    async fn read_location(&self, loc: &str) -> Result<Vec<u8>, AppError> {
        if is_url(loc) {
            let resp = self
                .http
                .inner()
                .get(loc)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| AppError::new("Blocked", format!("{loc}: {e}")))?;
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::new("Blocked", format!("{loc}: {e}")))?;
            return Ok(bytes.to_vec());
        }
        let path = loc.strip_prefix("file://").unwrap_or(loc);
        std::fs::read(path).map_err(|e| AppError::new("Extension", format!("{path}: {e}")))
    }

    fn repo_id(&self, loc: &str) -> Result<i64, AppError> {
        let urls = serde_json::to_string(&[loc])?;
        let conn = self.db.lock().unwrap();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM extension_repos WHERE urls_json = ?1",
                [&urls],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        conn.execute(
            "INSERT INTO extension_repos (urls_json, added_at) VALUES (?1, ?2)",
            params![urls, now_ts()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// What a repository offers, without installing anything.
    pub async fn fetch_index(&self, url_or_path: &str) -> Result<Vec<IndexEntry>, AppError> {
        let loc = url_or_path.trim().trim_matches('"');
        let listing: RepoListing = serde_json::from_slice(&self.read_location(loc).await?)
            .map_err(|e| AppError::new("Extension", format!("index.json: {e}")))?;
        Ok(listing.extensions)
    }

    pub fn installed(&self) -> Result<Vec<Installed>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, version, enabled FROM extensions ORDER BY position, installed_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Installed {
                    id: r.get(0)?,
                    version: r.get(1)?,
                    enabled: r.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), AppError> {
        self.db.lock().unwrap().execute(
            "UPDATE extensions SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )?;
        self.load_installed().await
    }

    pub async fn remove(&self, id: &str) -> Result<(), AppError> {
        check_id(id)?;
        {
            let conn = self.db.lock().unwrap();
            conn.execute("DELETE FROM extensions WHERE id = ?1", [id])?;
            conn.execute("DELETE FROM extension_kv WHERE extension_id = ?1", [id])?;
        }
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.js")));
        self.load_installed().await
    }

    /// Installs (or updates) the extensions listed in a repository `index.json`:
    /// all of them, or only the ids in `only`.
    pub async fn install_repo(
        &self,
        url_or_path: &str,
        only: Option<&[String]>,
    ) -> Result<Vec<Manifest>, AppError> {
        let loc = url_or_path.trim().trim_matches('"');
        let index: RepoIndex = serde_json::from_slice(&self.read_location(loc).await?)
            .map_err(|e| AppError::new("Extension", format!("index.json: {e}")))?;
        std::fs::create_dir_all(&self.dir)?;
        let repo_id = self.repo_id(loc)?;
        let mut manifests = Vec::new();
        for (position, entry) in index.extensions.iter().enumerate() {
            if only.is_some_and(|ids| !ids.contains(&entry.id)) {
                continue;
            }
            check_id(&entry.id)?;
            let bytes = self
                .read_location(&resolve_relative(loc, &entry.file)?)
                .await?;
            let sha = hex::encode(Sha256::digest(&bytes));
            if !sha.eq_ignore_ascii_case(&entry.sha256) {
                return Err(AppError::new(
                    "Extension",
                    format!(
                        "{}: sha256 не совпадает (ожидался {}, получен {sha})",
                        entry.id, entry.sha256
                    ),
                ));
            }
            let source = String::from_utf8(bytes).map_err(|_| {
                AppError::new("Extension", format!("{}: скрипт не в UTF-8", entry.id))
            })?;
            let ext = JsExtension::load(
                &source,
                self.http.clone(),
                KvStore::new(self.db.clone(), entry.id.clone()),
            )
            .await?;
            let manifest = ext.manifest().clone();
            if manifest.id != entry.id {
                return Err(AppError::new(
                    "Extension",
                    format!(
                        "id в манифесте ({}) не совпадает с index.json ({})",
                        manifest.id, entry.id
                    ),
                ));
            }
            std::fs::write(self.dir.join(format!("{}.js", entry.id)), &source)?;
            {
                let conn = self.db.lock().unwrap();
                conn.execute(
                    "INSERT INTO extensions (id, repo_id, version, sha256, manifest_json, enabled, position, installed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET repo_id = excluded.repo_id, version = excluded.version,
                       sha256 = excluded.sha256, manifest_json = excluded.manifest_json, installed_at = excluded.installed_at",
                    params![
                        manifest.id,
                        repo_id,
                        manifest.version,
                        sha,
                        serde_json::to_string(&manifest)?,
                        position as i64,
                        now_ts()
                    ],
                )?;
                // A new version may match titles better: its uncertain guesses are searched again.
                conn.execute(
                    "DELETE FROM mappings WHERE extension_id = ?1 AND pinned = 0 AND confidence != 'exact'",
                    [&manifest.id],
                )?;
            }
            manifests.push(manifest);
        }
        if let Some(dubs) = &index.dubs {
            let bytes = self.read_location(&resolve_relative(loc, dubs)?).await?;
            std::fs::write(self.dubs_path(), bytes)?;
        }
        self.load_installed().await?;
        Ok(manifests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};

    const EXT: &str = r#"export const manifest = { id: "demo", name: "Demo", version: "0.1.0", apiVersion: 1,
        domains: ["https://example.org"], allowedHosts: [] };
        export async function search(q, ctx) { return []; }"#;

    fn write_repo(dir: &std::path::Path, sha: &str) -> std::path::PathBuf {
        std::fs::write(dir.join("demo.js"), EXT).unwrap();
        std::fs::write(
            dir.join("dubs.json"),
            r#"{"anidub":{"name":"AniDUB","aliases":["anidub"]}}"#,
        )
        .unwrap();
        let index = format!(
            r#"{{ "name": "test-repo", "dubs": "dubs.json", "extensions": [
                {{ "id": "demo", "name": "Demo", "version": "0.1.0", "apiVersion": 1,
                   "file": "demo.js", "sha256": "{sha}", "domains": ["https://example.org"] }} ] }}"#
        );
        let path = dir.join("index.json");
        std::fs::write(&path, index).unwrap();
        path
    }

    fn host(data: &std::path::Path) -> ExtensionHost {
        ExtensionHost::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            data.to_path_buf(),
        )
    }

    #[tokio::test]
    async fn toggling_and_removing_extensions() {
        let repo = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let sha = hex::encode(Sha256::digest(EXT.as_bytes()));
        let index = write_repo(repo.path(), &sha);
        let h = host(data.path());

        let offered = h.fetch_index(index.to_str().unwrap()).await.unwrap();
        assert_eq!(offered[0].id, "demo");
        assert!(h.installed().unwrap().is_empty());

        h.install_repo(index.to_str().unwrap(), Some(&["other".to_string()]))
            .await
            .unwrap();
        assert!(
            h.installed().unwrap().is_empty(),
            "only the listed ids are installed"
        );

        h.install_repo(index.to_str().unwrap(), None).await.unwrap();
        assert_eq!(h.installed().unwrap()[0].version, "0.1.0");

        h.set_enabled("demo", false).await.unwrap();
        assert!(h.list().is_empty(), "a disabled extension is not loaded");
        assert!(!h.installed().unwrap()[0].enabled);
        h.set_enabled("demo", true).await.unwrap();
        assert_eq!(h.list().len(), 1);

        h.remove("demo").await.unwrap();
        assert!(h.installed().unwrap().is_empty());
        assert!(h.list().is_empty());
        assert!(!data.path().join("demo.js").exists());
    }

    #[tokio::test]
    async fn install_rejects_ids_that_escape_the_folder() {
        for id in ["../evil", "..\\\\evil", "C:/evil", "Evil", ""] {
            let repo = tempfile::tempdir().unwrap();
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("extensions");
            let src = EXT.replace(r#"id: "demo""#, &format!(r#"id: "{id}""#));
            std::fs::write(repo.path().join("demo.js"), &src).unwrap();
            let sha = hex::encode(Sha256::digest(src.as_bytes()));
            let index = repo.path().join("index.json");
            std::fs::write(
                &index,
                format!(
                    r#"{{ "extensions": [ {{ "id": "{id}", "file": "demo.js", "sha256": "{sha}" }} ] }}"#
                ),
            )
            .unwrap();
            let h = host(&data);
            let err = h
                .install_repo(index.to_str().unwrap(), None)
                .await
                .unwrap_err();
            assert!(err.message.contains("id"), "{id}: {}", err.message);
            assert!(!root.path().join("evil.js").exists(), "{id} escaped");
            assert!(h.installed().unwrap().is_empty(), "{id} was recorded");
        }
    }

    #[tokio::test]
    async fn remove_rejects_ids_that_escape_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let victim = root.path().join("victim.js");
        std::fs::write(&victim, "keep").unwrap();
        let h = host(&root.path().join("extensions"));
        assert!(h.remove("../victim").await.is_err());
        assert!(victim.exists());
    }

    #[tokio::test]
    async fn a_remote_index_cannot_point_at_local_files() {
        let secret_dir = tempfile::tempdir().unwrap();
        let secret = secret_dir.path().join("secret.js");
        std::fs::write(&secret, EXT).unwrap();
        let sha = hex::encode(Sha256::digest(EXT.as_bytes()));
        let file_url = format!("file://{}", secret.to_string_lossy().replace('\\', "/"));
        let index = format!(
            r#"{{ "extensions": [ {{ "id": "demo", "file": "{file_url}", "sha256": "{sha}" }} ] }}"#
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = axum::Router::new().route(
            "/index.json",
            axum::routing::get(move || async move { index }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let data = tempfile::tempdir().unwrap();
        let h = host(data.path());
        let err = h
            .install_repo(&format!("http://{addr}/index.json"), None)
            .await
            .unwrap_err();
        assert!(err.message.contains("http"), "{}", err.message);
        assert!(h.installed().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reinstalling_an_extension_forgets_its_uncertain_title_matches() {
        let repo = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let sha = hex::encode(Sha256::digest(EXT.as_bytes()));
        let index = write_repo(repo.path(), &sha);
        let h = host(data.path());
        h.install_repo(index.to_str().unwrap(), None).await.unwrap();
        {
            let conn = h.db.lock().unwrap();
            for (id, ext, confidence, pinned) in [
                (1, "demo", "fuzzy", 0),
                (2, "demo", "exact", 0),
                (3, "demo", "fuzzy", 1),
                (4, "other", "fuzzy", 0),
            ] {
                conn.execute(
                    "INSERT INTO mappings (shikimori_id, extension_id, match_json, confidence, pinned, updated_at)
                     VALUES (?1, ?2, '{}', ?3, ?4, 0)",
                    params![id, ext, confidence, pinned],
                )
                .unwrap();
            }
        }
        // An update may match titles better; its old guesses must not stick.
        h.install_repo(index.to_str().unwrap(), None).await.unwrap();
        let left: Vec<i64> = {
            let conn = h.db.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT shikimori_id FROM mappings ORDER BY shikimori_id")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        assert_eq!(left, [2, 3, 4]);
    }

    #[tokio::test]
    async fn install_rejects_wrong_sha256() {
        let repo = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let index = write_repo(repo.path(), &"0".repeat(64));
        let err = host(data.path())
            .install_repo(index.to_str().unwrap(), None)
            .await
            .unwrap_err();
        assert!(err.message.contains("sha256"), "{}", err.message);
    }

    #[tokio::test]
    async fn install_loads_extension_and_dubs() {
        let repo = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let sha = hex::encode(Sha256::digest(EXT.as_bytes()));
        let index = write_repo(repo.path(), &sha);
        let h = host(data.path());
        let manifests = h.install_repo(index.to_str().unwrap(), None).await.unwrap();
        assert_eq!(manifests[0].id, "demo");
        assert_eq!(h.list()[0].id, "demo");
        assert!(h.dubs_path().exists());

        // A fresh host over the same database and folder restores installed extensions.
        let db = h.db.clone();
        let h2 = ExtensionHost::new(
            HttpClient::new("ninari-test"),
            db,
            data.path().to_path_buf(),
        );
        h2.load_installed().await.unwrap();
        assert_eq!(h2.list().len(), 1);
    }
}
