//! Cover art from AniList: larger posters than Shikimori has, and wide banners. Titles are
//! matched by MAL id, which Shikimori ids equal. AniList allows few requests a minute, so
//! lookups made at the same moment (a grid of cards) go out as one request, and the answers,
//! misses included, are kept in the database.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{params, OptionalExtension};
use serde_json::json;
use tokio::sync::oneshot;

use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::net::{FetchRequest, HttpClient};

pub const ENDPOINT: &str = "https://graphql.anilist.co";
const BATCH: usize = 50;
/// How long lookups are collected before the request goes out.
const GATHER: Duration = Duration::from_millis(40);
/// A title AniList did not have is asked about again after a week.
const RECHECK_MISSING: i64 = 7 * 24 * 3600;

const QUERY: &str = "query ($ids: [Int]) { Page(perPage: 50) { media(idMal_in: $ids, type: ANIME) { idMal bannerImage coverImage { extraLarge color } } } }";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Art {
    pub cover: Option<String>,
    pub banner: Option<String>,
    pub color: Option<String>,
}

/// Art by MAL id from an AniList `Page.media` answer.
pub fn parse_art(json: &str) -> Result<HashMap<i64, Art>, AppError> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let media = v["data"]["Page"]["media"]
        .as_array()
        .ok_or_else(|| AppError::new("Broken", "AniList: no media in the answer"))?;
    let text = |v: &serde_json::Value| v.as_str().map(str::to_string);
    Ok(media
        .iter()
        .filter_map(|m| {
            let art = Art {
                cover: text(&m["coverImage"]["extraLarge"]),
                banner: text(&m["bannerImage"]),
                color: text(&m["coverImage"]["color"]),
            };
            Some((m["idMal"].as_i64()?, art))
        })
        .collect())
}

const AIRING_QUERY: &str = "query ($from: Int, $to: Int, $page: Int) { Page(page: $page, perPage: 50) { pageInfo { hasNextPage } airingSchedules(airingAt_greater: $from, airingAt_lesser: $to, sort: TIME) { episode airingAt media { idMal isAdult } } } }";
/// A week of airing is 200 to 300 episodes; this is a guard, not a limit reached in practice.
const AIRING_PAGES: u32 = 12;

/// One episode on AniList's airing schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct Airing {
    pub mal_id: i64,
    pub episode: i64,
    /// Unix time of the broadcast.
    pub at: i64,
}

/// A page of `airingSchedules` and whether more pages follow. Titles without a MAL id cannot
/// be opened in the app, and adult ones are not shown.
pub fn parse_airing(json: &str) -> Result<(Vec<Airing>, bool), AppError> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let page = &v["data"]["Page"];
    let list = page["airingSchedules"]
        .as_array()
        .ok_or_else(|| AppError::new("Broken", "AniList: no airing schedule in the answer"))?;
    let airing = list
        .iter()
        .filter(|s| s["media"]["isAdult"].as_bool() != Some(true))
        .filter_map(|s| {
            Some(Airing {
                mal_id: s["media"]["idMal"].as_i64()?,
                episode: s["episode"].as_i64()?,
                at: s["airingAt"].as_i64()?,
            })
        })
        .collect();
    Ok((
        airing,
        page["pageInfo"]["hasNextPage"].as_bool() == Some(true),
    ))
}

type Waiter = (i64, oneshot::Sender<Option<Art>>);

pub struct AniList {
    http: HttpClient,
    db: Db,
    endpoint: String,
    queue: Mutex<Vec<Waiter>>,
    paused_until: Mutex<Option<Instant>>,
}

impl AniList {
    pub fn new(http: HttpClient, db: Db) -> Arc<Self> {
        Self::with_endpoint(http, db, ENDPOINT)
    }

    pub fn with_endpoint(http: HttpClient, db: Db, endpoint: &str) -> Arc<Self> {
        Arc::new(Self {
            http,
            db,
            endpoint: endpoint.to_string(),
            queue: Mutex::new(Vec::new()),
            paused_until: Mutex::new(None),
        })
    }

