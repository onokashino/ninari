//! More about a title for its page: source, season, rating, dates and key people from
//! Shikimori (in Russian), main characters with their Japanese voice actors from AniList.
//! Characters are matched by their Japanese name, which both catalogs keep.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex as AsyncMutex;

use super::anilist::AniList;
use super::posters::Posters;
use super::Catalog;
use crate::error::AppError;

pub const DETAILS_QUERY: &str = r#"query($ids: String) {
  animes(ids: $ids, limit: 1) {
    japanese origin season rating duration score
    airedOn { date } releasedOn { date }
    personRoles { rolesEn person { name russian } }
    characterRoles { rolesEn character { id name russian japanese poster { mainUrl } } }
    chronology { id russian name kind status episodes airedOn { date } }
    videos { name kind url playerUrl }
  }
}"#;

pub const CAST_QUERY: &str = "query ($mal: Int) { Media(idMal: $mal, type: ANIME) { characters(role: MAIN) { edges { node { name { native } } voiceActors(language: JAPANESE) { name { full } image { medium } } } } } }";

#[derive(Debug, Clone)]
pub struct PersonRole {
    pub roles: Vec<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct CharacterRole {
    /// Shikimori id, for the character's page.
    pub id: Option<i64>,
    pub main: bool,
    pub name: String,
    pub japanese: Option<String>,
    pub image_url: Option<String>,
}

/// A season, film or OVA of the same franchise.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub status: Option<String>,
    pub episodes: i64,
    pub year: Option<i64>,
    /// On the chain of sequels that makes the seasons; films, specials and side stories
    /// around it are not.
    pub main_line: bool,
}

/// Kinds that are something to watch; music videos, commercials and trailers are not.
const PART_KINDS: [&str; 6] = ["tv", "ona", "movie", "ova", "special", "tv_special"];

#[derive(Debug, Clone)]
pub struct RawDetails {
    pub japanese: Option<String>,
    pub origin: Option<String>,
    pub season: Option<String>,
    pub rating: Option<String>,
    pub duration: Option<i64>,
    pub score: Option<f64>,
    pub aired_on: Option<String>,
    pub released_on: Option<String>,
    pub persons: Vec<PersonRole>,
    pub characters: Vec<CharacterRole>,
    /// The franchise in airing order, the title itself included.
    pub parts: Vec<Part>,
    pub videos: Vec<Video>,
}

/// A trailer or a song of the title on YouTube.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    /// The YouTube video id.
    pub youtube: String,
    pub name: String,
    /// "trailer" (trailers, character trailers, ads) or "music" (openings and endings).
    pub group: String,
}

/// The id of a YouTube video from its youtu.be, embed or watch address.
fn youtube_id(url: &str) -> Option<String> {
    let rest = ["youtu.be/", "youtube.com/embed/", "youtube.com/watch?v="]
        .iter()
        .find_map(|marker| url.find(marker).map(|i| &url[i + marker.len()..]))?;
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (id.len() == 11).then_some(id)
}

