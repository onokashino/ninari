//! The user's list on Shikimori: import by nickname now, two-way sync once signed in.

use serde::Serialize;

use crate::catalog::calendar::unix_of;
use crate::catalog::Catalog;
use crate::error::AppError;
use crate::library::Library;

/// Rates asked for per page; Shikimori allows up to 5000.
const PAGE: usize = 5000;

/// A title in the user's Shikimori list.
#[derive(Debug, Clone, PartialEq)]
pub struct Rate {
    pub anime_id: i64,
    pub status: &'static str,
    /// Episodes watched.
    pub episodes: i64,
    /// When the user last changed it, unix seconds; 0 when unknown.
    pub updated_at: i64,
}

/// The library status of a Shikimori one; rewatching is watching again.
pub fn status_of(shikimori: &str) -> Option<&'static str> {
    Some(match shikimori {
        "planned" => "planned",
        "watching" | "rewatching" => "watching",
        "completed" => "completed",
        "on_hold" => "on_hold",
        "dropped" => "dropped",
        _ => return None,
    })
}

/// Anime rates from `/api/users/:nickname/anime_rates`; manga and unknown statuses are skipped.
pub fn parse_rates(json: &str) -> Result<Vec<Rate>, AppError> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json)?;
    Ok(list
        .iter()
        .filter_map(|r| {
            Some(Rate {
                anime_id: r["anime"]["id"].as_i64()?,
                status: status_of(r["status"].as_str()?)?,
                episodes: r["episodes"].as_i64().unwrap_or(0).max(0),
                updated_at: r["updated_at"].as_str().and_then(unix_of).unwrap_or(0),
            })
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// Titles in the Shikimori list.
    pub total: usize,
    pub added: usize,
    /// Already in the library, left as they were.
    pub skipped: usize,
}

/// Adds the titles of a public Shikimori list that the library does not have yet.
pub async fn import_list(
    catalog: &Catalog,
    library: &Library,
    nickname: &str,
) -> Result<ImportReport, AppError> {
    let nickname = nickname.trim();
    if nickname.is_empty() {
        return Err(AppError::new("BadRequest", "укажи никнейм на Shikimori"));
    }
    let mut rates = Vec::new();
    for page in 1.. {
        let body = catalog
            .user_anime_rates(nickname, page, PAGE)
            .await
            .map_err(|e| match e.kind.as_str() {
                "NotFound" => AppError::new(
                    "NotFound",
                    format!("На Shikimori нет пользователя «{nickname}» или его список закрыт"),
                ),
                _ => e,
            })?;
        let got = parse_rates(&body)?;
        let full = serde_json::from_str::<Vec<serde_json::Value>>(&body)
            .map(|l| l.len() >= PAGE)
            .unwrap_or(false);
        rates.extend(got);
        if !full {
            break;
        }
    }
    // Titles the library shows must be in the catalog cache.
    let ids: Vec<i64> = rates.iter().map(|r| r.anime_id).collect();
    catalog.get_many(&ids).await?;
    let mut added = 0;
    for r in &rates {
        if library.import_rate(r.anime_id, r.status, r.episodes, r.updated_at)? {
            added += 1;
        }
    }
    Ok(ImportReport {
        total: rates.len(),
        added,
        skipped: rates.len() - added,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test --lib live_import -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_import() {
        use crate::net::HttpClient;
        use std::sync::{Arc, Mutex};
        let db: crate::db::Db = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
        let catalog = Catalog::new(
            HttpClient::new("ninari-test"),
            db.clone(),
            crate::catalog::SHIKIMORI_DOMAINS
                .iter()
                .map(|d| d.to_string())
                .collect(),
        );
        let library = Library::new(db);
        let report = import_list(&catalog, &library, "morr").await.unwrap();
        println!("{report:?}");
        assert!(report.total > 0 && report.added == report.total);
        let again = import_list(&catalog, &library, "morr").await.unwrap();
        assert_eq!(again.added, 0, "a second import adds nothing");
        let missing = import_list(&catalog, &library, "no-such-user-ninari-0")
            .await
            .unwrap_err();
        assert!(
            missing.message.contains("нет пользователя"),
            "{}",
            missing.message
        );
    }

    #[test]
    fn shikimori_statuses_map_to_library_ones() {
        assert_eq!(status_of("planned"), Some("planned"));
        assert_eq!(status_of("watching"), Some("watching"));
        assert_eq!(status_of("rewatching"), Some("watching"));
        assert_eq!(status_of("completed"), Some("completed"));
        assert_eq!(status_of("on_hold"), Some("on_hold"));
        assert_eq!(status_of("dropped"), Some("dropped"));
        assert_eq!(status_of("unknown"), None);
    }

    #[test]
    fn rates_parse_with_their_title_status_and_episodes() {
        let body = r#"[
            {"id": 1, "status": "completed", "episodes": 26, "score": 10, "updated_at": "2024-05-01T10:00:00.000+03:00",
             "anime": {"id": 1, "name": "Cowboy Bebop"}, "manga": null},
            {"id": 2, "status": "rewatching", "episodes": 3, "score": 0, "updated_at": "bad date",
             "anime": {"id": 5114, "name": "FMA:B"}, "manga": null},
            {"id": 3, "status": "planned", "episodes": 0, "anime": null, "manga": {"id": 7}}
        ]"#;
        let rates = parse_rates(body).unwrap();
        assert_eq!(
            rates,
            vec![
                Rate {
                    anime_id: 1,
                    status: "completed",
                    episodes: 26,
                    updated_at: 1714546800
                },
                Rate {
                    anime_id: 5114,
                    status: "watching",
                    episodes: 3,
                    updated_at: 0
                },
            ]
        );
    }
}
