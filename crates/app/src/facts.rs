//! The title page header's facts in one line.

use ninari_core::catalog::Anime;
use ninari_core::catalog::details::TitleDetails;

use crate::format::{kind_label, plural};

#[derive(Debug, PartialEq)]
pub enum Fact {
    /// Shikimori's score, "8,35".
    Score(String),
    Status(String, Tone),
    /// "ONA · осень 2026".
    Kind(String),
    /// "10 × 28 мин".
    Episodes(String),
    /// "16+", a small outlined badge.
    Age(String),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Tone {
    Done,
    Live,
    Soon,
    Plain,
}

/// "Вышел", "Выходит", "Анонс".
pub fn status_label(status: Option<&str>) -> Option<(&'static str, Tone)> {
    match status? {
        "released" => Some(("Вышел", Tone::Done)),
        "ongoing" => Some(("Выходит", Tone::Live)),
        "anons" => Some(("Анонс", Tone::Soon)),
        _ => None,
    }
}

/// "fall_2023" -> "Осень 2023".
pub fn season_label(season: &str) -> Option<String> {
    let (name, year) = season.split_once('_')?;
    let name = match name {
        "winter" => "Зима",
        "spring" => "Весна",
        "summer" => "Лето",
        "fall" => "Осень",
        _ => return None,
    };
    (!year.is_empty()).then(|| format!("{name} {year}"))
}

/// The age a rating allows: "13+"; none for G.
pub fn age_label(rating: &str) -> Option<&'static str> {
    match rating {
        "pg" => Some("6+"),
        "pg_13" => Some("13+"),
        "r" => Some("17+"),
        "r_plus" | "rx" => Some("18+"),
        _ => None,
    }
}

/// The rating's own name: "PG-13".
pub fn rating_label(rating: &str) -> Option<&'static str> {
    match rating {
        "g" => Some("G"),
        "pg" => Some("PG"),
        "pg_13" => Some("PG-13"),
        "r" => Some("R-17"),
        "r_plus" => Some("R+"),
        "rx" => Some("Rx"),
        _ => None,
    }
}

/// What the anime is based on: "manga" -> "Манга".
pub fn origin_label(origin: &str) -> Option<&'static str> {
    Some(match origin {
        "original" => "Оригинал",
        "manga" => "Манга",
        "web_manga" => "Веб-манга",
        "four_koma_manga" => "Ёнкома",
        "novel" => "Роман",
        "web_novel" => "Веб-роман",
        "light_novel" => "Ранобэ",
        "visual_novel" => "Визуальная новелла",
        "game" => "Игра",
        "card_game" => "Карточная игра",
        "music" => "Музыка",
        "radio" => "Радио",
        "book" => "Книга",
        "picture_book" => "Книга с картинками",
        "mixed_media" => "Разное",
        _ => return None,
    })
}

/// "2023-09-29" -> "29 сентября 2023".
pub fn long_date(iso: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ];
    let mut parts = iso.split('-').map(|p| p.parse::<usize>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    Some(format!("{d} {} {y}", MONTHS.get(m.checked_sub(1)?)?))
}

/// When a title aired: one date, "с ..." while it airs, or "from - to" once it has finished.
pub fn aired_range(
    aired: Option<&str>,
    released: Option<&str>,
    status: Option<&str>,
) -> Option<String> {
    let from = long_date(aired?)?;
    match released.filter(|r| Some(*r) != aired).and_then(long_date) {
        Some(to) => Some(format!("{from} - {to}")),
        None if status == Some("ongoing") => Some(format!("с {from}")),
        None => Some(from),
    }
}

/// The facts in reading order; the ones a title lacks are left out.
pub fn title_facts(a: &Anime, details: Option<&TitleDetails>) -> Vec<Fact> {
    let mut facts = Vec::new();
    if let Some(score) = details.and_then(|d| d.score).filter(|s| *s > 0.) {
        facts.push(Fact::Score(format!("{score:.2}").replace('.', ",")));
    }
    if let Some((label, tone)) = status_label(a.status.as_deref()) {
        facts.push(Fact::Status(label.into(), tone));
    }
    let season = details
        .and_then(|d| d.season.as_deref())
        .and_then(season_label)
        .map(|s| s.to_lowercase())
        .or_else(|| a.year.map(|y| y.to_string()));
    let kind: Vec<String> = [Some(kind_label(a.kind.as_deref())), season]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !kind.is_empty() {
        facts.push(Fact::Kind(kind.join(" · ")));
    }
    if let Some(e) = episodes_fact(a, details.and_then(|d| d.duration)) {
        facts.push(Fact::Episodes(e));
    }
    if let Some(age) = details
        .and_then(|d| d.rating.as_deref())
        .and_then(age_label)
    {
        facts.push(Fact::Age(age.into()));
    }
    facts
}