/// Trailers first, then openings and endings; episode previews are spoilers and are left out,
/// as are videos off YouTube, which cannot be embedded.
pub fn parse_videos(list: &serde_json::Value) -> Vec<Video> {
    let mut trailers: Vec<Video> = Vec::new();
    let mut music: Vec<Video> = Vec::new();
    for v in list.as_array().into_iter().flatten() {
        let group = match v["kind"].as_str() {
            Some("pv" | "character_trailer" | "cm") => "trailer",
            Some("op" | "ed" | "op_ed_clip") => "music",
            _ => continue,
        };
        let Some(id) = v["playerUrl"]
            .as_str()
            .and_then(youtube_id)
            .or_else(|| v["url"].as_str().and_then(youtube_id))
        else {
            continue;
        };
        if trailers.iter().chain(music.iter()).any(|x| x.youtube == id) {
            continue;
        }
        let video = Video {
            youtube: id,
            name: text(&v["name"]).unwrap_or_default(),
            group: group.into(),
        };
        if group == "trailer" {
            trailers.push(video);
        } else {
            music.push(video);
        }
    }
    trailers.extend(music);
    trailers
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Staff {
    pub role: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Voice {
    pub name: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hero {
    pub id: Option<i64>,
    pub name: String,
    pub image_url: Option<String>,
    pub voice: Option<String>,
    pub voice_image_url: Option<String>,
}

fn text(v: &serde_json::Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(text).collect())
        .unwrap_or_default()
}

/// The first title of a Shikimori `animes` answer made with DETAILS_QUERY.
pub fn parse_details(json: &str) -> Result<RawDetails, AppError> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let a = &v["data"]["animes"][0];
    if !a.is_object() {
        return Err(AppError::new("NotFound", "Shikimori не вернул тайтл"));
    }
    let persons = a["personRoles"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    let person = &p["person"];
                    Some(PersonRole {
                        roles: strings(&p["rolesEn"]),
                        name: text(&person["russian"]).or_else(|| text(&person["name"]))?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let characters = a["characterRoles"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let ch = &c["character"];
                    Some(CharacterRole {
                        id: ch["id"]
                            .as_i64()
                            .or_else(|| ch["id"].as_str()?.parse().ok()),
                        main: strings(&c["rolesEn"]).iter().any(|r| r == "Main"),
                        name: text(&ch["russian"]).or_else(|| text(&ch["name"]))?,
                        japanese: text(&ch["japanese"]),
                        image_url: text(&ch["poster"]["mainUrl"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut parts: Vec<(Option<String>, Part)> = a["chronology"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let kind = text(&c["kind"])?;
                    if !PART_KINDS.contains(&kind.as_str()) {
                        return None;
                    }
                    let aired = text(&c["airedOn"]["date"]);
                    let id = c["id"]
                        .as_i64()
                        .or_else(|| c["id"].as_str()?.parse().ok())?;
                    let part = Part {
                        id,
                        name: text(&c["russian"]).or_else(|| text(&c["name"]))?,
                        kind,
                        status: text(&c["status"]),
                        episodes: c["episodes"].as_i64().unwrap_or(0),
                        year: aired.as_deref().and_then(|d| d.get(..4)?.parse().ok()),
                        main_line: false,
                    };
                    Some((aired, part))
                })
                .collect()
        })
        .unwrap_or_default();
    // Unannounced dates go last; ISO dates sort as text.
    parts.sort_by(|(a, pa), (b, pb)| (a.is_none(), a, pa.id).cmp(&(b.is_none(), b, pb.id)));
    Ok(RawDetails {
        parts: parts.into_iter().map(|(_, p)| p).collect(),
        japanese: text(&a["japanese"]),
        origin: text(&a["origin"]),
        season: text(&a["season"]),
        rating: text(&a["rating"]).filter(|r| r != "none"),
        duration: a["duration"].as_i64().filter(|d| *d > 0),
        score: a["score"].as_f64().filter(|s| *s > 0.0),
        aired_on: text(&a["airedOn"]["date"]),
        released_on: text(&a["releasedOn"]["date"]),
        persons,
        characters,
        videos: parse_videos(&a["videos"]),
    })
}

/// Roles worth showing on a title page, in this order, with their Russian labels.
/// Relations that continue one story. Shikimori's chronology also takes in works linked as
/// "other" or "alternative setting": a whole universe of separate donghua, parodies, ads.
const STORY_LINKS: [&str; 6] = [
    "sequel",
    "prequel",
    "side_story",
    "parent_story",
    "summary",
    "full_story",
];
const CHAIN_LINKS: [&str; 2] = ["sequel", "prequel"];
const SERIES_KINDS: [&str; 2] = ["tv", "ona"];

/// Each title's relations to other anime, as (kind, id).
pub type Relations = HashMap<i64, Vec<(String, i64)>>;

pub const RELATED_QUERY: &str = "query($ids: String, $limit: PositiveInt) { animes(ids: $ids, limit: $limit) { id related { relationKind anime { id } } } }";

pub fn parse_related(json: &str) -> Result<Relations, AppError> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    if let Some(errors) = v.get("errors") {
        return Err(AppError::new("Catalog", format!("shikimori: {errors}")));
    }
    let id_of = |x: &serde_json::Value| x.as_i64().or_else(|| x.as_str()?.parse().ok());
    Ok(v["data"]["animes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let links = a["related"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| Some((text(&r["relationKind"])?, id_of(&r["anime"]["id"])?)))
                .collect();
            Some((id_of(&a["id"])?, links))
        })
        .collect())
}

/// Titles reachable from `start` through the given kinds of relation.
fn reach(start: i64, links: &HashMap<i64, Vec<(&str, i64)>>, kinds: &[&str]) -> HashSet<i64> {
    let mut seen = HashSet::from([start]);
    let mut queue = vec![start];
    while let Some(at) = queue.pop() {
        for (kind, next) in links.get(&at).into_iter().flatten() {
            if kinds.contains(kind) && seen.insert(*next) {
                queue.push(*next);
            }
        }
    }
    seen
}

/// The parts of a franchise that belong to the story of `current`, in their order. The main
/// line is the chain of sequels with the most episodes of series, so a film opened on its own
/// still counts the seasons of the series it belongs to.
pub fn story_parts(current: i64, parts: Vec<Part>, related: &Relations) -> Vec<Part> {
    let members: HashSet<i64> = parts.iter().map(|p| p.id).collect();
    let mut links: HashMap<i64, Vec<(&str, i64)>> = HashMap::new();
    for (from, list) in related {
        for (kind, to) in list {
            if members.contains(from) && members.contains(to) {
                links.entry(*from).or_default().push((kind, *to));
                links.entry(*to).or_default().push((kind, *from));
            }
        }
    }
    let story = reach(current, &links, &STORY_LINKS);
    let series_episodes = |group: &HashSet<i64>| -> i64 {
        parts
            .iter()
            .filter(|p| group.contains(&p.id) && SERIES_KINDS.contains(&p.kind.as_str()))
            .map(|p| p.episodes.max(1))
            .sum()
    };
    let mut main: HashSet<i64> = HashSet::new();
    let mut best = (-1, false);
    let mut grouped: HashSet<i64> = HashSet::new();
    for p in parts.iter().filter(|p| story.contains(&p.id)) {
        if grouped.contains(&p.id) {
            continue;
        }
        let group = reach(p.id, &links, &CHAIN_LINKS);
        grouped.extend(&group);
        let score = (series_episodes(&group), group.contains(&current));
        if score > best {
            best = score;
            main = group;
        }
    }
    parts
        .into_iter()
        .filter(|p| story.contains(&p.id))
        .map(|mut p| {
            p.main_line = main.contains(&p.id);
            p
        })
        .collect()
}

const KEY_ROLES: [(&str, &str); 5] = [
    ("Director", "Режиссёр"),
    ("Original Creator", "Автор оригинала"),
    ("Series Composition", "Сценарий"),
    ("Character Design", "Дизайн персонажей"),
    ("Music", "Музыка"),
];

/// Director, original author and the like, at most two names each; animators are left out.
pub fn key_staff(persons: &[PersonRole]) -> Vec<Staff> {
    KEY_ROLES
        .iter()
        .filter_map(|(role, label)| {
            let names: Vec<String> = persons
                .iter()
                .filter(|p| p.roles.iter().any(|r| r == role))
                .map(|p| p.name.clone())
                .take(2)
                .collect();
            (!names.is_empty()).then(|| Staff {
                role: label.to_string(),
                names,
            })
        })
        .collect()
}

/// Japanese voice actors of the main characters, by the character's Japanese name.
pub fn parse_cast(json: &str) -> Result<HashMap<String, Voice>, AppError> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let edges = v["data"]["Media"]["characters"]["edges"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(edges
        .iter()
        .filter_map(|e| {
            let native = text(&e["node"]["name"]["native"])?;
            let actor = &e["voiceActors"][0];
            let voice = Voice {
                name: text(&actor["name"]["full"])?,
                image_url: text(&actor["image"]["medium"]),
            };
            Some((native, voice))
        })
        .collect())
}

/// Shikimori's placeholder for a character without a picture.
fn real_image(url: &Option<String>) -> Option<String> {
    url.clone().filter(|u| !u.contains("missing_"))
}

/// The main characters, each with a voice actor when AniList knows one.
pub fn main_characters(characters: &[CharacterRole], cast: &HashMap<String, Voice>) -> Vec<Hero> {
    characters
        .iter()
        .filter(|c| c.main)
        .take(8)
        .map(|c| {
            let voice = c.japanese.as_ref().and_then(|j| cast.get(j));
            Hero {
                id: c.id,
                name: c.name.clone(),
                image_url: real_image(&c.image_url),
                voice: voice.map(|v| v.name.clone()),
                voice_image_url: voice.and_then(|v| v.image_url.clone()),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeroView {
    /// Shikimori id, for the character's page.
    pub id: Option<i64>,
    pub name: String,
    /// Local file of the portrait.
    pub image: Option<String>,
    pub voice: Option<String>,
    pub voice_image: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleDetails {
    pub japanese: Option<String>,
    pub origin: Option<String>,
    pub season: Option<String>,
    pub rating: Option<String>,
    pub duration: Option<i64>,
    pub score: Option<f64>,
    pub aired_on: Option<String>,
    pub released_on: Option<String>,
    pub staff: Vec<Staff>,
    pub heroes: Vec<HeroView>,
    pub parts: Vec<Part>,
    pub videos: Vec<Video>,
}

const DETAILS_TTL: Duration = Duration::from_secs(6 * 3600);

/// Builds and keeps title details for the session.
pub struct Details {
    catalog: Arc<Catalog>,
    anilist: Arc<AniList>,
    posters: Arc<Posters>,
    cache: AsyncMutex<HashMap<i64, (Instant, TitleDetails)>>,
}

impl Details {
    pub fn new(catalog: Arc<Catalog>, anilist: Arc<AniList>, posters: Arc<Posters>) -> Self {
        Self {
            catalog,
            anilist,
            posters,
            cache: AsyncMutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, id: i64) -> Result<TitleDetails, AppError> {
        if let Some((at, d)) = self.cache.lock().await.get(&id) {
            if at.elapsed() < DETAILS_TTL {
                return Ok(d.clone());
            }
        }
        let (raw, cast) = tokio::join!(self.catalog.details_raw(id), self.anilist.main_cast(id));
        let raw = raw?;
        let heroes = main_characters(&raw.characters, &cast.unwrap_or_default());
        // Kept for later only when whole: a franchise left out by a failed request is asked again.
        let mut whole = true;
        let parts = if raw.parts.len() < 2 {
            raw.parts
        } else {
            let ids: Vec<i64> = raw.parts.iter().map(|p| p.id).collect();
            match self.catalog.related(&ids).await {
                Ok(related) => story_parts(id, raw.parts, &related),
                // Without relations the chronology may mix in other stories: show no seasons.
                Err(e) => {
                    tracing::warn!("relations of the franchise of {id}: {e}");
                    whole = false;
                    Vec::new()
                }
            }
        };
        let local = |url: Option<String>| async move {
            match url {
                Some(u) => self
                    .posters
                    .image(&u)
                    .await
                    .map(|p| p.to_string_lossy().into_owned()),
                None => None,
            }
        };
        let heroes = futures_util::future::join_all(heroes.into_iter().map(|h| async move {
            let (image, voice_image) = tokio::join!(local(h.image_url), local(h.voice_image_url));
            HeroView {
                id: h.id,
                name: h.name,
                image,
                voice: h.voice,
                voice_image,
            }
        }))
        .await;
        let details = TitleDetails {
            staff: key_staff(&raw.persons),
            heroes,
            parts,
            videos: raw.videos,
            japanese: raw.japanese,
            origin: raw.origin,
            season: raw.season,
            rating: raw.rating,
            duration: raw.duration,
            score: raw.score,
            aired_on: raw.aired_on,
            released_on: raw.released_on,
        };
        if whole {
            self.cache
                .lock()
                .await
                .insert(id, (Instant::now(), details.clone()));
        }
        Ok(details)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIKI: &str = include_str!("../../tests/fixtures/shikimori_details.json");
    const CAST: &str = include_str!("../../tests/fixtures/anilist_cast.json");

    /// `cargo test --lib live_details -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_details() {
        use crate::net::HttpClient;
        let db: crate::db::Db =
            Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let http = HttpClient::new("ninari-test");
        let catalog = Arc::new(Catalog::new(
            http.clone(),
            db.clone(),
            super::super::SHIKIMORI_DOMAINS
                .iter()
                .map(|d| d.to_string())
                .collect(),
        ));
        let anilist = AniList::new(http.clone(), db.clone());
        let dir = tempfile::tempdir().unwrap();
        let posters = Arc::new(Posters::new(
            http,
            db,
            dir.path().to_path_buf(),
            anilist.clone(),
        ));
        let d = Details::new(catalog, anilist, posters)
            .get(52991)
            .await
            .unwrap();
        println!(
            "{:?} {:?} {:?} {:?} {:?}-{:?}",
            d.origin, d.season, d.rating, d.duration, d.aired_on, d.released_on
        );
        for s in &d.staff {
            println!("  {}: {}", s.role, s.names.join(", "));
        }
        for h in &d.heroes {
            println!(
                "  {} / {:?} img={} voice_img={}",
                h.name,
                h.voice,
                h.image.is_some(),
                h.voice_image.is_some()
            );
        }
        assert!(d
            .heroes
            .iter()
            .all(|h| h.image.is_some() && h.voice.is_some()));
    }

    #[test]
    fn reads_title_details() {
        let d = parse_details(SHIKI).unwrap();
        assert_eq!(d.origin.as_deref(), Some("manga"));
        assert_eq!(d.season.as_deref(), Some("fall_2023"));
        assert_eq!(d.rating.as_deref(), Some("pg_13"));
        assert_eq!(d.duration, Some(24));
        assert_eq!(d.aired_on.as_deref(), Some("2023-09-29"));
        assert_eq!(d.released_on.as_deref(), Some("2024-03-22"));
        assert_eq!(d.japanese.as_deref(), Some("葬送のフリーレン"));
    }

    #[test]
    fn key_people_come_in_a_fixed_order_with_russian_labels() {
        let staff = key_staff(&parse_details(SHIKI).unwrap().persons);
        let roles: Vec<&str> = staff.iter().map(|s| s.role.as_str()).collect();
        assert_eq!(
            roles,
            [
                "Режиссёр",
                "Автор оригинала",
                "Сценарий",
                "Дизайн персонажей",
                "Музыка"
            ],
            "animators and producers are left out"
        );
        assert_eq!(staff[0].names, ["Кэйитиро Сайто"]);
        assert_eq!(staff[1].names, ["Канэхито Ямада"]);
    }

    #[test]
    fn main_characters_get_their_voice_actors_by_japanese_name() {
        let d = parse_details(SHIKI).unwrap();
        let heroes = main_characters(&d.characters, &parse_cast(CAST).unwrap());
        assert_eq!(heroes.len(), 3, "supporting characters are left out");
        let frieren = heroes.iter().find(|h| h.name == "Фрирен").unwrap();
        assert_eq!(frieren.voice.as_deref(), Some("Atsumi Tanezaki"));
        assert!(frieren
            .image_url
            .as_deref()
            .unwrap()
            .contains("/characters/184947/"));
        assert!(frieren
            .voice_image_url
            .as_deref()
            .unwrap()
            .starts_with("https://s4.anilist.co/"));
    }

    #[test]
    fn franchise_parts_in_airing_order_without_clips_and_ads() {
        let parts = parse_details(SHIKI).unwrap().parts;
        let got: Vec<(i64, &str)> = parts.iter().map(|p| (p.id, p.kind.as_str())).collect();
        assert_eq!(
            got,
            [(52991, "tv"), (56885, "ona"), (59978, "tv"), (63816, "tv")],
            "music videos and commercials are left out"
        );
        assert_eq!(parts[0].year, Some(2023));
        assert_eq!(parts[2].episodes, 10);
        assert_eq!(parts[3].status.as_deref(), Some("anons"));
        assert!(parts[1].name.contains("Магия"));
    }

    fn part(id: i64, kind: &str, episodes: i64) -> Part {
        Part {
            id,
            name: format!("Часть {id}"),
            kind: kind.into(),
            status: None,
            episodes,
            year: None,
            main_line: false,
        }
    }

    fn links(pairs: &[(i64, &str, i64)]) -> Relations {
        let mut out = Relations::new();
        for &(from, kind, to) in pairs {
            out.entry(from).or_default().push((kind.to_string(), to));
        }
        out
    }

    fn ids(parts: &[Part]) -> Vec<i64> {
        parts.iter().map(|p| p.id).collect()
    }

    fn main_line(parts: &[Part]) -> Vec<i64> {
        parts.iter().filter(|p| p.main_line).map(|p| p.id).collect()
    }

    // Er Gen's donghua: separate stories of one universe, linked only as "alternative setting".
    fn er_gen() -> (Vec<Part>, Relations) {
        let parts = vec![
            part(41923, "ona", 52),
            part(52519, "ona", 30),
            part(49574, "ona", 54),
            part(52180, "ona", 16),
            part(55809, "ona", 180),
            part(56215, "ona", 59),
            part(60279, "ona", 1),
            part(62340, "ona", 26),
            part(62248, "ona", 46),
            part(64398, "ona", 52),
            part(64445, "ona", 1),
            part(64444, "ona", 0),
        ];
        let related = links(&[
            (64444, "alternative_setting", 41923),
            (64444, "alternative_setting", 55809),
            (64445, "parent_story", 55809),
            (62248, "prequel", 56215),
            (64398, "prequel", 62340),
            (62340, "sequel", 64398),
            (62340, "alternative_setting", 55809),
            (60279, "parent_story", 55809),
            (56215, "prequel", 49574),
            (56215, "sequel", 62248),
            (55809, "side_story", 60279),
            (55809, "side_story", 64445),
            (55809, "alternative_setting", 52180),
            (52180, "alternative_setting", 41923),
            (49574, "prequel", 41923),
            (49574, "sequel", 56215),
            (52519, "alternative_version", 41923),
            (41923, "sequel", 49574),
            (41923, "alternative_version", 52519),
        ]);
        (parts, related)
    }

    #[test]
    fn other_stories_of_one_universe_are_not_its_seasons() {
        let (parts, related) = er_gen();
        assert_eq!(ids(&story_parts(64444, parts, &related)), [64444]);
    }

    #[test]
    fn seasons_follow_sequels_and_leave_alternative_versions_out() {
        let (parts, related) = er_gen();
        let got = story_parts(49574, parts, &related);
        assert_eq!(ids(&got), [41923, 49574, 56215, 62248]);
        assert_eq!(main_line(&got), [41923, 49574, 56215, 62248]);
    }

    #[test]
    fn side_stories_stay_but_the_longest_series_is_the_main_line() {
        let (parts, related) = er_gen();
        let got = story_parts(60279, parts, &related);
        assert_eq!(ids(&got), [55809, 60279, 64445]);
        assert_eq!(main_line(&got), [55809]);
    }

    #[test]
    fn films_in_the_sequel_chain_are_on_the_main_line_and_loose_links_drop() {
        // Demon Slayer: a film between TV seasons, a school parody linked only as "other".
        let parts = vec![
            part(38000, "tv", 26),
            part(48861, "special", 3),
            part(40456, "movie", 1),
            part(47778, "tv", 11),
        ];
        let related = links(&[
            (38000, "sequel", 40456),
            (38000, "other", 48861),
            (40456, "sequel", 47778),
            (47778, "prequel", 40456),
        ]);
        let got = story_parts(47778, parts, &related);
        assert_eq!(ids(&got), [38000, 40456, 47778]);
        assert_eq!(main_line(&got), [38000, 40456, 47778]);
    }

    /// `cargo test --lib live_story_parts -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_story_parts() {
        use crate::net::HttpClient;
        let db: crate::db::Db =
            Arc::new(std::sync::Mutex::new(crate::db::open_in_memory().unwrap()));
        let catalog = Catalog::new(
            HttpClient::new("ninari-test"),
            db,
            super::super::SHIKIMORI_DOMAINS
                .iter()
                .map(|d| d.to_string())
                .collect(),
        );
        let mut seasons = HashMap::new();
        for id in [64444, 49574, 51179, 52991, 47778] {
            let raw = catalog.details_raw(id).await.unwrap();
            let ids: Vec<i64> = raw.parts.iter().map(|p| p.id).collect();
            let related = catalog.related(&ids).await.unwrap();
            let got = story_parts(id, raw.parts, &related);
            println!("{id}: {} of {} parts", got.len(), ids.len());
            for p in &got {
                println!("   {} {} {} main={}", p.id, p.kind, p.name, p.main_line);
            }
            seasons.insert(id, got);
        }
        assert_eq!(seasons[&64444].len(), 1, "Qiu Mo stands alone");
        assert_eq!(ids(&seasons[&49574]), [41923, 49574, 56215, 62248]);
        assert_eq!(main_line(&seasons[&52991]), [52991, 59978, 63816]);
        assert!(
            seasons[&51179]
                .iter()
                .filter(|p| p.kind == "tv" && p.main_line)
                .count()
                == 6
        );
        assert!(seasons[&47778].iter().all(|p| p.kind != "special"));
    }

    #[test]
    fn videos_are_youtube_trailers_and_songs_without_episode_previews() {
        let videos = parse_videos(&serde_json::json!([
            {"name": "08", "kind": "episode_preview", "url": "https://youtu.be/zYnoUjuv7EE",
             "playerUrl": "//youtube.com/embed/zYnoUjuv7EE"},
            {"name": "PV 2", "kind": "pv", "url": "https://youtu.be/abcDEF12345",
             "playerUrl": "//youtube.com/embed/abcDEF12345"},
            {"name": "OP «Yuusha»", "kind": "op", "url": "https://www.youtube.com/watch?v=QoGM9hCxr4k",
             "playerUrl": null},
            {"name": "Трейлер на VK", "kind": "pv", "url": "https://vk.com/video-1_2",
             "playerUrl": "//vk.com/video_ext.php?oid=-1&id=2"},
            {"name": "Фрирен", "kind": "character_trailer", "url": "https://youtu.be/Xyz_-123456",
             "playerUrl": "//youtube.com/embed/Xyz_-123456"}
        ]));
        assert_eq!(
            videos,
            vec![
                Video {
                    youtube: "abcDEF12345".into(),
                    name: "PV 2".into(),
                    group: "trailer".into()
                },
                Video {
                    youtube: "Xyz_-123456".into(),
                    name: "Фрирен".into(),
                    group: "trailer".into()
                },
                Video {
                    youtube: "QoGM9hCxr4k".into(),
                    name: "OP «Yuusha»".into(),
                    group: "music".into()
                },
            ]
        );
    }

    #[test]
    fn relations_parse_from_shikimori() {
        let body = r#"{"data":{"animes":[
            {"id":"41923","related":[{"relationKind":"sequel","anime":{"id":"49574"}},{"relationKind":"adaptation","anime":null}]},
            {"id":"49574","related":[]}
        ]}}"#;
        let got = parse_related(body).unwrap();
        assert_eq!(got[&41923], vec![("sequel".to_string(), 49574)]);
        assert!(got[&49574].is_empty());
    }

    #[test]
    fn a_missing_portrait_is_no_portrait() {
        let mut d = parse_details(SHIKI).unwrap();
        d.characters[1].image_url = Some("/assets/globals/missing_original.jpg".into());
        let heroes = main_characters(&d.characters, &HashMap::new());
        assert!(heroes.iter().any(|h| h.image_url.is_none()));
        assert!(heroes.iter().all(|h| h.voice.is_none()));
    }
}
