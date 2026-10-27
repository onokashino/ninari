pub mod anilist;
pub mod aniskip;
pub mod browse;
pub mod calendar;
pub mod details;
pub mod extras;
pub mod jikan;
pub mod people;
pub mod posters;
pub mod recommend;
pub mod sequels;
pub mod trace;

use std::time::{Duration, Instant};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;

use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::net::{FetchRequest, HttpClient};

pub const SHIKIMORI_DOMAINS: [&str; 2] = ["https://shikimori.io", "https://shikimori.one"];

/// The fields of a title every query asks for, as `parse_animes` reads them.
macro_rules! anime_fields {
    () => {
        "id name russian english japanese synonyms kind status score episodes episodesAired
    airedOn { year } nextEpisodeAt poster { mainUrl originalUrl }
    genres { russian } studios { id name } description"
    };
}

const QUERY: &str = concat!(
    "query($search: String, $ids: String, $limit: PositiveInt) {
  animes(search: $search, ids: $ids, limit: $limit, censored: false) {
    ",
    anime_fields!(),
    "
  }
}"
);

/// The most popular series of a season.
const SEASON_QUERY: &str = concat!(
    r#"query($season: SeasonString, $limit: PositiveInt) {
  animes(season: $season, order: popularity, kind: "tv,ona", limit: $limit, censored: false) {
    "#,
    anime_fields!(),
    "
  }
}"
);

/// A page of the catalog; the variables come from `browse::browse_variables`.
const BROWSE_QUERY: &str = concat!(
    "query($page: PositiveInt, $limit: PositiveInt, $order: OrderEnum, $kind: AnimeKindString,
  $status: AnimeStatusString, $season: SeasonString, $score: Int, $genre: String, $studio: String,
  $rating: RatingString, $duration: DurationString, $search: String) {
  animes(page: $page, limit: $limit, order: $order, kind: $kind, status: $status, season: $season,
    score: $score, genre: $genre, studio: $studio, rating: $rating, duration: $duration,
    search: $search, censored: true) {
    ",
    anime_fields!(),
    "
  }
}"
);

/// Shikimori's season of a month: winter is January to March, and so on.
pub fn season_of(year: i64, month: i64) -> String {
    let name = match month {
        1..=3 => "winter",
        4..=6 => "spring",
        7..=9 => "summer",
        _ => "fall",
    };
    format!("{name}_{year}")
}

const SEASON_TTL: Duration = Duration::from_secs(6 * 3600);

/// Shikimori allows 5 requests per second; keep a safe gap between calls.
const MIN_INTERVAL: Duration = Duration::from_millis(220);
const TTL_ONGOING: i64 = 6 * 3600;
const TTL_RELEASED: i64 = 7 * 24 * 3600;
const CALENDAR_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Anime {
    pub id: i64,
    pub name: String,
    pub russian: Option<String>,
    pub english: Option<String>,
    pub japanese: Option<String>,
    pub synonyms: Vec<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub episodes: i64,
    pub episodes_aired: i64,
    pub year: Option<i64>,
    /// Shikimori's average score out of ten, when anyone rated it.
    #[serde(default)]
    pub score: Option<f64>,
    pub next_episode_at: Option<String>,
    pub poster_url: Option<String>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    /// Shikimori ids of `studios`, for the catalog; empty for titles cached before they were kept.
    #[serde(default)]
    pub studio_ids: Vec<i64>,
    pub description: Option<String>,
}

impl Anime {
    /// All known names, Russian first, without duplicates.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let candidates = [
            &self.russian,
            &Some(self.name.clone()),
            &self.english,
            &self.japanese,
        ];
        for n in candidates.into_iter().flatten().chain(self.synonyms.iter()) {
            let n = n.trim();
            if !n.is_empty() && !out.iter().any(|x| x == n) {
                out.push(n.to_string());
            }
        }
        out
    }

    /// Episodes that are already out: all of them for a finished title, the aired ones otherwise.
    pub fn available_episodes(&self) -> i64 {
        match self.status.as_deref() {
            Some("released") => self.episodes.max(self.episodes_aired),
            _ => self.episodes_aired,
        }
    }

    pub fn is_ongoing(&self) -> bool {
        matches!(self.status.as_deref(), Some("ongoing") | Some("anons"))
    }
}

