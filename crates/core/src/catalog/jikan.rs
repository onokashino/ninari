//! Jikan (an unofficial MyAnimeList API): which episodes are fillers or recaps, and their titles.
//! Shikimori ids are MyAnimeList ids, so no matching is needed. Jikan is slow, sometimes down and
//! out of reach from some networks; then MyAnimeList's own episode pages are read instead.
//! Everything here is optional and cached, and a failure just leaves episodes unmarked.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::net::HttpClient;

const BASE: &str = "https://api.jikan.moe/v4";
const MAL: &str = "https://myanimelist.net";
/// MyAnimeList's episode pages are read when Jikan cannot be; gently, one a second.
const MAL_GAP: Duration = Duration::from_secs(1);
/// After Jikan fails, MyAnimeList is asked directly for this long: a blocked Jikan takes seconds
/// to fail each time.
const JIKAN_PAUSE: Duration = Duration::from_secs(3600);
/// Episodes on one of MyAnimeList's pages.
const MAL_PAGE: usize = 100;
/// Jikan allows 3 requests a second; a little more room than that.
const GAP: Duration = Duration::from_millis(450);
/// One Piece has a dozen pages of 100; nothing needs more than this.
const MAX_PAGES: u32 = 30;
const FINISHED_TTL: i64 = 30 * 24 * 3600;
const ONGOING_TTL: i64 = 24 * 3600;
/// When Jikan stopped partway, the rest is asked for again after this long.
const RETRY: i64 = 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeMark {
    pub number: i64,
    /// MyAnimeList's title, usually English.
    pub title: Option<String>,
    pub filler: bool,
    pub recap: bool,
}

#[derive(Deserialize)]
struct Page {
    pagination: Pagination,
    data: Vec<RawEpisode>,
}

#[derive(Deserialize)]
struct Pagination {
    #[serde(default)]
    has_next_page: bool,
}

#[derive(Deserialize)]
struct RawEpisode {
    mal_id: i64,
    title: Option<String>,
    filler: Option<bool>,
    recap: Option<bool>,
}

/// One page of `/anime/{id}/episodes`: the episodes and whether another page follows.
pub fn parse_episodes(body: &str) -> Result<(Vec<EpisodeMark>, bool), AppError> {
    let page: Page = serde_json::from_str(body)
        .map_err(|_| AppError::new("Jikan", "Jikan ответил не списком серий"))?;
    let marks = page
        .data
        .into_iter()
        .map(|e| EpisodeMark {
            number: e.mal_id,
            title: e.title.filter(|t| !t.trim().is_empty()),
            filler: e.filler.unwrap_or(false),
            recap: e.recap.unwrap_or(false),
        })
        .collect();
    Ok((marks, page.pagination.has_next_page))
}

/// One of MyAnimeList's episode pages (`/anime/{id}/_/episode?offset=N`): the episodes and
/// whether another page follows. A page without the episode table is an error, not an empty
/// list, so a block page is not taken for a title without episodes.
pub fn parse_mal_page(html: &str) -> Result<(Vec<EpisodeMark>, bool), AppError> {
    use scraper::{Html, Selector};
    let sel = |s: &str| Selector::parse(s).expect("a valid selector");
    let doc = Html::parse_document(html);
    if doc.select(&sel("table.episode_list")).next().is_none() {
        return Err(AppError::new("MyAnimeList", "не страница со списком серий"));
    }
    let (number, title, kind) = (
        sel("td.episode-number"),
        sel("td.episode-title a.fw-b"),
        sel("td.episode-title span.icon-episode-type-bg"),
    );
    let text = |el: scraper::ElementRef| el.text().collect::<String>().trim().to_string();
    let marks: Vec<EpisodeMark> = doc
        .select(&sel("tr.episode-list-data"))
        .filter_map(|row| {
            let n = row.select(&number).next().map(text)?.parse().ok()?;
            let kind = row.select(&kind).next().map(text).unwrap_or_default();
            Some(EpisodeMark {
                number: n,
                title: row
                    .select(&title)
                    .next()
                    .map(text)
                    .filter(|t| !t.is_empty()),
                filler: kind.eq_ignore_ascii_case("filler"),
                recap: kind.eq_ignore_ascii_case("recap"),
            })
        })
        .collect();
    let more = marks.len() >= MAL_PAGE;
    Ok((marks, more))
}

