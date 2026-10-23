//! Characters and the people who voice them, for their own pages.

use serde::Serialize;
use serde_json::Value;

use super::strip_bbcode;
use crate::error::AppError;

/// A person or character linked from a page: a name and a photo.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonRef {
    pub id: i64,
    pub name: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterPage {
    pub id: i64,
    pub name: String,
    /// The name as written in the original, when it differs.
    pub original: Option<String>,
    pub japanese: Option<String>,
    pub altname: Option<String>,
    pub image: Option<String>,
    pub description: Option<String>,
    /// Who voices the character; Shikimori lists the Japanese voice first.
    pub voices: Vec<PersonRef>,
    /// Titles the character is in, and whether as a main character.
    #[serde(skip)]
    pub roles: Vec<(i64, bool)>,
}

/// One role of a voice actor: a character and the title it is from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    pub character_id: i64,
    pub character: String,
    pub character_image: Option<String>,
    pub anime_id: i64,
    pub anime: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonPage {
    pub id: i64,
    pub name: String,
    pub original: Option<String>,
    pub japanese: Option<String>,
    /// "Сэйю", "Режиссёр" and so on.
    pub job: Option<String>,
    /// "1990-09-27", when the full date is known.
    pub birth: Option<String>,
    pub image: Option<String>,
    pub roles: Vec<Role>,
}