#[derive(Deserialize)]
struct Gql {
    data: Option<GqlData>,
    errors: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GqlData {
    animes: Vec<RawAnime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAnime {
    id: String,
    name: String,
    russian: Option<String>,
    english: Option<String>,
    japanese: Option<String>,
    #[serde(default)]
    synonyms: Vec<String>,
    kind: Option<String>,
    status: Option<String>,
    score: Option<f64>,
    #[serde(default)]
    episodes: i64,
    #[serde(default)]
    episodes_aired: i64,
    aired_on: Option<RawYear>,
    next_episode_at: Option<String>,
    poster: Option<RawPoster>,
    #[serde(default)]
    genres: Vec<RawGenre>,
    #[serde(default)]
    studios: Vec<RawStudio>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct RawYear {
    year: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPoster {
    main_url: Option<String>,
    original_url: Option<String>,
}

#[derive(Deserialize)]
struct RawGenre {
    russian: Option<String>,
}

#[derive(Deserialize)]
struct RawStudio {
    id: Option<String>,
    name: String,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|x| !x.trim().is_empty())
}

/// Removes Shikimori BBCode tags like `[character=1]` and `[/character]`, keeping their text.
pub(crate) fn strip_bbcode(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            let end = chars[i + 1..].iter().take(120).position(|&c| c == ']');
            if let Some(len) = end {
                let inner = &chars[i + 1..i + 1 + len];
                if !inner.is_empty() && inner.iter().all(|c| !c.is_whitespace()) {
                    i += len + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Ids of the titles Shikimori finds similar, best first, without music videos and ads.
pub fn parse_similar_ids(json: &str) -> Result<Vec<i64>, AppError> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json)?;
    Ok(list
        .iter()
        .filter(|a| !matches!(a["kind"].as_str(), Some("music" | "pv" | "cm")))
        .filter_map(|a| a["id"].as_i64())
        .collect())
}

pub fn parse_animes(json: &str) -> Result<Vec<Anime>, AppError> {
    let gql: Gql = serde_json::from_str(json)?;
    if let Some(errors) = gql.errors {
        return Err(AppError::new("Catalog", format!("shikimori: {errors}")));
    }
    let raw = gql.data.map(|d| d.animes).unwrap_or_default();
    raw.into_iter()
        .map(|r| {
            let id =
                r.id.parse::<i64>()
                    .map_err(|_| AppError::new("Catalog", format!("bad id {}", r.id)))?;
            Ok(Anime {
                id,
                name: r.name,
                russian: non_empty(r.russian),
                english: non_empty(r.english),
                japanese: non_empty(r.japanese),
                synonyms: r.synonyms,
                kind: r.kind,
                status: r.status,
                episodes: r.episodes,
                episodes_aired: r.episodes_aired,
                year: r.aired_on.and_then(|y| y.year),
                score: r.score.filter(|s| *s > 0.),
                next_episode_at: r.next_episode_at,
                poster_url: r.poster.and_then(|p| p.main_url.or(p.original_url)),
                genres: r.genres.into_iter().filter_map(|g| g.russian).collect(),
                studio_ids: r
                    .studios
                    .iter()
                    .filter_map(|s| s.id.as_deref()?.parse().ok())
                    .collect(),
                studios: r.studios.into_iter().map(|s| s.name).collect(),
                description: non_empty(r.description.map(|d| strip_bbcode(&d))),
            })
        })
        .collect()
}

pub struct Catalog {
    http: HttpClient,
    db: Db,
    domains: Vec<String>,
    last_request: AsyncMutex<Option<Instant>>,
    calendar: AsyncMutex<Option<(Instant, Vec<calendar::CalendarEntry>)>>,
    season: AsyncMutex<Option<(Instant, Vec<Anime>)>>,
    genres: AsyncMutex<Option<Vec<browse::Genre>>>,
}

impl Catalog {
    pub fn new(http: HttpClient, db: Db, domains: Vec<String>) -> Self {
        Self {
            http,
            db,
            domains,
            last_request: AsyncMutex::new(None),
            calendar: AsyncMutex::new(None),
            season: AsyncMutex::new(None),
            genres: AsyncMutex::new(None),
        }
    }

    async fn query(&self, variables: serde_json::Value) -> Result<Vec<Anime>, AppError> {
        parse_animes(&self.query_raw(QUERY, variables).await?)
    }

    /// Shikimori's GraphQL, one request at a time with a pause between them (its rate limit).
    async fn query_raw(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<String, AppError> {
        {
            let mut last = self.last_request.lock().await;
            if let Some(t) = *last {
                let elapsed = t.elapsed();
                if elapsed < MIN_INTERVAL {
                    tokio::time::sleep(MIN_INTERVAL - elapsed).await;
                }
            }
            *last = Some(Instant::now());
        }
        let body = json!({ "query": query, "variables": variables }).to_string();
        let (resp, _) = self
            .http
            .fetch_mirrored(
                &self.domains,
                "/api/graphql",
                FetchRequest::post_json("", body),
            )
            .await?;
        Ok(resp.body)
    }

    /// The title's poster as it was uploaded, larger than the one cards show: for looking at it
    /// whole.
    pub async fn poster_original(&self, id: i64) -> Result<Option<String>, AppError> {
        let body = self
            .query_raw(
                "query ($ids: String) { animes(ids: $ids) { poster { originalUrl } } }",
                json!({ "ids": id.to_string() }),
            )
            .await?;
        let v: serde_json::Value = serde_json::from_str(&body)?;
        Ok(v["data"]["animes"][0]["poster"]["originalUrl"]
            .as_str()
            .map(String::from))
    }

    /// Source, season, rating, key people and characters of a title.
    pub async fn details_raw(&self, id: i64) -> Result<details::RawDetails, AppError> {
        let body = self
            .query_raw(details::DETAILS_QUERY, json!({ "ids": id.to_string() }))
            .await?;
        details::parse_details(&body)
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Anime>, AppError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let list = self.query(json!({ "search": q, "limit": limit })).await?;
        for a in &list {
            self.store(a)?;
        }
        Ok(list)
    }

    pub async fn get(&self, id: i64) -> Result<Anime, AppError> {
        let cached = self.cached(id)?;
        if let Some((a, fetched_at)) = &cached {
            let ttl = if a.is_ongoing() {
                TTL_ONGOING
            } else {
                TTL_RELEASED
            };
            if now_ts() - fetched_at < ttl {
                return Ok(a.clone());
            }
        }
        match self
            .query(json!({ "ids": id.to_string(), "limit": 1 }))
            .await
        {
            Ok(list) => match list.into_iter().next() {
                Some(a) => {
                    self.store(&a)?;
                    Ok(a)
                }
                None => Err(AppError::new(
                    "NotFound",
                    format!("Тайтл {id} не найден в каталоге"),
                )),
            },
            // Stale data is better than nothing when Shikimori is unreachable.
            Err(e) => cached.map(|(a, _)| a).ok_or(e),
        }
    }

    pub fn store(&self, a: &Anime) -> Result<(), AppError> {
        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT INTO titles (shikimori_id, data_json, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(shikimori_id) DO UPDATE SET data_json = excluded.data_json, fetched_at = excluded.fetched_at",
            params![a.id, serde_json::to_string(a)?, now_ts()],
        )?;
        Ok(())
    }

    fn cached(&self, id: i64) -> Result<Option<(Anime, i64)>, AppError> {
        let conn = self.db.lock().unwrap();
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT data_json, fetched_at FROM titles WHERE shikimori_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(json, ts)| Ok((serde_json::from_str(&json)?, ts)))
            .transpose()
    }

    /// Titles by id: fresh ones from the cache, the rest in requests of 50 (Shikimori's limit).
    /// Titles Shikimori does not return are left out.
    pub async fn get_many(&self, ids: &[i64]) -> Result<Vec<Anime>, AppError> {
        let mut found = Vec::new();
        let mut missing = Vec::new();
        for &id in ids {
            match self.cached(id)? {
                Some((a, at)) => {
                    let ttl = if a.is_ongoing() {
                        TTL_ONGOING
                    } else {
                        TTL_RELEASED
                    };
                    if now_ts() - at < ttl {
                        found.push(a);
                    } else {
                        missing.push(id);
                    }
                }
                None => missing.push(id),
            }
        }
        for chunk in missing.chunks(50) {
            let ids: Vec<String> = chunk.iter().map(i64::to_string).collect();
            match self
                .query(json!({ "ids": ids.join(","), "limit": chunk.len() }))
                .await
            {
                Ok(list) => {
                    for a in list {
                        self.store(&a)?;
                        found.push(a);
                    }
                }
                // Keep what the cache has when Shikimori is unreachable.
                Err(e) => {
                    let stale = self.cached_many(chunk)?;
                    if stale.is_empty() && found.is_empty() {
                        return Err(e);
                    }
                    found.extend(stale);
                }
            }
        }
        Ok(found)
    }

    /// A page of the catalog.
    pub async fn browse(
        &self,
        filter: &browse::BrowseFilter,
        page: u32,
    ) -> Result<Vec<Anime>, AppError> {
        let body = self
            .query_raw(BROWSE_QUERY, browse::browse_variables(filter, page))
            .await?;
        let list = parse_animes(&body)?;
        for a in &list {
            self.store(a)?;
        }
        Ok(list)
    }

    /// Each title's relations to other anime, as (kind, id).
    pub async fn related(&self, ids: &[i64]) -> Result<details::Relations, AppError> {
        let mut out = details::Relations::new();
        for chunk in ids.chunks(50) {
            let list: Vec<String> = chunk.iter().map(|i| i.to_string()).collect();
            let body = self
                .query_raw(
                    details::RELATED_QUERY,
                    json!({ "ids": list.join(","), "limit": chunk.len() }),
                )
                .await?;
            out.extend(details::parse_related(&body)?);
        }
        Ok(out)
    }

    /// Genres, themes and audiences to filter by; they hardly change, so once per run.
    pub async fn genres(&self) -> Result<Vec<browse::Genre>, AppError> {
        if let Some(list) = self.genres.lock().await.as_ref() {
            return Ok(list.clone());
        }
        let body = self.query_raw(browse::GENRES_QUERY, json!({})).await?;
        let list = browse::parse_genres(&body)?;
        *self.genres.lock().await = Some(list.clone());
        Ok(list)
    }

    /// The most popular series of the current season, cached for six hours.
    pub async fn season_top(&self, limit: u32) -> Result<Vec<Anime>, AppError> {
        if let Some((at, list)) = self.season.lock().await.as_ref() {
            if at.elapsed() < SEASON_TTL {
                return Ok(list.clone());
            }
        }
        let (year, month, _) = calendar::civil_from_days(now_ts().div_euclid(86400));
        let body = self
            .query_raw(
                SEASON_QUERY,
                json!({ "season": season_of(year, month), "limit": limit }),
            )
            .await?;
        let list = parse_animes(&body)?;
        for a in &list {
            self.store(a)?;
        }
        *self.season.lock().await = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// The release calendar, cached for half an hour.
    pub async fn calendar(&self) -> Result<Vec<calendar::CalendarEntry>, AppError> {
        if let Some((at, list)) = self.calendar.lock().await.as_ref() {
            if at.elapsed() < CALENDAR_TTL {
                return Ok(list.clone());
            }
        }
        let req = FetchRequest {
            method: "GET".into(),
            ..Default::default()
        };
        let (resp, _) = self
            .http
            .fetch_mirrored(&self.domains, "/api/calendar", req)
            .await?;
        let list = calendar::parse_calendar(&resp.body)?;
        *self.calendar.lock().await = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// The title's reviews on Shikimori, as its reviews page lists them.
    pub async fn reviews(&self, id: i64) -> Result<Vec<extras::Review>, AppError> {
        let (html, _) = self.rest(&format!("/animes/{id}/reviews")).await?;
        Ok(extras::parse_reviews(&html))
    }

    /// A REST answer of Shikimori and the mirror that gave it, for building image addresses.
    pub async fn rest(&self, path: &str) -> Result<(String, String), AppError> {
        let req = FetchRequest {
            method: "GET".into(),
            ..Default::default()
        };
        let (resp, base) = self.http.fetch_mirrored(&self.domains, path, req).await?;
        Ok((resp.body, base))
    }

    /// A page of a user's public anime list, as Shikimori returns it.
    pub async fn user_anime_rates(
        &self,
        nickname: &str,
        page: usize,
        limit: usize,
    ) -> Result<String, AppError> {
        let nick: String = url::form_urlencoded::byte_serialize(nickname.as_bytes()).collect();
        let req = FetchRequest {
            method: "GET".into(),
            ..Default::default()
        };
        let (resp, _) = self
            .http
            .fetch_mirrored(
                &self.domains,
                &format!("/api/users/{nick}/anime_rates?limit={limit}&page={page}"),
                req,
            )
            .await?;
        Ok(resp.body)
    }

    /// Ids of the titles Shikimori finds similar to `id`, best first.
    pub async fn similar_ids(&self, id: i64) -> Result<Vec<i64>, AppError> {
        let req = FetchRequest {
            method: "GET".into(),
            ..Default::default()
        };
        let (resp, _) = self
            .http
            .fetch_mirrored(&self.domains, &format!("/api/animes/{id}/similar"), req)
            .await?;
        parse_similar_ids(&resp.body)
    }

    /// Titles by id in the order given; ids Shikimori does not know are skipped.
    pub async fn get_ordered(&self, ids: &[i64]) -> Result<Vec<Anime>, AppError> {
        let found = self.get_many(ids).await?;
        Ok(ids
            .iter()
            .filter_map(|i| found.iter().find(|a| a.id == *i).cloned())
            .collect())
    }

    /// Titles Shikimori finds similar to `id`, best first.
    pub async fn similar(&self, id: i64, limit: usize) -> Result<Vec<Anime>, AppError> {
        let ids: Vec<i64> = self
            .similar_ids(id)
            .await?
            .into_iter()
            .take(limit)
            .collect();
        self.get_ordered(&ids).await
    }

    pub fn cached_many(&self, ids: &[i64]) -> Result<Vec<Anime>, AppError> {
        let mut out = Vec::new();
        for id in ids {
            if let Some((a, _)) = self.cached(*id)? {
                out.push(a);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::sync::{Arc, Mutex};

    const FIXTURE: &str = include_str!("../../tests/fixtures/shikimori_search.json");

    #[test]
    fn the_season_follows_the_calendar() {
        assert_eq!(season_of(2026, 9), "summer_2026");
        assert_eq!(season_of(2026, 10), "fall_2026");
        assert_eq!(season_of(2026, 12), "fall_2026");
        assert_eq!(season_of(2027, 1), "winter_2027");
        assert_eq!(season_of(2026, 4), "spring_2026");
    }

    #[test]
    fn reads_the_release_calendar() {
        let entries =
            calendar::parse_calendar(include_str!("../../tests/fixtures/shikimori_calendar.json"))
                .unwrap();
        assert_eq!(entries.len(), 4, "announcements without a date are skipped");
        assert_eq!(
            entries[2],
            calendar::CalendarEntry {
                anime_id: 62513,
                episode: 13,
                at: "2026-09-30T15:00:00.000+03:00".into(),
            }
        );
    }

    /// `cargo test --lib live_similar -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_similar() {
        let c = Catalog::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            SHIKIMORI_DOMAINS.iter().map(|d| d.to_string()).collect(),
        );
        let list = c.similar(52991, 24).await.unwrap();
        println!(
            "{:?}",
            list.iter().take(5).map(|a| &a.russian).collect::<Vec<_>>()
        );
        assert_eq!(list.len(), 24);
        assert_eq!(list[0].id, 33352, "Violet Evergarden first");
        assert!(c.similar(64444, 24).await.unwrap().is_empty());
    }

    /// `cargo test --lib live_browse -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_browse() {
        let c = Catalog::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            SHIKIMORI_DOMAINS.iter().map(|d| d.to_string()).collect(),
        );
        let genres = c.genres().await.unwrap();
        assert!(genres.len() > 50, "{genres:?}");
        let drama = genres.iter().find(|g| g.name == "Драма").unwrap().id;
        let ecchi = genres.iter().find(|g| g.name == "Этти").unwrap().id;
        let filter = browse::BrowseFilter {
            genres: vec![drama],
            exclude: vec![ecchi],
            kinds: vec!["tv".into()],
            score: Some(8),
            order: Some("ranked".into()),
            ..Default::default()
        };
        let page1 = c.browse(&filter, 1).await.unwrap();
        let page2 = c.browse(&filter, 2).await.unwrap();
        assert_eq!(page1.len(), browse::PAGE_SIZE as usize);
        assert!(page1.iter().all(|a| a.genres.iter().any(|g| g == "Драма")));
        assert!(page1.iter().all(|a| !a.genres.iter().any(|g| g == "Этти")));
        assert!(page1.iter().all(|a| a.kind.as_deref() == Some("tv")));
        assert!(page1.iter().all(|a| !page2.iter().any(|b| b.id == a.id)));
        assert!(page1.iter().any(|a| !a.studio_ids.is_empty()));
        println!(
            "{} genres; top: {:?}",
            genres.len(),
            page1.iter().take(3).map(|a| &a.russian).collect::<Vec<_>>()
        );
    }

    /// `cargo test --lib live_calendar -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_calendar() {
        let c = Catalog::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            SHIKIMORI_DOMAINS.iter().map(|d| d.to_string()).collect(),
        );
        let started = Instant::now();
        let entries = c.calendar().await.unwrap();
        let ids: Vec<i64> = entries.iter().map(|e| e.anime_id).collect();
        let titles = c.get_many(&ids).await.unwrap();
        println!(
            "{} entries, {} titles in {:?}; first: {:?}",
            entries.len(),
            titles.len(),
            started.elapsed(),
            entries.first()
        );
        assert!(
            titles.len() * 10 >= entries.len() * 9,
            "most titles resolve"
        );
    }

    #[tokio::test]
    async fn titles_missing_from_the_cache_are_fetched_fifty_at_a_time() {
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = requests.clone();
        let router = axum::Router::new().route(
            "/api/graphql",
            axum::routing::post(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { r#"{"data":{"animes":[]}}"# }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let c = Catalog::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            vec![base],
        );
        let known = parse_animes(FIXTURE).unwrap().remove(0);
        c.store(&known).unwrap();

        let ids: Vec<i64> = (1..=60).chain([known.id]).collect();
        let got = c.get_many(&ids).await.unwrap();
        assert_eq!(got.iter().map(|a| a.id).collect::<Vec<_>>(), [known.id]);
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    fn catalog_with_dead_network() -> Catalog {
        let conn = db::open_in_memory().unwrap();
        Catalog::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(conn)),
            vec!["http://127.0.0.1:9".into()],
        )
    }

    #[test]
    fn similar_titles_keep_their_order_without_clips() {
        let body = r#"[{"id":33352,"kind":"tv"},{"id":5,"kind":"music"},{"id":35851,"kind":"movie"},{"id":7,"kind":"pv"}]"#;
        assert_eq!(parse_similar_ids(body).unwrap(), vec![33352, 35851]);
        assert!(parse_similar_ids("[]").unwrap().is_empty());
    }

    #[test]
    fn studios_keep_their_ids_for_the_catalog() {
        let body = r#"{"data":{"animes":[{"id":"1","name":"X","studios":[{"id":"569","name":"MAPPA"}]}]}}"#;
        let a = &parse_animes(body).unwrap()[0];
        assert_eq!(a.studios, vec!["MAPPA"]);
        assert_eq!(a.studio_ids, vec![569]);
    }

    #[test]
    fn titles_cached_before_studio_ids_still_load() {
        let old = r#"{"id":1,"name":"X","russian":null,"english":null,"japanese":null,"synonyms":[],
            "kind":"tv","status":"released","episodes":1,"episodesAired":1,"year":2020,
            "nextEpisodeAt":null,"posterUrl":null,"genres":[],"studios":["MAPPA"],"description":null}"#;
        let a: Anime = serde_json::from_str(old).unwrap();
        assert!(a.studio_ids.is_empty());
    }

    #[test]
    fn parses_graphql_response() {
        let list = parse_animes(FIXTURE).unwrap();
        assert_eq!(list.len(), 3);
        let a = &list[0];
        assert_eq!(a.id, 52991);
        assert!(a.russian.as_deref().unwrap().contains("Фрирен"));
        assert_eq!(a.year, Some(2023));
        assert_eq!(a.episodes, 28);
        assert_eq!(a.kind.as_deref(), Some("tv"));
        assert!(a.poster_url.as_deref().unwrap().starts_with("https://"));
        assert!(a.genres.iter().any(|g| g == "Драма"));
        let d = a.description.as_deref().unwrap();
        assert!(!d.contains("[character"), "bbcode must be stripped: {d}");
        assert!(d.contains("Химмеля"));
    }

    #[test]
    fn available_episodes_depend_on_status() {
        let mut a = parse_animes(FIXTURE).unwrap().remove(0);
        a.status = Some("released".into());
        a.episodes = 28;
        a.episodes_aired = 0;
        assert_eq!(a.available_episodes(), 28);
        a.status = Some("ongoing".into());
        a.episodes = 24;
        a.episodes_aired = 7;
        assert_eq!(a.available_episodes(), 7);
        a.status = Some("anons".into());
        a.episodes_aired = 0;
        assert_eq!(a.available_episodes(), 0);
    }

    #[tokio::test]
    async fn blank_search_does_not_hit_network() {
        let c = catalog_with_dead_network();
        assert!(c.search("   ", 20).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_serves_fresh_cache_without_network() {
        let c = catalog_with_dead_network();
        let a = parse_animes(FIXTURE).unwrap().remove(0);
        c.store(&a).unwrap();
        let got = c.get(52991).await.unwrap();
        assert_eq!(got.name, "Sousou no Frieren");
        assert_eq!(c.cached_many(&[52991, 1]).unwrap().len(), 1);
    }
}
