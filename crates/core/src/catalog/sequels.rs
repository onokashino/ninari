//! New seasons of what the user finished: sequels that are not in the library yet.

use std::collections::HashSet;

use super::details::Relations;
use super::{Anime, Catalog};
use crate::error::AppError;
use crate::library::Library;

/// (finished title, its sequel) for sequels the library does not have, each sequel once.
pub fn unwatched_sequels(
    finished: &[i64],
    library: &HashSet<i64>,
    relations: &Relations,
) -> Vec<(i64, i64)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for from in finished {
        for (kind, to) in relations.get(from).into_iter().flatten() {
            if kind == "sequel" && !library.contains(to) && seen.insert(*to) {
                out.push((*from, *to));
            }
        }
    }
    out
}

/// A sequel that is out, and the finished title it continues.
pub struct NewSeason {
    pub anime: Anime,
    pub after: Anime,
}

/// Airing ones first, then the latest.
fn freshest_first(list: &mut [NewSeason]) {
    list.sort_by_key(|n| {
        (
            n.anime.status.as_deref() != Some("ongoing"),
            std::cmp::Reverse(n.anime.year),
        )
    });
}

/// Sequels of the finished titles that have started airing and are not in the library,
/// airing ones first, then the latest.
pub async fn new_seasons(catalog: &Catalog, library: &Library) -> Result<Vec<NewSeason>, AppError> {
    let entries = library.list(None)?;
    let owned: HashSet<i64> = entries.iter().map(|e| e.shikimori_id).collect();
    let finished: Vec<i64> = entries
        .iter()
        .filter(|e| e.status == "completed")
        .map(|e| e.shikimori_id)
        .collect();
    if finished.is_empty() {
        return Ok(Vec::new());
    }
    let relations = catalog.related(&finished).await?;
    let pairs = unwatched_sequels(&finished, &owned, &relations);
    let ids: Vec<i64> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let titles = catalog.get_many(&ids).await?;
    let find = |id: i64| titles.iter().find(|a| a.id == id).cloned();
    let mut out: Vec<NewSeason> = pairs
        .into_iter()
        .filter_map(|(from, to)| {
            Some(NewSeason {
                anime: find(to)?,
                after: find(from)?,
            })
        })
        .filter(|n| matches!(n.anime.status.as_deref(), Some("ongoing" | "released")))
        .collect();
    freshest_first(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relations(pairs: &[(i64, &str, i64)]) -> Relations {
        let mut out = Relations::new();
        for &(from, kind, to) in pairs {
            out.entry(from).or_default().push((kind.to_string(), to));
        }
        out
    }

    /// `cargo test --lib live_new_seasons -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_new_seasons() {
        use crate::net::HttpClient;
        use std::sync::{Arc, Mutex};
        let db: crate::db::Db = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
        let catalog = Catalog::new(
            HttpClient::new("ninari-test"),
            db.clone(),
            super::super::SHIKIMORI_DOMAINS
                .iter()
                .map(|d| d.to_string())
                .collect(),
        );
        let library = Library::new(db);
        crate::sync::import_list(&catalog, &library, "morr")
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let seasons = new_seasons(&catalog, &library).await.unwrap();
        println!("{} new seasons in {:?}", seasons.len(), started.elapsed());
        for n in seasons.iter().take(8) {
            println!(
                "  {:?} after {:?} ({:?})",
                n.anime.russian, n.after.russian, n.anime.status
            );
        }
        assert!(seasons
            .iter()
            .all(|n| matches!(n.anime.status.as_deref(), Some("ongoing" | "released"))));
    }

    #[test]
    fn the_sequel_of_a_finished_title_is_new_unless_already_in_the_library() {
        let rel = relations(&[
            (1, "sequel", 2),
            (1, "side_story", 9),
            (3, "sequel", 4),
            (5, "prequel", 6),
        ]);
        let finished = [1, 3, 5];
        let library = HashSet::from([1, 3, 4, 5]);
        assert_eq!(unwatched_sequels(&finished, &library, &rel), vec![(1, 2)]);
    }

    fn season(id: i64, status: &str, year: i64) -> NewSeason {
        let body = format!(
            r#"{{"data":{{"animes":[{{"id":"{id}","name":"x{id}","status":"{status}","airedOn":{{"year":{year}}}}}]}}}}"#
        );
        let anime = crate::catalog::parse_animes(&body).unwrap().remove(0);
        NewSeason {
            after: anime.clone(),
            anime,
        }
    }

    #[test]
    fn airing_sequels_come_first_then_the_latest() {
        let mut list = vec![
            season(1, "released", 2015),
            season(2, "released", 2024),
            season(3, "ongoing", 2026),
        ];
        freshest_first(&mut list);
        assert_eq!(
            list.iter().map(|n| n.anime.id).collect::<Vec<_>>(),
            [3, 2, 1]
        );
    }

    #[test]
    fn a_sequel_two_finished_titles_lead_to_is_listed_once() {
        let rel = relations(&[(1, "sequel", 3), (2, "sequel", 3)]);
        let library = HashSet::from([1, 2]);
        assert_eq!(unwatched_sequels(&[1, 2], &library, &rel), vec![(1, 3)]);
    }
}