/// Episodes the player may pass over: fillers and recaps.
pub fn skippable(marks: &[EpisodeMark]) -> BTreeSet<i64> {
    marks
        .iter()
        .filter(|m| m.filler || m.recap)
        .map(|m| m.number)
        .collect()
}

/// How marks are kept in the database.
#[derive(Serialize, Deserialize)]
struct Stored {
    marks: Vec<EpisodeMark>,
    complete: bool,
}

#[derive(Debug, Clone)]
pub struct Cached {
    pub marks: Vec<EpisodeMark>,
    /// Every page came; otherwise Jikan failed partway and the rest is asked for again soon.
    pub complete: bool,
    pub fetched_at: i64,
}

impl Cached {
    pub fn fresh(&self, ongoing: bool, now: i64) -> bool {
        let ttl = match (self.complete, ongoing || self.marks.is_empty()) {
            (false, _) => RETRY,
            // Still airing, or nothing on MyAnimeList yet: an announced title gets episodes later.
            (true, true) => ONGOING_TTL,
            (true, false) => FINISHED_TTL,
        };
        now - self.fetched_at < ttl
    }
}

pub struct Jikan {
    /// No automatic compression: Jikan's cache keys pages by Accept-Encoding, see `get`.
    client: reqwest::Client,
    base: String,
    mal: String,
    db: Db,
    last_request: tokio::sync::Mutex<Option<Instant>>,
    /// When Jikan last failed.
    jikan_failed: std::sync::Mutex<Option<Instant>>,
}

impl Jikan {
    pub fn new(http: HttpClient, db: Db) -> Arc<Self> {
        Self::with_base(http, db, BASE)
    }

    pub fn with_base(http: HttpClient, db: Db, base: &str) -> Arc<Self> {
        Self::with_bases(http, db, base, MAL)
    }

