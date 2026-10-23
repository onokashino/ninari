//! The catalog: Shikimori's titles by genre, type, status, season, score and so on.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::AppError;

pub const PAGE_SIZE: u32 = 30;

const KINDS: [&str; 6] = ["tv", "movie", "ova", "ona", "special", "tv_special"];
const STATUSES: [&str; 3] = ["anons", "ongoing", "released"];
/// No Rx: the catalog asks Shikimori for censored results.
const RATINGS: [&str; 5] = ["g", "pg", "pg_13", "r", "r_plus"];
/// Episode length: up to 10 minutes, up to 30, longer.
const DURATIONS: [&str; 3] = ["S", "D", "F"];
const ORDERS: [&str; 5] = ["popularity", "ranked", "aired_on", "name", "random"];
/// Without a type chosen: everything but music videos, promos and commercials.
const WATCHABLE: &str = "!music,!pv,!cm";

pub const GENRES_QUERY: &str = "{ genres(entryType: Anime) { id russian kind } }";

/// What the user filters the catalog by; unknown values are dropped, not sent.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowseFilter {
    pub genres: Vec<i64>,
    pub exclude: Vec<i64>,
    pub kinds: Vec<String>,
    pub statuses: Vec<String>,
    /// "summer_2024", a year "2024" or years "2015_2020".
    pub season: Option<String>,
    /// The lowest score.
    pub score: Option<u32>,
    pub ratings: Vec<String>,
    pub durations: Vec<String>,
    pub studio: Option<i64>,
    pub order: Option<String>,
}

fn known(values: &[String], allowed: &[&str]) -> Option<String> {
    let kept: Vec<&str> = values
        .iter()
        .map(String::as_str)
        .filter(|v| allowed.contains(v))
        .collect();
    (!kept.is_empty()).then(|| kept.join(","))
}

fn valid_season(s: &str) -> bool {
    let year = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit());
    match s.split('_').collect::<Vec<_>>().as_slice() {
        [y] => year(y),
        [a, b] => (year(a) || ["winter", "spring", "summer", "fall"].contains(a)) && year(b),
        _ => false,
    }
}

/// Variables of the catalog query for one page.
pub fn browse_variables(f: &BrowseFilter, page: u32) -> Value {
    let genre: Vec<String> = f
        .genres
        .iter()
        .map(|g| g.to_string())
        .chain(f.exclude.iter().map(|g| format!("!{g}")))
        .collect();
    json!({
        "page": page.max(1),
        "limit": PAGE_SIZE,
        "order": f.order.as_deref().filter(|o| ORDERS.contains(o)).unwrap_or("popularity"),
        "kind": known(&f.kinds, &KINDS).unwrap_or_else(|| WATCHABLE.into()),
        "status": known(&f.statuses, &STATUSES),
        "season": f.season.as_deref().filter(|s| valid_season(s)),
        "score": f.score.filter(|s| (1..=9).contains(s)),
        "genre": (!genre.is_empty()).then(|| genre.join(",")),
        "studio": f.studio.map(|s| s.to_string()),
        "rating": known(&f.ratings, &RATINGS),
        "duration": known(&f.durations, &DURATIONS),
    })
}

/// A genre, theme or audience ("demographic") to filter by.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Genre {
    pub id: i64,
    pub name: String,
    pub kind: String,
}

pub fn parse_genres(body: &str) -> Result<Vec<Genre>, AppError> {
    let v: Value = serde_json::from_str(body)?;
    if let Some(errors) = v.get("errors") {
        return Err(AppError::new("Catalog", format!("shikimori: {errors}")));
    }
    Ok(v["data"]["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            Some(Genre {
                id: g["id"].as_str()?.parse().ok()?,
                name: g["russian"].as_str()?.to_string(),
                kind: g["kind"].as_str().unwrap_or("genre").to_string(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_empty_filter_lists_popular_titles_without_music_videos_and_ads() {
        assert_eq!(
            browse_variables(&BrowseFilter::default(), 1),
            json!({
                "page": 1, "limit": PAGE_SIZE, "order": "popularity", "kind": "!music,!pv,!cm",
                "status": null, "season": null, "score": null, "genre": null, "studio": null,
                "rating": null, "duration": null
            })
        );
    }

    #[test]
    fn every_filter_becomes_a_shikimori_argument() {
        let f = BrowseFilter {
            genres: vec![10, 22],
            exclude: vec![9],
            kinds: vec!["tv".into(), "movie".into()],
            statuses: vec!["ongoing".into()],
            season: Some("2015_2020".into()),
            score: Some(8),
            ratings: vec!["pg_13".into(), "r".into()],
            durations: vec!["D".into()],
            studio: Some(569),
            order: Some("ranked".into()),
        };
        assert_eq!(
            browse_variables(&f, 3),
            json!({
                "page": 3, "limit": PAGE_SIZE, "order": "ranked", "kind": "tv,movie",
                "status": "ongoing", "season": "2015_2020", "score": 8, "genre": "10,22,!9",
                "studio": "569", "rating": "pg_13,r", "duration": "D"
            })
        );
    }

    #[test]
    fn values_shikimori_would_reject_are_dropped() {
        let f = BrowseFilter {
            kinds: vec!["tv".into(), "drop table".into()],
            statuses: vec!["soon".into()],
            season: Some("summer_2024) { x".into()),
            score: Some(11),
            ratings: vec!["rx".into()],
            durations: vec!["XL".into()],
            order: Some("nope".into()),
            ..BrowseFilter::default()
        };
        let v = browse_variables(&f, 0);
        assert_eq!(v["page"], 1);
        assert_eq!(v["kind"], "tv");
        assert_eq!(v["order"], "popularity");
        for key in ["status", "season", "score", "rating", "duration"] {
            assert_eq!(v[key], json!(null), "{key}");
        }
    }

    #[test]
    fn seasons_and_year_ranges_pass() {
        for s in ["summer_2024", "2024", "2015_2020", "fall_2026"] {
            let f = BrowseFilter {
                season: Some(s.into()),
                ..BrowseFilter::default()
            };
            assert_eq!(browse_variables(&f, 1)["season"], s);
        }
    }

    #[test]
    fn genres_parse_with_their_group() {
        let body = r#"{"data":{"genres":[
            {"id":"10","russian":"Фэнтези","kind":"genre"},
            {"id":"130","russian":"Исэкай","kind":"theme"},
            {"id":"27","russian":"Сёнен","kind":"demographic"},
            {"id":"x","russian":"Сломанный","kind":"genre"}
        ]}}"#;
        assert_eq!(
            parse_genres(body).unwrap(),
            vec![
                Genre {
                    id: 10,
                    name: "Фэнтези".into(),
                    kind: "genre".into()
                },
                Genre {
                    id: 130,
                    name: "Исэкай".into(),
                    kind: "theme".into()
                },
                Genre {
                    id: 27,
                    name: "Сёнен".into(),
                    kind: "demographic".into()
                },
            ]
        );
    }
}