    /// What the database knows: `Some(None)` is a recent miss, `None` means ask AniList.
    pub fn stored(&self, id: i64) -> Option<Option<Art>> {
        type Row = (Option<String>, Option<String>, Option<String>, i64);
        let conn = self.db.lock().unwrap();
        let row: Option<Row> = conn
            .query_row(
                "SELECT cover_url, banner_url, color, fetched_at FROM anilist_art WHERE mal_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .ok()
            .flatten();
        match row? {
            (None, None, _, at) if now_ts() - at > RECHECK_MISSING => None,
            (None, None, _, _) => Some(None),
            (cover, banner, color, _) => Some(Some(Art {
                cover,
                banner,
                color,
            })),
        }
    }

    fn store(&self, ids: &[i64], found: &HashMap<i64, Art>) {
        let Ok(mut conn) = self.db.lock() else { return };
        let Ok(tx) = conn.transaction() else { return };
        let now = now_ts();
        for id in ids {
            let art = found.get(id).cloned().unwrap_or_default();
            let _ = tx.execute(
                "INSERT OR REPLACE INTO anilist_art (mal_id, cover_url, banner_url, color, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, art.cover, art.banner, art.color, now],
            );
        }
        let _ = tx.commit();
    }

    fn paused(&self) -> bool {
        self.paused_until
            .lock()
            .unwrap()
            .is_some_and(|t| Instant::now() < t)
    }

    /// Art for a title, or `None` when AniList has none or cannot be asked right now.
    pub async fn art(self: &Arc<Self>, id: i64) -> Option<Art> {
        if let Some(known) = self.stored(id) {
            return known;
        }
        if self.paused() {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        let first = {
            let mut queue = self.queue.lock().unwrap();
            queue.push((id, tx));
            queue.len() == 1
        };
        if first {
            tokio::spawn(self.clone().flush_after(GATHER));
        }
        rx.await.ok().flatten()
    }

    async fn flush_after(self: Arc<Self>, delay: Duration) {
        tokio::time::sleep(delay).await;
        let waiting = std::mem::take(&mut *self.queue.lock().unwrap());
        let mut waiting = waiting.into_iter().peekable();
        while waiting.peek().is_some() {
            let mut batch: Vec<Waiter> = Vec::new();
            let mut ids: Vec<i64> = Vec::new();
            for w in waiting.by_ref() {
                if !ids.contains(&w.0) {
                    ids.push(w.0);
                }
                batch.push(w);
                if ids.len() == BATCH {
                    break;
                }
            }
            let found = if self.paused() {
                None
            } else {
                self.fetch(&ids).await
            };
            if let Some(found) = &found {
                self.store(&ids, found);
            }
            for (id, tx) in batch {
                let _ = tx.send(found.as_ref().and_then(|f| f.get(&id).cloned()));
            }
        }
    }

    /// Sends a query; the answer body, or None when AniList is down, limited or refuses.
    async fn post(&self, query: &str, variables: serde_json::Value) -> Option<String> {
        if self.paused() {
            return None;
        }
        let body = json!({ "query": query, "variables": variables }).to_string();
        let resp = match self
            .http
            .fetch(FetchRequest::post_json(&self.endpoint, body))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::info!("AniList unavailable: {e}");
                return None;
            }
        };
        if resp.status == 429 {
            let wait = resp
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                .unwrap_or(60);
            *self.paused_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(wait));
            tracing::info!("AniList rate limit, pausing for {wait} s");
            return None;
        }
        if resp.status != 200 {
            tracing::info!("AniList answered {}", resp.status);
            return None;
        }
        Some(resp.body)
    }

    async fn fetch(&self, ids: &[i64]) -> Option<HashMap<i64, Art>> {
        let body = self.post(QUERY, json!({ "ids": ids })).await?;
        parse_art(&body)
            .map_err(|e| tracing::info!("AniList answer not understood: {}", e.message))
            .ok()
    }

    /// Japanese voice actors of a title's main characters, by the character's Japanese name.
    pub async fn main_cast(&self, mal_id: i64) -> Option<HashMap<String, super::details::Voice>> {
        let body = self
            .post(super::details::CAST_QUERY, json!({ "mal": mal_id }))
            .await?;
        super::details::parse_cast(&body).ok()
    }

    /// What AniList's users recommend to those who liked a title, best rated first, as MAL ids.
    pub async fn recommended(&self, mal_id: i64) -> Option<Vec<i64>> {
        let body = self
            .post(
                "query ($mal: Int) { Media(idMal: $mal, type: ANIME) { recommendations(sort: RATING_DESC, perPage: 12) { nodes { mediaRecommendation { idMal } } } } }",
                json!({ "mal": mal_id }),
            )
            .await?;
        let v: serde_json::Value = serde_json::from_str(&body).ok()?;
        Some(
            v["data"]["Media"]["recommendations"]["nodes"]
                .as_array()?
                .iter()
                .filter_map(|n| n["mediaRecommendation"]["idMal"].as_i64())
                .collect(),
        )
    }

    /// Episodes airing between two Unix times, aired ones included; None when AniList cannot
    /// be asked right now.
    pub async fn airing(&self, from: i64, to: i64) -> Option<Vec<Airing>> {
        let mut out = Vec::new();
        for page in 1..=AIRING_PAGES {
            let body = self
                .post(
                    AIRING_QUERY,
                    json!({ "from": from, "to": to, "page": page }),
                )
                .await?;
            let (list, more) = parse_airing(&body)
                .map_err(|e| tracing::info!("AniList schedule not understood: {}", e.message))
                .ok()?;
            out.extend(list);
            if !more {
                break;
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const PAGE: &str = r##"{"data":{"Page":{"media":[
        {"idMal":52991,"bannerImage":"https://s4.anilist.co/file/anilistcdn/media/anime/banner/154587.jpg",
         "coverImage":{"extraLarge":"https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/bx154587.jpg","color":"#bbf1a1"}},
        {"idMal":21,"bannerImage":null,"coverImage":{"extraLarge":"https://s4.anilist.co/cover/bx21.jpg","color":null}},
        {"idMal":null,"bannerImage":null,"coverImage":{"extraLarge":"https://s4.anilist.co/cover/x.jpg","color":null}}
    ]}}}"##;

    #[test]
    fn reads_the_airing_schedule() {
        let (airing, more) =
            parse_airing(include_str!("../../tests/fixtures/anilist_airing.json")).unwrap();
        assert!(more);
        assert_eq!(
            airing,
            [
                Airing {
                    mal_id: 63151,
                    episode: 39,
                    at: 1790722800
                },
                Airing {
                    mal_id: 62248,
                    episode: 13,
                    at: 1790733600
                },
                Airing {
                    mal_id: 64461,
                    episode: 10,
                    at: 1790733600
                },
                Airing {
                    mal_id: 62218,
                    episode: 3,
                    at: 1790733600
                },
            ],
            "entries without a MAL id and adult titles are left out"
        );
    }

    #[tokio::test]
    async fn the_airing_schedule_is_read_page_by_page() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let router = axum::Router::new().route(
            "/",
            axum::routing::post(move |body: String| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                    let page = v["variables"]["page"].as_i64().unwrap();
                    serde_json::json!({"data": {"Page": {
                        "pageInfo": {"hasNextPage": page < 2},
                        "airingSchedules": [{"episode": page, "airingAt": 1000 + page,
                            "media": {"idMal": 10 + page, "isAdult": false}}]}}})
                    .to_string()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let got = client(&endpoint).airing(0, 5000).await.unwrap();
        assert_eq!(got.iter().map(|a| a.mal_id).collect::<Vec<_>>(), [11, 12]);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn reads_covers_and_banners_by_mal_id() {
        let art = parse_art(PAGE).unwrap();
        assert_eq!(art.len(), 2, "entries without a MAL id are dropped");
        let frieren = &art[&52991];
        assert!(frieren.cover.as_deref().unwrap().ends_with("bx154587.jpg"));
        assert!(frieren.banner.as_deref().unwrap().ends_with("154587.jpg"));
        assert_eq!(frieren.color.as_deref(), Some("#bbf1a1"));
        assert_eq!(art[&21].banner, None);
    }

    struct Fake {
        requests: Arc<AtomicUsize>,
        endpoint: String,
    }

    /// A local AniList: answers every id it is asked for, except `missing`; `limited` returns 429.
    async fn fake(missing: &'static [i64], limited: bool) -> Fake {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let router = axum::Router::new().route(
            "/",
            axum::routing::post(move |body: String| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    if limited {
                        return (
                            axum::http::StatusCode::TOO_MANY_REQUESTS,
                            [("retry-after", "60")],
                            String::new(),
                        );
                    }
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                    let media: Vec<_> = v["variables"]["ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|id| id.as_i64())
                        .filter(|id| !missing.contains(id))
                        .map(|id| {
                            serde_json::json!({"idMal": id, "bannerImage": null,
                                "coverImage": {"extraLarge": format!("https://cdn.test/{id}.jpg"), "color": null}})
                        })
                        .collect();
                    (
                        axum::http::StatusCode::OK,
                        [("content-type", "application/json")],
                        serde_json::json!({"data": {"Page": {"media": media}}}).to_string(),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Fake {
            requests,
            endpoint: format!("http://{addr}/"),
        }
    }

    fn client(endpoint: &str) -> Arc<AniList> {
        AniList::with_endpoint(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            endpoint,
        )
    }

    #[tokio::test]
    async fn lookups_made_together_share_one_request() {
        let f = fake(&[], false).await;
        let a = client(&f.endpoint);
        let got = futures_util::future::join_all((1..=5).map(|id| a.art(id))).await;
        assert!(got
            .iter()
            .all(|x| x.as_ref().is_some_and(|a| a.cover.is_some())));
        assert_eq!(f.requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            a.art(3).await.unwrap().cover.as_deref(),
            Some("https://cdn.test/3.jpg")
        );
        assert_eq!(
            f.requests.load(Ordering::SeqCst),
            1,
            "known art comes from the database"
        );
    }

    #[tokio::test]
    async fn titles_missing_on_anilist_are_not_asked_again() {
        let f = fake(&[9], false).await;
        let a = client(&f.endpoint);
        assert_eq!(a.art(9).await, None);
        assert_eq!(a.art(9).await, None);
        assert_eq!(f.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_rate_limit_pauses_lookups_without_caching_misses() {
        let f = fake(&[], true).await;
        let a = client(&f.endpoint);
        assert_eq!(a.art(3).await, None);
        assert_eq!(a.art(4).await, None);
        assert_eq!(
            f.requests.load(Ordering::SeqCst),
            1,
            "no requests while limited"
        );
        assert!(a.stored(3).is_none(), "a limited answer is not a miss");
    }
}