    /// Jikan at `base`, MyAnimeList at `mal`.
    pub fn with_bases(http: HttpClient, db: Db, base: &str, mal: &str) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .user_agent(http.user_agent())
            .no_gzip()
            .no_brotli()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("a plain HTTP client");
        Arc::new(Self {
            client,
            base: base.trim_end_matches('/').to_string(),
            mal: mal.trim_end_matches('/').to_string(),
            db,
            last_request: tokio::sync::Mutex::new(None),
            jikan_failed: std::sync::Mutex::new(None),
        })
    }

    /// What is cached for a title, however old.
    pub fn cached(&self, id: i64) -> Option<Cached> {
        let row: Option<(String, i64)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT marks_json, fetched_at FROM jikan_episodes WHERE mal_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (json, fetched_at) = row?;
        let stored: Stored = serde_json::from_str(&json).ok()?;
        Some(Cached {
            marks: stored.marks,
            complete: stored.complete,
            fetched_at,
        })
    }

    pub fn store(&self, id: i64, marks: &[EpisodeMark], complete: bool) {
        let json = serde_json::to_string(&Stored {
            marks: marks.to_vec(),
            complete,
        })
        .unwrap_or_default();
        let _ = self.db.lock().unwrap().execute(
            "INSERT INTO jikan_episodes (mal_id, marks_json, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(mal_id) DO UPDATE SET marks_json = excluded.marks_json, fetched_at = excluded.fetched_at",
            params![id, json, now_ts()],
        );
    }

    /// The marks of a title: from the cache while fresh, from Jikan otherwise. When Jikan fails,
    /// whatever was cached before, or nothing. `ongoing`: the title is not fully out yet.
    pub async fn marks(&self, id: i64, ongoing: bool) -> Vec<EpisodeMark> {
        let cached = self.cached(id);
        if let Some(c) = cached.as_ref().filter(|c| c.fresh(ongoing, now_ts())) {
            return c.marks.clone();
        }
        let paused = self
            .jikan_failed
            .lock()
            .unwrap()
            .is_some_and(|at| at.elapsed() < JIKAN_PAUSE);
        let jikan = if paused {
            Err(AppError::new("Jikan", "failed recently"))
        } else {
            self.fetch_all(id).await
        };
        let fetched = match jikan {
            Ok(got) => Ok(got),
            Err(e) => {
                if !paused {
                    *self.jikan_failed.lock().unwrap() = Some(Instant::now());
                }
                tracing::info!(
                    "jikan episodes of {id}: {}; reading MyAnimeList's pages",
                    e.message
                );
                self.fetch_mal(id).await
            }
        };
        match fetched {
            Ok((marks, complete)) => {
                // A shorter list than the one kept means Jikan lost pages it had before.
                if let Some(c) = cached.filter(|c| c.marks.len() > marks.len()) {
                    return c.marks;
                }
                self.store(id, &marks, complete);
                marks
            }
            Err(e) => {
                tracing::info!("episodes of {id} from MyAnimeList: {}", e.message);
                cached.map(|c| c.marks).unwrap_or_default()
            }
        }
    }

    /// Every page, or the pages up to the first that failed; `false` then says the list is cut short.
    async fn fetch_all(&self, id: i64) -> Result<(Vec<EpisodeMark>, bool), AppError> {
        let mut all = Vec::new();
        for page in 1..=MAX_PAGES {
            let body = match self
                .get(&format!("{}/anime/{id}/episodes?page={page}", self.base))
                .await
            {
                Ok(body) => body,
                Err(e) if page == 1 => return Err(e),
                Err(_) => return Ok((all, false)),
            };
            let (marks, more) = parse_episodes(&body)?;
            all.extend(marks);
            if !more {
                return Ok((all, true));
            }
        }
        Ok((all, true))
    }

    /// Every page from MyAnimeList itself, or the pages up to the first that failed.
    async fn fetch_mal(&self, id: i64) -> Result<(Vec<EpisodeMark>, bool), AppError> {
        let mut all = Vec::new();
        for page in 0..MAX_PAGES as usize {
            let url = format!(
                "{}/anime/{id}/_/episode?offset={}",
                self.mal,
                page * MAL_PAGE
            );
            let body = match self.get_mal(&url).await {
                Ok(body) => body,
                Err(e) if page == 0 => return Err(e),
                Err(_) => return Ok((all, false)),
            };
            let (marks, more) = parse_mal_page(&body)?;
            all.extend(marks);
            if !more {
                return Ok((all, true));
            }
        }
        Ok((all, true))
    }

    async fn get_mal(&self, url: &str) -> Result<String, AppError> {
        let mut last = self.last_request.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep(MAL_GAP.saturating_sub(at.elapsed())).await;
        }
        *last = Some(Instant::now());
        drop(last);
        let failed = |e: String| AppError::new("MyAnimeList", e);
        let res = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| failed(e.to_string()))?;
        if !res.status().is_success() {
            return Err(failed(format!("HTTP {}", res.status().as_u16())));
        }
        res.text().await.map_err(|e| failed(e.to_string()))
    }

    /// A request no sooner than GAP after the previous one, from any title.
    ///
    /// When MyAnimeList is out of Jikan's reach, Jikan answers only from its cache, which keeps a
    /// page in the Accept-Encoding variant it was first asked in: without the header, then as
    /// `identity`, both readable without decompressing.
    async fn get(&self, url: &str) -> Result<String, AppError> {
        let mut last_error = AppError::new("Jikan", "нет ответа");
        for encoding in [None, Some("identity")] {
            let mut last = self.last_request.lock().await;
            if let Some(at) = *last {
                tokio::time::sleep(GAP.saturating_sub(at.elapsed())).await;
            }
            *last = Some(Instant::now());
            drop(last);
            let mut req = self.client.get(url);
            if let Some(e) = encoding {
                req = req.header("Accept-Encoding", e);
            }
            match req.send().await {
                Ok(res) if res.status().is_success() => {
                    return res
                        .text()
                        .await
                        .map_err(|e| AppError::new("Jikan", e.to_string()));
                }
                Ok(res) => {
                    last_error = AppError::new("Jikan", format!("HTTP {}", res.status().as_u16()))
                }
                Err(e) => return Err(AppError::new("Jikan", e.to_string())),
            }
        }
        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"{
      "pagination": {"last_visible_page": 3, "has_next_page": true},
      "data": [
        {"mal_id": 25, "title": "Hidden Plan", "filler": false, "recap": false},
        {"mal_id": 26, "title": "Special Report: Live from the Forest of Death!", "filler": false, "recap": true},
        {"mal_id": 97, "title": "Kidnapped! Naruto's Hot Spring Adventure!", "filler": true, "recap": false},
        {"mal_id": 98, "title": null, "filler": null, "recap": null}
      ]
    }"#;

    #[test]
    fn a_page_gives_each_episode_its_marks_and_says_if_more_follow() {
        let (marks, more) = parse_episodes(PAGE).unwrap();
        assert!(more);
        assert_eq!(marks.len(), 4);
        assert_eq!(
            marks[1],
            EpisodeMark {
                number: 26,
                title: Some("Special Report: Live from the Forest of Death!".into()),
                filler: false,
                recap: true
            }
        );
        assert!(marks[2].filler);
        assert_eq!(marks[3].title, None, "an empty title is no title");
        let (_, more) =
            parse_episodes(r#"{"pagination":{"has_next_page":false},"data":[]}"#).unwrap();
        assert!(!more);
    }

    #[test]
    fn fillers_and_recaps_are_the_episodes_to_skip() {
        let (marks, _) = parse_episodes(PAGE).unwrap();
        assert_eq!(skippable(&marks).into_iter().collect::<Vec<_>>(), [26, 97]);
    }

    /// `cargo test --lib live_jikan -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn live_jikan() {
        let db = std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let jikan = Jikan::new(HttpClient::new("ninari-test"), db);
        let marks = jikan.marks(20, false).await;
        let complete = jikan.cached(20).map(|c| c.complete);
        println!(
            "naruto: {} episodes, skippable {}, complete {complete:?}",
            marks.len(),
            skippable(&marks).len()
        );
        // How many pages come depends on whether Jikan reaches MyAnimeList today.
        assert!(marks.len() >= 100);
        let skip = skippable(&marks);
        assert!(skip.contains(&26) && skip.contains(&97));
        assert!(jikan.cached(20).is_some(), "kept for next time");
    }

    #[test]
    fn what_came_before_a_failed_page_is_kept_and_asked_again_sooner() {
        let db = std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let jikan = Jikan::new(HttpClient::new("ninari-test"), db);
        let (marks, _) = parse_episodes(PAGE).unwrap();
        jikan.store(20, &marks, false);
        let c = jikan.cached(20).unwrap();
        assert_eq!((c.marks.len(), c.complete), (4, false));
        assert!(c.fresh(false, c.fetched_at + 60));
        assert!(
            !c.fresh(false, c.fetched_at + 3601),
            "an unfinished list is asked again after an hour"
        );
        jikan.store(20, &marks, true);
        let c = jikan.cached(20).unwrap();
        assert!(
            c.fresh(false, c.fetched_at + 7 * 24 * 3600),
            "a finished title's full list keeps a month"
        );
        assert!(
            !c.fresh(true, c.fetched_at + 2 * 24 * 3600),
            "an ongoing one's a day"
        );
    }

    /// Jikan's cache keeps one variant of a page per Accept-Encoding; when MyAnimeList is down only
    /// the variant it happened to keep answers.
    #[tokio::test]
    async fn a_page_jikan_cannot_fetch_is_asked_for_in_the_variant_it_kept() {
        use axum::http::{HeaderMap, StatusCode};
        let app = axum::Router::new().route(
            "/anime/20/episodes",
            axum::routing::get(|headers: HeaderMap| async move {
                match headers.get("accept-encoding").and_then(|v| v.to_str().ok()) {
                    Some("identity") => (StatusCode::OK, PAGE.replace("true}", "false}")),
                    _ => (StatusCode::GATEWAY_TIMEOUT, "{\"status\":504}".to_string()),
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db = std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let jikan = Jikan::with_base(HttpClient::new("ninari-test"), db, &base);
        let marks = jikan.marks(20, false).await;
        assert_eq!(marks.len(), 4);
    }

    /// Rows as MyAnimeList draws them: plain, a filler, a recap.
    const MAL_PAGE_HTML: &str = r#"<html><body>
      <table class="mt8 episode_list js-watch-episode-list"><tbody>
      <tr class="episode-list-data">
        <td class="episode-number nowrap" data-raw="101">101</td>
        <td class="episode-title fs12"><a href="/anime/21/x/episode/101" class="fl-l fw-b ">Showdown in a Heat Haze! Ace vs. the Gallant Scorpion!</a>
          <br><span class="di-ib">Youen no Kettou!</span></td>
      </tr>
      <tr class="episode-list-data">
        <td class="episode-number nowrap" data-raw="102">102</td>
        <td class="episode-title fs12"><span class="fl-r di-ib pr4 icon-episode-type-bg">Filler</span><a href="/anime/21/x/episode/102" class="fl-l fw-b ">Ruins and Lost Ways! Vivi, Her Friends, and the Country&#039;s Form!</a></td>
      </tr>
      <tr class="episode-list-data">
        <td class="episode-number nowrap" data-raw="103">103</td>
        <td class="episode-title fs12"><span class="fl-r di-ib pr4 icon-episode-type-bg">Recap</span><a href="/anime/21/x/episode/103" class="fl-l fw-b "></a></td>
      </tr>
      </tbody></table></body></html>"#;

    #[test]
    fn a_mal_page_gives_titles_and_marks() {
        let (marks, more) = parse_mal_page(MAL_PAGE_HTML).unwrap();
        assert!(!more, "fewer than a hundred: the last page");
        assert_eq!(
            marks[1],
            EpisodeMark {
                number: 102,
                title: Some(
                    "Ruins and Lost Ways! Vivi, Her Friends, and the Country's Form!".into()
                ),
                filler: true,
                recap: false
            }
        );
        assert_eq!(marks[0].number, 101);
        assert!(!marks[0].filler && !marks[0].recap);
        assert_eq!((marks[2].title.as_deref(), marks[2].recap), (None, true));
        assert!(parse_mal_page("<html><body>Too many requests</body></html>").is_err());
        let empty = parse_mal_page(r#"<table class="episode_list"></table>"#).unwrap();
        assert_eq!(empty, (Vec::new(), false));
    }

    #[tokio::test]
    async fn mal_pages_stand_in_when_jikan_is_out_of_reach() {
        use axum::http::StatusCode;
        let app = axum::Router::new()
            .route(
                "/anime/20/episodes",
                axum::routing::get(|| async { (StatusCode::BAD_GATEWAY, "") }),
            )
            .route(
                "/anime/20/_/episode",
                axum::routing::get(|| async { MAL_PAGE_HTML }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db = std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let jikan = Jikan::with_bases(HttpClient::new("ninari-test"), db, &base, &base);
        let marks = jikan.marks(20, false).await;
        assert_eq!(
            skippable(&marks).into_iter().collect::<Vec<_>>(),
            [102, 103]
        );
        assert!(
            jikan.cached(20).is_some_and(|c| c.complete),
            "kept for next time"
        );
    }

    #[test]
    fn an_empty_list_is_asked_again_the_next_day() {
        let db = std::sync::Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let jikan = Jikan::new(HttpClient::new("ninari-test"), db);
        // An announced title: MyAnimeList has no episodes yet.
        jikan.store(52480, &[], true);
        let c = jikan.cached(52480).unwrap();
        assert!(c.fresh(false, c.fetched_at + 3600));
        assert!(!c.fresh(false, c.fetched_at + 2 * 24 * 3600));
    }

    #[test]
    fn an_error_page_is_not_an_empty_list() {
        assert!(parse_episodes(
            r#"{"status":504,"message":"Jikan failed to connect to MyAnimeList"}"#
        )
        .is_err());
    }
}
