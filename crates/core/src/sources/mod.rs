pub mod dubs;
pub mod matching;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::catalog::{Anime, Catalog};
use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::extensions::host::ExtensionHost;
use crate::extensions::runtime::{ExtError, JsExtension};
use crate::extensions::types::{Dub, EpisodeList, Match, SearchQuery, SkipTimes, Stream};
use dubs::DubTable;

const EPISODES_TIMEOUT: Duration = Duration::from_secs(8);
const SOURCES_TTL: Duration = Duration::from_secs(600);
const HEALTH_WINDOW: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub extension_id: String,
    pub max_quality: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DubView {
    pub key: String,
    pub name: String,
    pub max_quality: Option<u32>,
    pub episodes: Vec<i64>,
    pub sources: Vec<SourceRef>,
    pub fuzzy: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceError {
    pub extension_id: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TitleSources {
    pub dubs: Vec<DubView>,
    pub errors: Vec<SourceError>,
    pub skip: HashMap<i64, SkipTimes>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub extension_id: String,
    pub quality: Option<u32>,
    pub health: f64,
}

#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub extension_id: String,
    pub stream: Stream,
    /// Other qualities from the same source (used for seek-bar previews).
    pub alternatives: Vec<Stream>,
    pub dub_name: String,
}

/// Best quality first, then the user's extension order, then recent reliability.
pub fn order_candidates(cands: &mut [Candidate], ext_order: &[String]) {
    let pos = |id: &str| ext_order.iter().position(|x| x == id).unwrap_or(usize::MAX);
    cands.sort_by(|a, b| {
        b.quality
            .unwrap_or(0)
            .cmp(&a.quality.unwrap_or(0))
            .then(pos(&a.extension_id).cmp(&pos(&b.extension_id)))
            .then(
                b.health
                    .partial_cmp(&a.health)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
}

/// What one extension offers for a title.
#[derive(Clone)]
struct ExtSource {
    ext: Arc<JsExtension>,
    matched: Match,
    offset: i64,
    fuzzy: bool,
    list: EpisodeList,
}

impl ExtSource {
    fn id(&self) -> &str {
        &self.ext.manifest().id
    }
}

/// What one source matched for a title, for the screen where the user can correct it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMatch {
    pub extension_id: String,
    pub extension_name: String,
    /// `None` when the source found nothing, or the user said it has no such title.
    pub matched: Option<Match>,
    pub pinned: bool,
    /// What the source's last search returned.
    pub candidates: Vec<Match>,
}

/// How one source did over the last week.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealth {
    pub extension_id: String,
    pub ok: i64,
    pub failed: i64,
    pub last_ok_at: Option<i64>,
    /// The kind of the latest failure: Blocked, GeoBlocked, Challenge, RateLimited or Broken.
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
}

/// A pinned match with this id means "this source has no such title".
const NO_MATCH: &str = "";

/// A match remembered from an earlier search, or set by the user.
struct Stored {
    matched: Match,
    confidence: String,
    offset: i64,
    pinned: bool,
}

/// What each source gave for a title, by extension id.
type Gathered = Vec<(String, Result<Option<ExtSource>, ExtError>)>;
type SourceCache = HashMap<i64, (Instant, Arc<Vec<ExtSource>>)>;

pub struct Resolver {
    catalog: Arc<Catalog>,
    host: Arc<ExtensionHost>,
    db: Db,
    dubs: RwLock<DubTable>,
    cache: Mutex<SourceCache>,
}

impl Resolver {
    pub fn new(catalog: Arc<Catalog>, host: Arc<ExtensionHost>, db: Db, dubs: DubTable) -> Self {
        Self {
            catalog,
            host,
            db,
            dubs: RwLock::new(dubs),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn reload_dubs(&self, table: DubTable) {
        *self.dubs.write().unwrap() = table;
        self.cache.lock().unwrap().clear();
    }

    fn query_for(anime: &Anime) -> SearchQuery {
        SearchQuery {
            shikimori_id: anime.id,
            mal_id: anime.id,
            names: anime.names(),
            year: anime.year,
            kind: anime.kind.clone(),
            episodes: (anime.episodes > 0).then_some(anime.episodes),
        }
    }

    fn stored_mapping(db: &Db, id: i64, ext_id: &str) -> Result<Option<Stored>, AppError> {
        let conn = db.lock().unwrap();
        let row: Option<(String, String, i64, bool)> = conn
            .query_row(
                "SELECT match_json, confidence, episode_offset, pinned FROM mappings WHERE shikimori_id = ?1 AND extension_id = ?2",
                params![id, ext_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(json, confidence, offset, pinned)| {
            Ok(Stored {
                matched: serde_json::from_str(&json)?,
                confidence,
                offset,
                pinned,
            })
        })
        .transpose()
    }

    /// Drops a wrong match along with the episodes indexed from it.
    fn forget_mapping(db: &Db, id: i64, ext_id: &str) -> Result<(), AppError> {
        let conn = db.lock().unwrap();
        conn.execute(
            "DELETE FROM mappings WHERE shikimori_id = ?1 AND extension_id = ?2 AND pinned = 0",
            params![id, ext_id],
        )?;
        conn.execute(
            "DELETE FROM episode_index WHERE shikimori_id = ?1 AND extension_id = ?2",
            params![id, ext_id],
        )?;
        Ok(())
    }

    fn store_mapping(
        db: &Db,
        id: i64,
        ext_id: &str,
        best: &Match,
        all: &[Match],
    ) -> Result<(), AppError> {
        let conn = db.lock().unwrap();
        conn.execute(
            "INSERT INTO mappings (shikimori_id, extension_id, match_json, candidates_json, confidence, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(shikimori_id, extension_id) DO UPDATE SET match_json = excluded.match_json,
               candidates_json = excluded.candidates_json, confidence = excluded.confidence, updated_at = excluded.updated_at
             WHERE pinned = 0",
            params![id, ext_id, serde_json::to_string(best)?, serde_json::to_string(all)?, best.confidence, now_ts()],
        )?;
        Ok(())
    }

    async fn gather_one(
        db: &Db,
        anime: &Anime,
        ext: Arc<JsExtension>,
    ) -> Result<Option<ExtSource>, ExtError> {
        let ext_id = ext.manifest().id.clone();
        let broken = |e: AppError| ExtError::broken(e.message);
        let names = anime.names();
        // A fuzzy match is only a site's search result: it stands only when its title is one of
        // the title's own names. A match the user pinned stands regardless.
        let trusted = |m: &Match| m.confidence == "exact" || matching::plausible(&m.title, &names);
        let stored = Self::stored_mapping(db, anime.id, &ext_id).map_err(broken)?;
        if stored
            .as_ref()
            .is_some_and(|s| s.pinned && s.matched.id == NO_MATCH)
        {
            return Ok(None);
        }
        let (matched, confidence, offset) = match stored {
            // What the user chose is no guess.
            Some(s) if s.pinned => (s.matched, "exact".to_string(), s.offset),
            Some(s) if trusted(&s.matched) => (s.matched, s.confidence, s.offset),
            stale => {
                if stale.is_some() {
                    Self::forget_mapping(db, anime.id, &ext_id).map_err(broken)?;
                }
                let matches = ext.search(&Self::query_for(anime)).await?;
                let mut kept: Vec<(f64, &Match)> = matches
                    .iter()
                    .filter(|m| trusted(m))
                    .map(|m| (matching::score(&m.title, &names), m))
                    .collect();
                kept.sort_by(|(a, x), (b, y)| {
                    (x.confidence != "exact")
                        .cmp(&(y.confidence != "exact"))
                        .then(b.total_cmp(a))
                });
                let Some(best) = kept.first().map(|(_, m)| (*m).clone()) else {
                    return Ok(None);
                };
                Self::store_mapping(db, anime.id, &ext_id, &best, &matches).map_err(broken)?;
                let conf = best.confidence.clone();
                (best, conf, 0)
            }
        };
        let list = ext.episodes(&matched).await?;
        Ok(Some(ExtSource {
            ext,
            matched,
            offset,
            fuzzy: confidence != "exact",
            list,
        }))
    }

    async fn gather(&self, anime: &Anime) -> Gathered {
        let anime = Arc::new(anime.clone());
        let tasks = self.host.all().into_iter().map(|ext| {
            let id = ext.manifest().id.clone();
            let (db, anime) = (self.db.clone(), anime.clone());
            // A task per source: QuickJS work is synchronous, so within one task a source
            // parsing a large list would hold up all the others.
            let task = tokio::spawn(async move {
                tokio::time::timeout(EPISODES_TIMEOUT, Self::gather_one(&db, &anime, ext)).await
            });
            async move {
                let result = match task.await {
                    Ok(Ok(r)) => r,
                    Ok(Err(_)) => Err(ExtError::broken("источник не ответил за 8 секунд")),
                    Err(e) => Err(ExtError::broken(format!("источник упал: {e}"))),
                };
                (id, result)
            }
        });
        join_all(tasks).await
    }

    fn remember_episodes(&self, id: i64, sources: &[ExtSource]) {
        let dubs = self.dubs.read().unwrap();
        let Ok(mut conn) = self.db.lock() else { return };
        // One transaction: committing every row separately syncs the disk thousands of times
        // for a long series (17 s for One Piece).
        let Ok(tx) = conn.transaction() else { return };
        let now = now_ts();
        for s in sources {
            let names: HashMap<&str, &Dub> =
                s.list.dubs.iter().map(|d| (d.id.as_str(), d)).collect();
            for ep in &s.list.episodes {
                for a in &ep.available {
                    let Some(dub) = names.get(a.dub_id.as_str()) else {
                        continue;
                    };
                    let (key, _) = dubs.canonical_dub(dub);
                    let _ = tx.execute(
                        "INSERT OR IGNORE INTO episode_index (shikimori_id, extension_id, episode, dub_key, max_quality, first_seen_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![id, s.id(), ep.number - s.offset, key, a.max_quality, now],
                    );
                }
            }
        }
        if let Err(e) = tx.commit() {
            tracing::warn!("episode index for {id} not saved: {e}");
        }
    }

    async fn sources_for(
        &self,
        id: i64,
        force: bool,
    ) -> Result<(Arc<Vec<ExtSource>>, Vec<SourceError>), AppError> {
        if !force {
            let live = self.host.all();
            if let Some((at, cached)) = self.cache.lock().unwrap().get(&id) {
                // A disabled, removed or updated extension makes the entry stale.
                let current = cached
                    .iter()
                    .all(|s| live.iter().any(|e| Arc::ptr_eq(e, &s.ext)));
                if at.elapsed() < SOURCES_TTL && current {
                    return Ok((cached.clone(), Vec::new()));
                }
            }
        }
        let anime = self.catalog.get(id).await?;
        let mut sources = Vec::new();
        let mut errors = Vec::new();
        for (extension_id, r) in self.gather(&anime).await {
            match r {
                // Finding nothing is an answer too: the source works.
                Ok(found) => {
                    self.record_health(&extension_id, true, None);
                    sources.extend(found);
                }
                Err(e) => {
                    self.record_health(&extension_id, false, Some(e.kind.as_str()));
                    errors.push(SourceError {
                        extension_id,
                        kind: e.kind.as_str().into(),
                        message: e.message,
                    });
                }
            }
        }
        self.remember_episodes(id, &sources);
        let sources = Arc::new(sources);
        self.cache
            .lock()
            .unwrap()
            .insert(id, (Instant::now(), sources.clone()));
        Ok((sources, errors))
    }

    /// Every loaded source with what it matched for the title.
    pub fn matches(&self, id: i64) -> Result<Vec<SourceMatch>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut out = Vec::new();
        for ext in self.host.all() {
            let m = ext.manifest();
            let row: Option<(String, String, bool)> = conn
                .query_row(
                    "SELECT match_json, candidates_json, pinned FROM mappings WHERE shikimori_id = ?1 AND extension_id = ?2",
                    params![id, m.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let (matched, candidates, pinned) = match row {
                Some((json, candidates, pinned)) => (
                    serde_json::from_str::<Match>(&json)
                        .ok()
                        .filter(|x| x.id != NO_MATCH),
                    serde_json::from_str(&candidates).unwrap_or_default(),
                    pinned,
                ),
                None => (None, Vec::new(), false),
            };
            out.push(SourceMatch {
                extension_id: m.id.clone(),
                extension_name: m.name.clone(),
                matched,
                pinned,
                candidates,
            });
        }
        Ok(out)
    }

    /// Makes a source use the match the user chose, or none at all for this title.
    pub fn pin_match(&self, id: i64, ext_id: &str, chosen: Option<Match>) -> Result<(), AppError> {
        let chosen = chosen.unwrap_or(Match {
            id: NO_MATCH.into(),
            title: String::new(),
            confidence: "none".into(),
            data: serde_json::Value::Null,
        });
        {
            let conn = self.db.lock().unwrap();
            conn.execute(
                "INSERT INTO mappings (shikimori_id, extension_id, match_json, confidence, pinned, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)
                 ON CONFLICT(shikimori_id, extension_id) DO UPDATE SET match_json = excluded.match_json,
                   confidence = excluded.confidence, pinned = 1, episode_offset = 0, updated_at = excluded.updated_at",
                params![id, ext_id, serde_json::to_string(&chosen)?, chosen.confidence, now_ts()],
            )?;
            conn.execute(
                "DELETE FROM episode_index WHERE shikimori_id = ?1 AND extension_id = ?2",
                params![id, ext_id],
            )?;
        }
        self.cache.lock().unwrap().remove(&id);
        Ok(())
    }

    /// Forgets the user's choice: the source searches for the title again.
    pub fn unpin(&self, id: i64, ext_id: &str) -> Result<(), AppError> {
        {
            let conn = self.db.lock().unwrap();
            conn.execute(
                "DELETE FROM mappings WHERE shikimori_id = ?1 AND extension_id = ?2",
                params![id, ext_id],
            )?;
            conn.execute(
                "DELETE FROM episode_index WHERE shikimori_id = ?1 AND extension_id = ?2",
                params![id, ext_id],
            )?;
        }
        self.cache.lock().unwrap().remove(&id);
        Ok(())
    }

    /// Searches one source by the user's own query, for a title it did not find by itself.
    pub async fn search_in(
        &self,
        id: i64,
        ext_id: &str,
        query: &str,
    ) -> Result<Vec<Match>, AppError> {
        let ext = self
            .host
            .all()
            .into_iter()
            .find(|e| e.manifest().id == ext_id)
            .ok_or_else(|| AppError::new("NotFound", format!("источник {ext_id} не подключён")))?;
        let anime = self.catalog.get(id).await?;
        let mut q = Self::query_for(&anime);
        q.names = vec![query.trim().to_string()];
        ext.search(&q)
            .await
            .map_err(|e| AppError::new("Source", e.message))
    }

    pub async fn title_sources(&self, id: i64) -> Result<TitleSources, AppError> {
        let (sources, errors) = self.sources_for(id, true).await?;
        let dubs = self.dubs.read().unwrap().clone();
        let mut views: HashMap<String, (DubView, BTreeSet<i64>)> = HashMap::new();
        let mut skip = HashMap::new();
        for s in sources.iter() {
            let names: HashMap<&str, &Dub> =
                s.list.dubs.iter().map(|d| (d.id.as_str(), d)).collect();
            for ep in &s.list.episodes {
                let number = ep.number - s.offset;
                if let Some(times) = &ep.skip {
                    skip.entry(number).or_insert_with(|| times.clone());
                }
                for a in &ep.available {
                    let Some(dub) = names.get(a.dub_id.as_str()) else {
                        continue;
                    };
                    let (key, display) = dubs.canonical_dub(dub);
                    let (view, episodes) = views.entry(key.clone()).or_insert_with(|| {
                        (
                            DubView {
                                key,
                                name: display,
                                max_quality: None,
                                episodes: vec![],
                                sources: vec![],
                                fuzzy: false,
                            },
                            BTreeSet::new(),
                        )
                    });
                    episodes.insert(number);
                    view.max_quality = view.max_quality.max(a.max_quality);
                    view.fuzzy |= s.fuzzy;
                    match view.sources.iter_mut().find(|r| r.extension_id == s.id()) {
                        Some(r) => r.max_quality = r.max_quality.max(a.max_quality),
                        None => view.sources.push(SourceRef {
                            extension_id: s.id().to_string(),
                            max_quality: a.max_quality,
                        }),
                    }
                }
            }
        }
        let order: Vec<String> = self.host.list().into_iter().map(|m| m.id).collect();
        let pos = |id: &str| order.iter().position(|x| x == id).unwrap_or(usize::MAX);
        let mut dubs: Vec<DubView> = views
            .into_values()
            .map(|(mut v, eps)| {
                v.episodes = eps.into_iter().collect();
                v.sources.sort_by(|a, b| {
                    b.max_quality
                        .cmp(&a.max_quality)
                        .then(pos(&a.extension_id).cmp(&pos(&b.extension_id)))
                });
                v
            })
            .collect();
        dubs.sort_by(|a, b| {
            b.max_quality
                .cmp(&a.max_quality)
                .then(b.episodes.len().cmp(&a.episodes.len()))
                .then(a.name.cmp(&b.name))
        });
        self.remember_names(dubs.iter().map(|d| (d.key.as_str(), d.name.as_str())));
        Ok(TitleSources { dubs, errors, skip })
    }

    fn health(&self, ext_id: &str) -> f64 {
        let Ok(conn) = self.db.lock() else { return 1.0 };
        conn.query_row(
            "SELECT AVG(ok) FROM extension_health WHERE extension_id = ?1 AND ts > ?2",
            params![ext_id, now_ts() - HEALTH_WINDOW],
            |r| r.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or(1.0)
    }

    fn record_health(&self, ext_id: &str, ok: bool, kind: Option<&str>) {
        if let Ok(conn) = self.db.lock() {
            let now = now_ts();
            let _ = conn.execute(
                "INSERT INTO extension_health (extension_id, ts, ok, error_kind) VALUES (?1, ?2, ?3, ?4)",
                params![ext_id, now, ok as i64, kind],
            );
            let _ = conn.execute(
                "DELETE FROM extension_health WHERE ts < ?1",
                params![now - HEALTH_WINDOW],
            );
        }
    }

    /// How each loaded source did over the last week. A title the source lacks is an answer,
    /// not a failure.
    pub fn health_report(&self) -> Result<Vec<SourceHealth>, AppError> {
        let conn = self.db.lock().unwrap();
        let since = now_ts() - HEALTH_WINDOW;
        let mut out = Vec::new();
        for ext in self.host.all() {
            let id = &ext.manifest().id;
            let (ok, failed, last_ok_at, last_error_at): (i64, i64, Option<i64>, Option<i64>) = conn
                .query_row(
                    "SELECT COALESCE(SUM(ok), 0),
                            COALESCE(SUM(ok = 0 AND COALESCE(error_kind, '') != 'NotFound'), 0),
                            MAX(CASE WHEN ok = 1 THEN ts END),
                            MAX(CASE WHEN ok = 0 AND COALESCE(error_kind, '') != 'NotFound' THEN ts END)
                     FROM extension_health WHERE extension_id = ?1 AND ts > ?2",
                    params![id, since],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
            let last_error: Option<String> = conn
                .query_row(
                    "SELECT error_kind FROM extension_health
                     WHERE extension_id = ?1 AND ts > ?2 AND ok = 0 AND COALESCE(error_kind, '') != 'NotFound'
                     ORDER BY ts DESC, rowid DESC LIMIT 1",
                    params![id, since],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            out.push(SourceHealth {
                extension_id: id.clone(),
                ok,
                failed,
                last_ok_at,
                last_error,
                last_error_at,
            });
        }
        Ok(out)
    }

    /// Display name for a dub key ("aniliberty" -> "AniLibria"): as a source last spelled it,
    /// otherwise from the dub table, which knows only the teams it lists.
    pub fn dub_name(&self, key: &str) -> String {
        let seen: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT name FROM dub_names WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .ok()
            .flatten();
        seen.unwrap_or_else(|| self.dubs.read().unwrap().display_name(key))
    }

    /// Keeps how the sources spell dub names, for screens that only have the key.
    fn remember_names<'a>(&self, names: impl IntoIterator<Item = (&'a str, &'a str)>) {
        let conn = self.db.lock().unwrap();
        for (key, name) in names {
            if let Err(e) = conn.execute(
                "INSERT INTO dub_names (key, name) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET name = excluded.name",
                [key, name],
            ) {
                tracing::warn!("cannot keep the dub name {name}: {e}");
            }
        }
    }

    /// Episode numbers (Shikimori numbering) available in a dub across all sources.
    pub async fn episodes_in_dub(&self, id: i64, dub_key: &str) -> Result<Vec<i64>, AppError> {
        let (sources, _) = self.sources_for(id, false).await?;
        let dubs = self.dubs.read().unwrap().clone();
        let mut out = BTreeSet::new();
        for s in sources.iter() {
            let ids: Vec<&str> = s
                .list
                .dubs
                .iter()
                .filter(|d| dubs.canonical_dub(d).0 == dub_key)
                .map(|d| d.id.as_str())
                .collect();
            for ep in &s.list.episodes {
                if ep
                    .available
                    .iter()
                    .any(|a| ids.contains(&a.dub_id.as_str()))
                {
                    out.insert(ep.number - s.offset);
                }
            }
        }
        Ok(out.into_iter().collect())
    }

    /// The dub to use when the user has not chosen one: the best one `title_sources` lists first.
    pub async fn default_dub(&self, id: i64) -> Result<Option<String>, AppError> {
        Ok(self
            .title_sources(id)
            .await?
            .dubs
            .into_iter()
            .next()
            .map(|d| d.key))
    }

    /// Opening and ending timings for an episode, when any cached source knows them.
    pub async fn skip_for(&self, id: i64, episode: i64) -> Option<SkipTimes> {
        let (sources, _) = self.sources_for(id, false).await.ok()?;
        sources.iter().find_map(|s| {
            s.list
                .episodes
                .iter()
                .find(|e| e.number - s.offset == episode)
                .and_then(|e| e.skip.clone())
                .filter(|t| t.opening.is_some() || t.ending.is_some())
        })
    }

    /// Finds a playable stream for an episode in the given dub, trying sources best-first.
    pub async fn resolve(
        &self,
        id: i64,
        episode: i64,
        dub_key: &str,
    ) -> Result<ResolvedStream, AppError> {
        let (sources, _) = self.sources_for(id, false).await?;
        let dubs = self.dubs.read().unwrap().clone();
        let mut dub_name = dub_key.to_string();
        let mut options: Vec<(Candidate, ExtSource, String)> = Vec::new();
        for s in sources.iter() {
            let source_episode = episode + s.offset;
            let Some(ep) = s.list.episodes.iter().find(|e| e.number == source_episode) else {
                continue;
            };
            for d in &s.list.dubs {
                let (key, display) = dubs.canonical_dub(d);
                if key != dub_key {
                    continue;
                }
                dub_name = display;
                if let Some(a) = ep.available.iter().find(|a| a.dub_id == d.id) {
                    let cand = Candidate {
                        extension_id: s.id().to_string(),
                        quality: a.max_quality,
                        health: self.health(s.id()),
                    };
                    options.push((cand, s.clone(), d.id.clone()));
                }
            }
        }
        if options.is_empty() {
            return Err(AppError::new(
                "NoSource",
                format!("Серии {episode} в озвучке {dub_name} пока нет ни в одном источнике"),
            ));
        }
        self.remember_names([(dub_key, dub_name.as_str())]);
        let order: Vec<String> = self.host.list().into_iter().map(|m| m.id).collect();
        let mut cands: Vec<Candidate> = options.iter().map(|(c, _, _)| c.clone()).collect();
        order_candidates(&mut cands, &order);
        let mut last = None;
        for cand in cands {
            let Some((_, source, dub_id)) = options
                .iter()
                .find(|(c, _, _)| c.extension_id == cand.extension_id)
            else {
                continue;
            };
            match source
                .ext
                .resolve(&source.matched, episode + source.offset, dub_id)
                .await
            {
                Ok(mut streams) if !streams.is_empty() => {
                    streams.sort_by_key(|s| std::cmp::Reverse(s.quality));
                    self.record_health(&cand.extension_id, true, None);
                    let best = streams.remove(0);
                    return Ok(ResolvedStream {
                        extension_id: cand.extension_id,
                        stream: best,
                        alternatives: streams,
                        dub_name,
                    });
                }
                Ok(_) => last = Some(ExtError::broken("источник не вернул ни одного потока")),
                Err(e) => {
                    self.record_health(&cand.extension_id, false, Some(e.kind.as_str()));
                    last = Some(e);
                }
            }
        }
        let e = last.unwrap_or_else(|| ExtError::broken("нет источников"));
        Err(AppError::new(
            e.kind.as_str(),
            format!("Не удалось запустить серию {episode}: {}", e.message),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{parse_animes, Catalog};
    use crate::db;
    use crate::extensions::host::ExtensionHost;
    use crate::extensions::runtime::{JsExtension, KvStore};
    use crate::net::HttpClient;
    use std::sync::{Arc, Mutex};

    #[test]
    fn orders_candidates_by_quality_then_user_order_then_health() {
        let mut c = vec![
            Candidate {
                extension_id: "kodik".into(),
                quality: Some(720),
                health: 1.0,
            },
            Candidate {
                extension_id: "cvh".into(),
                quality: Some(1080),
                health: 0.5,
            },
            Candidate {
                extension_id: "anilibria".into(),
                quality: Some(1080),
                health: 0.9,
            },
            Candidate {
                extension_id: "unknown".into(),
                quality: None,
                health: 1.0,
            },
        ];
        order_candidates(&mut c, &["cvh".into(), "anilibria".into(), "kodik".into()]);
        let ids: Vec<_> = c.iter().map(|x| x.extension_id.as_str()).collect();
        assert_eq!(ids, ["cvh", "anilibria", "kodik", "unknown"]);
    }

    fn ext_src(id: &str, body: &str) -> String {
        format!(
            r#"export const manifest = {{ id: "{id}", name: "{id}", version: "0.0.1", apiVersion: 1,
                 domains: ["https://example.org"] }};
               {body}"#
        )
    }

    async fn resolver() -> Resolver {
        let alpha = ext_src(
            "alpha",
            r#"export async function search(q) { return [{ id: "a1", title: "A", confidence: "exact" }]; }
               export async function episodes(m) {
                 return { dubs: [{ id: "x1", name: "AniDub Online", type: "voice" }],
                          episodes: [1, 2, 3].map(n => ({ number: n, available: [{ dubId: "x1", maxQuality: 720 }] })) };
               }
               export async function resolve(m, ep, dub) {
                 return [{ url: "https://a.example/" + ep + "/720.m3u8", kind: "hls", quality: 720 }];
               }"#,
        );
        let beta = ext_src(
            "beta",
            r#"export async function search(q) {
                 return [{ id: "b1", title: "Фрирен: провожающая в последний путь", confidence: "fuzzy" }];
               }
               export async function episodes(m) {
                 return { dubs: [{ id: "y", name: "AniDUB", type: "voice" }],
                          episodes: [1, 2].map(n => ({ number: n, available: [{ dubId: "y", maxQuality: 1080 }] })) };
               }
               export async function resolve(m, ep, dub, ctx) { ctx.fail.geoBlocked("регион"); }"#,
        );
        let gamma = ext_src(
            "gamma",
            r#"export async function search(q) { throw new Error("boom"); }"#,
        );

        resolver_with(vec![alpha, beta, gamma]).await
    }

    async fn resolver_with(sources: Vec<String>) -> Resolver {
        resolver_on(Arc::new(Mutex::new(db::open_in_memory().unwrap())), sources).await
    }

    async fn resolver_on(db: crate::db::Db, sources: Vec<String>) -> Resolver {
        let http = HttpClient::new("ninari-test");
        let catalog = Arc::new(Catalog::new(
            http.clone(),
            db.clone(),
            vec!["http://127.0.0.1:9".into()],
        ));
        catalog
            .store(
                &parse_animes(include_str!("../../tests/fixtures/shikimori_search.json")).unwrap()
                    [0],
            )
            .unwrap();
        let host = Arc::new(ExtensionHost::new(
            http.clone(),
            db.clone(),
            std::env::temp_dir(),
        ));
        let mut loaded = Vec::new();
        for src in sources {
            let id = src.split('"').nth(1).unwrap().to_string();
            loaded.push(Arc::new(
                JsExtension::load(&src, http.clone(), KvStore::new(db.clone(), id))
                    .await
                    .unwrap(),
            ));
        }
        host.replace_loaded(loaded);
        let dubs = dubs::DubTable::from_json(
            r#"{"anidub":{"name":"AniDUB","aliases":["anidub","anidubonline"]}}"#,
        );
        Resolver::new(catalog, host, db, dubs)
    }

    #[tokio::test]
    async fn a_dub_outside_the_table_keeps_the_name_its_source_gave() {
        let src = ext_src(
            "delta",
            r#"export async function search(q) { return [{ id: "d1", title: "A", confidence: "exact" }]; }
               export async function episodes(m) {
                 return { dubs: [{ id: "z", name: "Дубляж YoukaiStudio", type: "voice" }],
                          episodes: [{ number: 1, available: [{ dubId: "z", maxQuality: 720 }] }] };
               }"#,
        );
        let r = resolver_with(vec![src]).await;
        assert_eq!(r.dub_name("дубляжyoukaistudio"), "дубляжyoukaistudio");
        let s = r.title_sources(52991).await.unwrap();
        assert_eq!(s.dubs[0].key, "дубляжyoukaistudio");
        assert_eq!(r.dub_name("дубляжyoukaistudio"), "Дубляж YoukaiStudio");
        assert_eq!(
            r.dub_name("anidub"),
            "AniDUB",
            "the table still names its teams"
        );
    }

    #[tokio::test]
    async fn title_sources_merges_dubs_across_extensions() {
        let r = resolver().await;
        let s = r.title_sources(52991).await.unwrap();
        assert_eq!(s.dubs.len(), 1, "{:?}", s.dubs);
        let d = &s.dubs[0];
        assert_eq!(d.key, "anidub");
        assert_eq!(d.name, "AniDUB");
        assert_eq!(d.max_quality, Some(1080));
        assert_eq!(d.episodes, vec![1, 2, 3]);
        assert!(d.fuzzy, "beta matched fuzzily");
        let ids: Vec<_> = d.sources.iter().map(|s| s.extension_id.as_str()).collect();
        assert_eq!(ids, ["beta", "alpha"]);
        assert_eq!(s.errors.len(), 1);
        assert_eq!(s.errors[0].extension_id, "gamma");
    }

    #[tokio::test]
    async fn resolve_falls_back_to_next_candidate() {
        let r = resolver().await;
        let got = r.resolve(52991, 1, "anidub").await.unwrap();
        assert_eq!(got.extension_id, "alpha");
        assert_eq!(got.stream.url, "https://a.example/1/720.m3u8");
        assert_eq!(got.dub_name, "AniDUB");
    }

    #[tokio::test]
    async fn dub_helpers_use_cached_sources() {
        let r = resolver().await;
        r.title_sources(52991).await.unwrap();
        assert_eq!(
            r.episodes_in_dub(52991, "anidub").await.unwrap(),
            vec![1, 2, 3]
        );
        assert!(r.episodes_in_dub(52991, "shiza").await.unwrap().is_empty());
        assert_eq!(
            r.default_dub(52991).await.unwrap().as_deref(),
            Some("anidub")
        );
        assert_eq!(r.skip_for(52991, 1).await, None);
    }

    #[tokio::test]
    async fn an_extension_that_went_away_is_not_used_from_the_cache() {
        let r = resolver().await;
        assert_eq!(
            r.resolve(52991, 1, "anidub").await.unwrap().extension_id,
            "alpha"
        );
        let rest = r
            .host
            .all()
            .into_iter()
            .filter(|e| e.manifest().id != "alpha")
            .collect();
        r.host.replace_loaded(rest);
        let after = r.resolve(52991, 1, "anidub").await;
        assert!(
            after.as_ref().map_or(true, |g| g.extension_id != "alpha"),
            "the disabled extension still played"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sources_are_gathered_in_parallel() {
        let busy = |id: &str| {
            ext_src(
                id,
                r#"export async function search(q) {
                     const end = Date.now() + 600;
                     while (Date.now() < end) {}
                     return [{ id: "s", title: "S", confidence: "exact" }];
                   }
                   export async function episodes(m) { return { dubs: [], episodes: [] }; }"#,
            )
        };
        let r = resolver_with(vec![busy("one"), busy("two"), busy("three")]).await;
        let started = std::time::Instant::now();
        r.title_sources(52991).await.unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(1500),
            "sources ran one by one: {took:?}"
        );
    }

    #[tokio::test]
    async fn a_long_series_is_indexed_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Mutex::new(db::open(&dir.path().join("t.db")).unwrap()));
        let long = ext_src(
            "long",
            r#"export async function search(q) { return [{ id: "l", title: "L", confidence: "exact" }]; }
               export async function episodes(m) {
                 const dubs = Array.from({ length: 10 }, (_, i) => ({ id: "d" + i, name: "Team " + i, type: "voice" }));
                 return { dubs, episodes: Array.from({ length: 300 }, (_, n) => ({
                   number: n + 1, available: dubs.map(d => ({ dubId: d.id, maxQuality: 720 })) })) };
               }"#,
        );
        let r = resolver_on(db, vec![long]).await;
        let started = std::time::Instant::now();
        r.title_sources(52991).await.unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(1),
            "indexing 3000 episodes took {took:?}"
        );
    }

    fn one_dub(id: &str, search: &str) -> String {
        ext_src(
            id,
            &format!(
                r#"export async function search(q) {{ return {search}; }}
                   export async function episodes(m) {{
                     return {{ dubs: [{{ id: "d", name: "Other Team", type: "voice" }}],
                              episodes: [{{ number: 1, available: [{{ dubId: "d", maxQuality: 720 }}] }}] }};
                   }}"#
            ),
        )
    }

    fn mapping_of(r: &Resolver, ext: &str) -> Option<(String, i64)> {
        r.db.lock()
            .unwrap()
            .query_row(
                "SELECT match_json, pinned FROM mappings WHERE shikimori_id = 52991 AND extension_id = ?1",
                [ext],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn put_mapping(r: &Resolver, ext: &str, title: &str, pinned: i64) {
        let json = serde_json::json!({ "id": "old", "title": title, "confidence": "fuzzy" });
        r.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO mappings (shikimori_id, extension_id, match_json, confidence, pinned, updated_at)
                 VALUES (52991, ?1, ?2, 'fuzzy', ?3, 0)",
                params![ext, json.to_string(), pinned],
            )
            .unwrap();
    }

    #[tokio::test]
    async fn an_unrelated_fuzzy_match_is_not_used() {
        let noise = one_dub(
            "noise",
            r#"[{ id: "n", title: "Реинкарнация аристократа", confidence: "fuzzy" }]"#,
        );
        let r = resolver_with(vec![noise]).await;
        let s = r.title_sources(52991).await.unwrap();
        assert!(s.dubs.is_empty(), "{:?}", s.dubs);
        assert_eq!(mapping_of(&r, "noise"), None);
    }

    #[tokio::test]
    async fn the_closest_plausible_candidate_wins() {
        let many = one_dub(
            "many",
            r#"[{ id: "far", title: "Провожающая в последний путь Фрирен (ТВ)", confidence: "fuzzy" },
                { id: "near", title: "Провожающая в последний путь Фрирен", confidence: "fuzzy" }]"#,
        );
        let r = resolver_with(vec![many]).await;
        r.title_sources(52991).await.unwrap();
        let (json, _) = mapping_of(&r, "many").unwrap();
        assert!(json.contains(r#""id":"near""#), "{json}");
    }

    #[tokio::test]
    async fn a_stored_wrong_match_is_dropped() {
        let quiet = one_dub("quiet", "[]");
        let r = resolver_with(vec![quiet]).await;
        put_mapping(&r, "quiet", "Расхититель гробниц", 0);
        r.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO episode_index (shikimori_id, extension_id, episode, dub_key, max_quality, first_seen_at)
                 VALUES (52991, 'quiet', 1, 'x', 720, 0)",
                [],
            )
            .unwrap();
        let s = r.title_sources(52991).await.unwrap();
        assert!(s.dubs.is_empty(), "{:?}", s.dubs);
        assert_eq!(mapping_of(&r, "quiet"), None);
        let indexed: i64 =
            r.db.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM episode_index WHERE extension_id = 'quiet'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(indexed, 0, "episodes of the wrong match stay indexed");
    }

    #[tokio::test]
    async fn a_match_the_user_pinned_is_kept() {
        let quiet = one_dub("quiet", "[]");
        let r = resolver_with(vec![quiet]).await;
        put_mapping(&r, "quiet", "Расхититель гробниц", 1);
        let s = r.title_sources(52991).await.unwrap();
        assert_eq!(s.dubs.len(), 1, "{:?}", s.dubs);
        assert!(mapping_of(&r, "quiet").is_some());
    }

    fn fuzzy_match(id: &str, title: &str) -> Match {
        Match {
            id: id.into(),
            title: title.into(),
            confidence: "fuzzy".into(),
            data: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn the_user_can_pin_a_match_the_resolver_rejected() {
        let noise = one_dub(
            "noise",
            r#"[{ id: "n", title: "Реинкарнация аристократа", confidence: "fuzzy" }]"#,
        );
        let r = resolver_with(vec![noise]).await;
        assert!(r.title_sources(52991).await.unwrap().dubs.is_empty());
        r.pin_match(
            52991,
            "noise",
            Some(fuzzy_match("n", "Реинкарнация аристократа")),
        )
        .unwrap();
        let s = r.title_sources(52991).await.unwrap();
        assert_eq!(s.dubs.len(), 1);
        assert!(!s.dubs[0].fuzzy, "a match the user chose is not a guess");
        let m = r.matches(52991).unwrap();
        assert_eq!(m.len(), 1);
        assert!(m[0].pinned);
        assert_eq!(m[0].matched.as_ref().unwrap().id, "n");
    }

    #[tokio::test]
    async fn a_source_can_be_told_it_has_no_such_title_and_asked_again() {
        let r = resolver().await;
        r.title_sources(52991).await.unwrap();
        r.pin_match(52991, "alpha", None).unwrap();
        let s = r.title_sources(52991).await.unwrap();
        assert!(
            s.dubs
                .iter()
                .all(|d| d.sources.iter().all(|x| x.extension_id != "alpha")),
            "{:?}",
            s.dubs
        );
        let alpha = r
            .matches(52991)
            .unwrap()
            .into_iter()
            .find(|m| m.extension_id == "alpha")
            .unwrap();
        assert!(alpha.pinned && alpha.matched.is_none());
        r.unpin(52991, "alpha").unwrap();
        let back = r.title_sources(52991).await.unwrap();
        assert!(back
            .dubs
            .iter()
            .any(|d| d.sources.iter().any(|x| x.extension_id == "alpha")));
    }

    #[tokio::test]
    async fn every_source_is_listed_with_its_candidates() {
        let r = resolver().await;
        r.title_sources(52991).await.unwrap();
        let m = r.matches(52991).unwrap();
        let ids: Vec<_> = m.iter().map(|x| x.extension_id.as_str()).collect();
        assert_eq!(ids, ["alpha", "beta", "gamma"]);
        assert_eq!(m[0].candidates.len(), 1);
        assert!(
            m[2].matched.is_none() && !m[2].pinned,
            "gamma failed to search"
        );
    }

    #[tokio::test]
    async fn one_source_can_be_searched_by_hand() {
        let echo = one_dub(
            "echo",
            r#"[{ id: "e", title: q.names.join("|"), confidence: "fuzzy" }]"#,
        );
        let r = resolver_with(vec![echo]).await;
        let found = r.search_in(52991, "echo", "Фрирен ТВ").await.unwrap();
        assert_eq!(found[0].title, "Фрирен ТВ");
    }

    #[tokio::test]
    async fn health_counts_what_worked_and_what_broke_this_week() {
        let r = resolver().await;
        r.title_sources(52991).await.unwrap();
        let report = r.health_report().unwrap();
        let ids: Vec<_> = report.iter().map(|h| h.extension_id.as_str()).collect();
        assert_eq!(ids, ["alpha", "beta", "gamma"]);
        assert_eq!(
            (report[0].ok, report[0].failed),
            (1, 0),
            "a gathered source works"
        );
        assert_eq!((report[2].ok, report[2].failed), (0, 1));
        assert_eq!(report[2].last_error.as_deref(), Some("Broken"));
        assert!(report[2].last_error_at.is_some());
    }

    #[tokio::test]
    async fn a_title_the_source_lacks_is_not_a_failure() {
        let quiet = one_dub("quiet", "[]");
        let r = resolver_with(vec![quiet]).await;
        r.title_sources(52991).await.unwrap();
        r.record_health("quiet", false, Some("NotFound"));
        let report = r.health_report().unwrap();
        assert_eq!((report[0].ok, report[0].failed), (1, 0));
    }

    #[tokio::test]
    async fn resolve_reports_missing_episode() {
        let r = resolver().await;
        let err = r.resolve(52991, 5, "anidub").await.unwrap_err();
        assert_eq!(err.kind, "NoSource");
        assert!(err.message.contains("5"), "{}", err.message);
    }
}