fn id_of(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The Russian name, else the original.
fn display_name(v: &Value) -> String {
    text(&v["russian"])
        .or_else(|| text(&v["name"]))
        .unwrap_or_default()
}

/// A full image address; Shikimori gives paths, and a placeholder for missing pictures.
fn image(v: &Value, base: &str) -> Option<String> {
    let path = v["image"]["original"].as_str()?;
    if path.contains("missing") {
        return None;
    }
    Some(if path.starts_with("http") {
        path.to_string()
    } else {
        format!("{}{}", base.trim_end_matches('/'), path)
    })
}

fn original_of(v: &Value) -> Option<String> {
    let name = text(&v["name"])?;
    (text(&v["russian"]).is_some_and(|r| r != name)).then_some(name)
}

pub fn parse_character(json: &str, base: &str) -> Result<CharacterPage, AppError> {
    let v: Value = serde_json::from_str(json)?;
    let id = id_of(&v["id"]).ok_or_else(|| AppError::new("Catalog", "нет id персонажа"))?;
    Ok(CharacterPage {
        id,
        name: display_name(&v),
        original: original_of(&v),
        japanese: text(&v["japanese"]),
        altname: text(&v["altname"]),
        image: image(&v, base),
        description: text(&v["description"]).map(|d| strip_bbcode(&d)),
        voices: v["seyu"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some(PersonRef {
                    id: id_of(&p["id"])?,
                    name: display_name(p),
                    image: image(p, base),
                })
            })
            .collect(),
        roles: v["animes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| Some((id_of(&a["id"])?, a["role"].as_str() == Some("Main"))))
            .collect(),
    })
}

pub fn parse_person(json: &str, base: &str) -> Result<PersonPage, AppError> {
    let v: Value = serde_json::from_str(json)?;
    let id = id_of(&v["id"]).ok_or_else(|| AppError::new("Catalog", "нет id человека"))?;
    let b = &v["birth_on"];
    let birth = match (b["year"].as_i64(), b["month"].as_i64(), b["day"].as_i64()) {
        (Some(y), Some(m), Some(d)) => Some(format!("{y:04}-{m:02}-{d:02}")),
        _ => None,
    };
    let roles = v["roles"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|r| {
            let anime = r["animes"].as_array().and_then(|a| a.first()).cloned();
            r["characters"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |c| {
                    let a = anime.as_ref()?;
                    Some(Role {
                        character_id: id_of(&c["id"])?,
                        character: display_name(&c),
                        character_image: image(&c, base),
                        anime_id: id_of(&a["id"])?,
                        anime: display_name(a),
                    })
                })
        })
        .collect();
    Ok(PersonPage {
        id,
        name: display_name(&v),
        original: original_of(&v),
        japanese: text(&v["japanese"]),
        job: text(&v["job_title"]),
        birth,
        image: image(&v, base),
        roles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://shikimori.io";

    /// `cargo test --lib live_people -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_people() {
        use crate::net::HttpClient;
        use std::sync::{Arc, Mutex};
        let catalog = super::super::Catalog::new(
            HttpClient::new("ninari-test"),
            Arc::new(Mutex::new(crate::db::open_in_memory().unwrap())),
            super::super::SHIKIMORI_DOMAINS
                .iter()
                .map(|d| d.to_string())
                .collect(),
        );
        let (body, base) = catalog.rest("/api/characters/184947").await.unwrap();
        let c = parse_character(&body, &base).unwrap();
        println!(
            "{} ({:?}), {} voices, {} titles, image {:?}",
            c.name,
            c.japanese,
            c.voices.len(),
            c.roles.len(),
            c.image
        );
        assert_eq!(c.name, "Фрирен");
        assert!(c.voices.iter().any(|v| v.id == 17215));
        let (body, base) = catalog.rest("/api/people/17215").await.unwrap();
        let p = parse_person(&body, &base).unwrap();
        println!(
            "{} {:?} {:?}, {} roles, first {:?}",
            p.name,
            p.job,
            p.birth,
            p.roles.len(),
            p.roles.first()
        );
        assert!(p.roles.iter().any(|r| r.character_id == 184947));
    }

    #[test]
    fn a_character_page_has_names_portrait_titles_and_voices() {
        let body = r#"{
            "id": "184947", "name": "Frieren", "russian": "Фрирен", "japanese": "フリーレン",
            "altname": "Frieren the Slayer",
            "description": "Эльф. Ученица [character=184950]Фламме[/character].",
            "image": {"original": "/system/characters/original/184947.jpg?1", "preview": "/p.jpg"},
            "seyu": [
                {"id": 17215, "name": "Atsumi Tanezaki", "russian": "Ацуми Танэдзаки",
                 "image": {"original": "/system/people/original/17215.jpg"}},
                {"id": 85141, "name": "Sandra Villa García", "russian": "", "image": {"original": "/assets/globals/missing_original.jpg"}}
            ],
            "animes": [
                {"id": 52991, "role": "Main"}, {"id": 56805, "role": "Supporting"}
            ]
        }"#;
        let c = parse_character(body, BASE).unwrap();
        assert_eq!(c.name, "Фрирен");
        assert_eq!(c.original.as_deref(), Some("Frieren"));
        assert_eq!(c.japanese.as_deref(), Some("フリーレン"));
        assert_eq!(
            c.image.as_deref(),
            Some("https://shikimori.io/system/characters/original/184947.jpg?1")
        );
        assert_eq!(c.description.as_deref(), Some("Эльф. Ученица Фламме."));
        assert_eq!(c.voices[0].name, "Ацуми Танэдзаки");
        assert_eq!(
            c.voices[1].name, "Sandra Villa García",
            "no Russian name: the original"
        );
        assert_eq!(c.voices[1].image, None, "a placeholder is no photo");
        assert_eq!(c.roles, vec![(52991, true), (56805, false)]);
    }

    #[test]
    fn a_person_page_lists_roles_as_character_and_title() {
        let body = r#"{
            "id": "17215", "name": "Atsumi Tanezaki", "russian": "Ацуми Танэдзаки", "japanese": "種﨑 敦美",
            "job_title": "Сэйю", "birth_on": {"day": 27, "month": 9, "year": 1990},
            "image": {"original": "/system/people/original/17215.jpg"},
            "roles": [
                {"characters": [{"id": 184947, "name": "Frieren", "russian": "Фрирен",
                                 "image": {"original": "/system/characters/original/184947.jpg"}}],
                 "animes": [{"id": 52991, "name": "Sousou no Frieren", "russian": "Провожающая в последний путь Фрирен"},
                            {"id": 59978, "name": "Sousou no Frieren 2nd Season", "russian": "Фрирен 2"}]}
            ]
        }"#;
        let p = parse_person(body, BASE).unwrap();
        assert_eq!(p.name, "Ацуми Танэдзаки");
        assert_eq!(p.job.as_deref(), Some("Сэйю"));
        assert_eq!(p.birth.as_deref(), Some("1990-09-27"));
        assert_eq!(p.roles.len(), 1);
        let r = &p.roles[0];
        assert_eq!((r.character_id, r.character.as_str()), (184947, "Фрирен"));
        assert_eq!(
            (r.anime_id, r.anime.as_str()),
            (52991, "Провожающая в последний путь Фрирен")
        );
    }
}