/// "10 × 28 мин", "7 из 12 × 24 мин" while airing, "110 мин" for a single episode.
fn episodes_fact(a: &Anime, duration: Option<i64>) -> Option<String> {
    let duration = duration.filter(|d| *d > 0);
    let length = duration.map(|d| format!(" × {d} мин")).unwrap_or_default();
    if a.status.as_deref() == Some("ongoing") {
        let total = if a.episodes > 0 {
            a.episodes.to_string()
        } else {
            "?".into()
        };
        return Some(format!("{} из {total}{length}", a.episodes_aired));
    }
    match (a.episodes, duration) {
        (1, Some(d)) => Some(format!("{d} мин")),
        (n, Some(_)) if n > 0 => Some(format!("{n}{length}")),
        (n, None) if n > 0 => Some(format!("{n} {}", plural(n, "серия", "серии", "серий"))),
        _ => None,
    }
}

/// "ещё 25 мин" for the continue button; nothing when the length is unknown.
pub fn time_left(position: f64, duration: f64) -> Option<String> {
    if duration <= 0. || position <= 0. {
        return None;
    }
    let minutes = ((duration - position) / 60.).floor();
    Some(if minutes < 1. {
        "меньше минуты".into()
    } else {
        format!("ещё {minutes} мин")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anime(status: &str, episodes: i64, aired: i64) -> Anime {
        Anime {
            id: 1,
            name: "x".into(),
            russian: None,
            english: None,
            japanese: None,
            synonyms: vec![],
            kind: Some("ona".into()),
            status: Some(status.into()),
            episodes,
            episodes_aired: aired,
            year: Some(2026),
            next_episode_at: None,
            poster_url: None,
            genres: vec![],
            studios: vec![],
            studio_ids: vec![],
            score: None,
            description: None,
        }
    }

    fn details(score: f64, season: &str, rating: &str, duration: i64) -> TitleDetails {
        TitleDetails {
            japanese: None,
            origin: None,
            season: Some(season.into()),
            rating: Some(rating.into()),
            duration: Some(duration),
            score: Some(score),
            aired_on: None,
            released_on: None,
            staff: vec![],
            heroes: vec![],
            parts: vec![],
            videos: vec![],
            viewers: Default::default(),
            shots: vec![],
        }
    }

    #[test]
    fn facts_read_as_one_line() {
        let d = details(8.351, "fall_2026", "pg_13", 28);
        assert_eq!(
            title_facts(&anime("released", 10, 10), Some(&d)),
            vec![
                Fact::Score("8,35".into()),
                Fact::Status("Вышел".into(), Tone::Done),
                Fact::Kind("ONA · осень 2026".into()),
                Fact::Episodes("10 × 28 мин".into()),
                Fact::Age("13+".into()),
            ]
        );
        assert_eq!(
            episodes_fact(&anime("ongoing", 12, 7), Some(24)).as_deref(),
            Some("7 из 12 × 24 мин")
        );
        assert_eq!(
            episodes_fact(&anime("released", 1, 1), Some(110)).as_deref(),
            Some("110 мин")
        );
        assert_eq!(
            episodes_fact(&anime("released", 5, 5), None).as_deref(),
            Some("5 серий")
        );
        assert_eq!(title_facts(&anime("released", 0, 0), None).len(), 2);
    }

    #[test]
    fn aired_ranges_read_as_dates() {
        assert_eq!(
            aired_range(Some("2023-09-29"), Some("2024-03-22"), Some("released")).as_deref(),
            Some("29 сентября 2023 - 22 марта 2024")
        );
        assert_eq!(
            aired_range(Some("2026-10-03"), None, Some("ongoing")).as_deref(),
            Some("с 3 октября 2026")
        );
        assert_eq!(
            aired_range(Some("2020-01-10"), Some("2020-01-10"), Some("released")).as_deref(),
            Some("10 января 2020")
        );
        assert_eq!(aired_range(None, None, None), None);
    }

    #[test]
    fn time_left_says_ещё() {
        assert_eq!(time_left(100., 1600.).as_deref(), Some("ещё 25 мин"));
        assert_eq!(time_left(1590., 1600.).as_deref(), Some("меньше минуты"));
        assert_eq!(time_left(0., 1600.), None);
    }
}
