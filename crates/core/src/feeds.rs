//! The lists the screens show, the same in both apps: cards with their cached posters, what to
//! continue, new seasons, recommendations, the week's schedule, the library.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::catalog::calendar::Schedule;
use crate::catalog::people::{parse_character, parse_person, CharacterPage, PersonPage};
use crate::catalog::posters::{Palette, Posters};
use crate::catalog::recommend::merge_similar;
use crate::catalog::{Anime, Catalog};
use crate::error::AppError;
use crate::library::{
    continue_target, fresh_episodes, last_watched, EpisodeProgress, Library, LibraryEntry,
};
use crate::sources::Resolver;

type R<T> = Result<T, AppError>;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnimeCard {
    #[serde(flatten)]
    pub anime: Anime,
    pub poster_path: Option<String>,
    pub palette: Option<Palette>,
}

/// A title with its poster if it is already downloaded; the rest are fetched as cards show.
pub fn card(posters: &Posters, anime: Anime) -> AnimeCard {
    let cached = posters.cached(anime.id).ok().flatten();
    AnimeCard {
        poster_path: cached
            .as_ref()
            .map(|(p, _)| p.to_string_lossy().into_owned()),
        palette: cached.map(|(_, pal)| pal),
        anime,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueAt {
    pub episode: i64,
    pub position: f64,
}

pub fn continue_for(anime: &Anime, progress: &[EpisodeProgress]) -> Option<ContinueAt> {
    continue_target(progress, anime.available_episodes().max(1))
        .map(|(episode, position)| ContinueAt { episode, position })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueItem {
    pub card: AnimeCard,
    pub episode: i64,
    /// Episodes out after the last one opened (ongoing titles only).
    pub fresh: i64,
    pub position: f64,
    pub duration: f64,
    pub dub_key: Option<String>,
    pub dub_name: Option<String>,
}

/// The titles to continue, most recent first, each at the episode to play next.
pub fn continue_items(
    library: &Library,
    catalog: &Catalog,
    resolver: &Resolver,
    posters: &Posters,
) -> R<Vec<ContinueItem>> {
    let mut items = Vec::new();
    for row in library.continue_list()? {
        let Some(anime) = catalog.cached_many(&[row.shikimori_id])?.into_iter().next() else {
            continue;
        };
        let progress = library.progress(row.shikimori_id)?;
        let Some(at) = continue_for(&anime, &progress) else {
            continue;
        };
        let duration = progress
            .iter()
            .find(|p| p.episode == at.episode)
            .map(|p| p.duration_sec)
            .unwrap_or(0.0);
        let fresh = if anime.is_ongoing() {
            fresh_episodes(&progress, anime.available_episodes())
        } else {
            0
        };
        items.push(ContinueItem {
            fresh,
            dub_name: row.dub_key.as_deref().map(|k| resolver.dub_name(k)),
            dub_key: row.dub_key,
            episode: at.episode,
            position: at.position,
            duration,
            card: card(posters, anime),
        });
    }
    Ok(items)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSeasonCard {
    pub card: AnimeCard,
    /// The finished title it continues.
    pub after: String,
}

/// Sequels of finished titles that are out and not in the library yet.
pub async fn new_seasons(
    catalog: &Catalog,
    library: &Library,
    posters: &Posters,
) -> R<Vec<NewSeasonCard>> {
    let list = crate::catalog::sequels::new_seasons(catalog, library).await?;
    Ok(list
        .into_iter()
        .take(24)
        .map(|n| NewSeasonCard {
            after: n
                .after
                .russian
                .clone()
                .unwrap_or_else(|| n.after.name.clone()),
            card: card(posters, n.anime),
        })
        .collect())
}

/// How many recently watched titles the recommendations are drawn from.
const RECOMMEND_SEEDS: usize = 5;

/// "You may like": titles similar to what the user watched lately, minus the library.
pub async fn recommendations(
    library: &Library,
    catalog: &Catalog,
    posters: &Posters,
) -> R<Vec<AnimeCard>> {
    let entries = library.list(None)?;
    let owned: HashSet<i64> = entries.iter().map(|e| e.shikimori_id).collect();
    let mut seeds = Vec::new();
    for e in entries
        .iter()
        .filter(|e| e.status == "watching" || e.status == "completed")
    {
        let progress = library.progress(e.shikimori_id)?;
        seeds.push((
            last_watched(&progress).unwrap_or(e.updated_at),
            e.shikimori_id,
        ));
    }
    seeds.sort_by(|a, b| b.cmp(a));
    let mut lists = Vec::new();
    // One at a time: Shikimori limits how fast it may be asked.
    for (_, id) in seeds.into_iter().take(RECOMMEND_SEEDS) {
        match catalog.similar_ids(id).await {
            Ok(list) => lists.push(list),
            Err(e) => tracing::info!("similar titles of {id}: {e}"),
        }
    }
    let ids = merge_similar(&lists, &owned, 18);
    let list = catalog.get_ordered(&ids).await?;
    Ok(list.into_iter().map(|a| card(posters, a)).collect())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleItem {
    pub card: AnimeCard,
    pub episode: i64,
    /// Broadcast time, ISO 8601 with offset.
    pub at: String,
    /// The title's status in the user's library, if it is there.
    pub status: Option<String>,
}

/// Episodes of ongoing titles airing this week (AniList and Shikimori), with library statuses.
pub async fn schedule(
    schedule: &Schedule,
    catalog: &Catalog,
    library: &Library,
    posters: &Posters,
) -> R<Vec<ScheduleItem>> {
    let entries = schedule.entries().await?;
    let ids: Vec<i64> = entries.iter().map(|e| e.anime_id).collect();
    let animes: HashMap<i64, Anime> = catalog
        .get_many(&ids)
        .await?
        .into_iter()
        .map(|a| (a.id, a))
        .collect();
    let statuses: HashMap<i64, String> = library
        .list(None)?
        .into_iter()
        .map(|e| (e.shikimori_id, e.status))
        .collect();
    Ok(entries
        .into_iter()
        .filter_map(|e| {
            let anime = animes.get(&e.anime_id)?.clone();
            Some(ScheduleItem {
                status: statuses.get(&e.anime_id).cloned(),
                card: card(posters, anime),
                episode: e.episode,
                at: e.at,
            })
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub entry: LibraryEntry,
    pub card: AnimeCard,
    pub watched: i64,
    pub last_watched_at: Option<i64>,
}

/// The library, or the part of it with `status`; titles missing from the cache are fetched.
pub async fn library_items(
    library: &Library,
    catalog: &Catalog,
    posters: &Posters,
    status: Option<&str>,
) -> R<Vec<LibraryItem>> {
    let entries = library.list(status)?;
    let mut out = Vec::new();
    for entry in entries {
        let anime = match catalog
            .cached_many(&[entry.shikimori_id])?
            .into_iter()
            .next()
        {
            Some(a) => a,
            None => catalog.get(entry.shikimori_id).await?,
        };
        let progress = library.progress(entry.shikimori_id)?;
        out.push(LibraryItem {
            card: card(posters, anime),
            entry,
            watched: progress.iter().filter(|p| p.watched).count() as i64,
            last_watched_at: last_watched(&progress),
        });
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// Every title the episodes belong to, once.
    pub titles: Vec<AnimeCard>,
    pub episodes: Vec<EpisodeProgress>,
}

const HISTORY_LIMIT: usize = 400;

/// What was watched lately, newest first; episodes of titles not in the cache are left out.
pub fn history(library: &Library, catalog: &Catalog, posters: &Posters) -> R<History> {
    let mut episodes = library.history(HISTORY_LIMIT)?;
    let mut ids: Vec<i64> = episodes.iter().map(|p| p.shikimori_id).collect();
    ids.sort_unstable();
    ids.dedup();
    let titles: Vec<AnimeCard> = catalog
        .cached_many(&ids)?
        .into_iter()
        .map(|a| card(posters, a))
        .collect();
    episodes.retain(|p| titles.iter().any(|t| t.anime.id == p.shikimori_id));
    Ok(History { titles, episodes })
}

/// The statistics screen's numbers; `tz_offset_min` puts days and months on the user's clock.
pub fn stats(library: &Library, catalog: &Catalog, tz_offset_min: i64) -> R<crate::stats::Stats> {
    let progress = library.all_progress()?;
    let entries = library.list(None)?;
    let ids: Vec<i64> = entries.iter().map(|e| e.shikimori_id).collect();
    let titles = catalog.cached_many(&ids)?;
    Ok(crate::stats::build(
        &progress,
        &entries,
        &titles,
        crate::db::now_ts(),
        tz_offset_min,
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterTitle {
    pub card: AnimeCard,
    pub main: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterView {
    #[serde(flatten)]
    pub page: CharacterPage,
    pub titles: Vec<CharacterTitle>,
}

/// A character's page: names, portrait, description, who voices them and their titles.
pub async fn character(catalog: &Catalog, posters: &Posters, id: i64) -> R<CharacterView> {
    let (body, base) = catalog.rest(&format!("/api/characters/{id}")).await?;
    let page = parse_character(&body, &base)?;
    let ids: Vec<i64> = page.roles.iter().map(|(id, _)| *id).collect();
    let titles = catalog
        .get_ordered(&ids)
        .await?
        .into_iter()
        .map(|a| CharacterTitle {
            main: page.roles.iter().any(|(id, main)| *id == a.id && *main),
            card: card(posters, a),
        })
        .collect();
    Ok(CharacterView { page, titles })
}

/// A voice actor's or other person's page with their roles.
pub async fn person(catalog: &Catalog, id: i64) -> R<PersonPage> {
    let (body, base) = catalog.rest(&format!("/api/people/{id}")).await?;
    parse_person(&body, &base)
}
