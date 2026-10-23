//! The franchise's parts as season chips.

use ninari_core::catalog::details::Part;

use crate::format::plural;

#[derive(Debug, Clone, PartialEq)]
pub struct PartChip {
    pub id: i64,
    pub label: String,
    /// "2023, 28 серий", "2027, анонс".
    pub sub: String,
    /// The full title.
    pub name: String,
    /// A numbered season or one of its parts, as opposed to a film or a special.
    pub season: bool,
    pub current: bool,
}

/// The number in `chars[at..]` and where it ends.
fn number_at(chars: &[char], at: usize) -> Option<(u32, usize)> {
    let digits: String = chars
        .get(at..)?
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    Some((digits.parse().ok()?, at + digits.len()))
}

fn starts_with_at(chars: &[char], at: usize, word: &str) -> bool {
    let word: Vec<char> = word.chars().collect();
    chars.get(at..at + word.len()) == Some(&word[..])
}

/// Which part of a season aired in halves this is: "Часть 2", "Part 2", "2nd Cour". None for
/// anything else, a first part included, since that one starts the season.
pub fn cour_of(name: &str) -> Option<u32> {
    let chars: Vec<char> = name.to_lowercase().chars().collect();
    let letter_at = |i: usize| chars.get(i).is_some_and(|c| c.is_alphabetic());
    let skip_spaces = |mut i: usize| {
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        i
    };
    let mut found = None;
    // "часть 2", "part 2", "cour 2", the word not inside another one.
    for i in 0..chars.len() {
        if i > 0 && letter_at(i - 1) {
            continue;
        }
        for word in ["часть", "part", "cour"] {
            if starts_with_at(&chars, i, word)
                && let Some((n, _)) = number_at(&chars, skip_spaces(i + word.chars().count()))
            {
                found = Some(n);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    // "2nd cour", "2-я часть".
    if found.is_none() {
        for i in 0..chars.len() {
            if i > 0 && chars[i - 1].is_ascii_digit() {
                continue;
            }
            let Some((n, mut at)) = number_at(&chars, i) else {
                continue;
            };
            for suffix in ["st", "nd", "rd", "th", "-я"] {
                if starts_with_at(&chars, at, suffix) {
                    at += suffix.chars().count();
                    break;
                }
            }
            let word_at = skip_spaces(at);
            if word_at == at {
                continue;
            }
            for word in ["часть", "cour"] {
                let end = word_at + word.chars().count();
                if starts_with_at(&chars, word_at, word) && !letter_at(end) {
                    found = Some(n);
                }
            }
            if found.is_some() {
                break;
            }
        }
    }
    found.filter(|n| *n > 1)
}

fn kind_label(kind: &str) -> String {
    match kind {
        "tv" => "TV".into(),
        "ona" => "ONA".into(),
        "movie" => "Фильм".into(),
        "ova" => "OVA".into(),
        "special" | "tv_special" => "Спешл".into(),
        other => other.into(),
    }
}

/// Chips for switching between the parts of a franchise, in airing order. Parts of the
/// series' kind (TV, or ONA for donghua) on its main line are numbered as seasons, and a
/// season's second half keeps its number as "часть 2"; films, OVAs and side stories keep
/// their kind. Empty when there is nothing to switch to.
pub fn part_chips(parts: &[Part], current_id: i64) -> Vec<PartChip> {
    if parts.len() < 2 {
        return Vec::new();
    }
    let count = |kind: &str| parts.iter().filter(|p| p.kind == kind).count();
    let series = match parts.iter().find(|p| p.id == current_id) {
        Some(p) if p.kind == "tv" || p.kind == "ona" => p.kind.clone(),
        _ if count("tv") >= count("ona") => "tv".into(),
        _ => "ona".into(),
    };
    let mut season = 0;
    parts
        .iter()
        .map(|p| {
            let is_season = p.kind == series && p.main_line;
            let cour = if is_season && season > 0 {
                cour_of(&p.name)
            } else {
                None
            };
            if is_season && cour.is_none() {
                season += 1;
            }
            let label = match (is_season, cour) {
                (false, _) => kind_label(&p.kind),
                (true, Some(c)) => format!("{season} сезон, часть {c}"),
                (true, None) => format!("{season} сезон"),
            };
            let detail = if p.status.as_deref() == Some("anons") {
                Some("анонс".to_string())
            } else if p.episodes > 1 {
                Some(format!(
                    "{} {}",
                    p.episodes,
                    plural(p.episodes, "серия", "серии", "серий")
                ))
            } else {
                None
            };
            let sub = [p.year.map(|y| y.to_string()), detail]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
            PartChip {
                id: p.id,
                label,
                sub,
                name: p.name.clone(),
                season: is_season,
                current: p.id == current_id,
            }
        })
        .collect()
}

/// The season after this one once it has started airing.
pub fn next_season(parts: &[Part], current_id: i64) -> Option<(i64, String)> {
    let chips = part_chips(parts, current_id);
    let at = chips.iter().position(|c| c.current)?;
    let next = chips[at + 1..].iter().find(|c| c.season)?;
    let part = parts.iter().find(|p| p.id == next.id)?;
    (part.status.as_deref() != Some("anons")).then(|| (next.id, next.label.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: i64, kind: &str, year: i64, episodes: i64, status: &str) -> Part {
        named(id, &format!("Тайтл {id}"), year, episodes, kind, status)
    }

    fn named(id: i64, name: &str, year: i64, episodes: i64, kind: &str, status: &str) -> Part {
        Part {
            id,
            name: name.into(),
            kind: kind.into(),
            status: Some(status.into()),
            episodes,
            year: Some(year),
            main_line: true,
        }
    }

    fn frieren() -> Vec<Part> {
        vec![
            part(52991, "tv", 2023, 28, "released"),
            part(56885, "ona", 2023, 24, "released"),
            part(59978, "tv", 2026, 10, "released"),
            part(63816, "tv", 2027, 0, "anons"),
        ]
    }

    fn labels(chips: &[PartChip]) -> Vec<&str> {
        chips.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn seasons_are_numbered_among_their_kind() {
        let chips = part_chips(&frieren(), 59978);
        assert_eq!(labels(&chips), ["1 сезон", "ONA", "2 сезон", "3 сезон"]);
        assert_eq!(
            chips.iter().map(|c| c.current).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        let chips = part_chips(&frieren(), 52991);
        assert_eq!(chips[0].sub, "2023, 28 серий");
        assert_eq!(chips[3].sub, "2027, анонс");
    }

    #[test]
    fn donghua_seasons_are_onas_and_side_stories_keep_their_kind() {
        let donghua = vec![
            part(1, "ona", 2020, 52, "released"),
            part(2, "movie", 2021, 1, "released"),
            part(3, "ona", 2022, 52, "released"),
            part(4, "ova", 2023, 2, "released"),
        ];
        assert_eq!(
            labels(&part_chips(&donghua, 3)),
            ["1 сезон", "Фильм", "2 сезон", "OVA"]
        );
        let mut xian_ni = vec![
            part(55809, "ona", 2023, 180, "released"),
            part(60279, "ona", 2025, 1, "released"),
            part(64445, "ona", 2026, 1, "released"),
        ];
        xian_ni[1].main_line = false;
        xian_ni[2].main_line = false;
        assert_eq!(
            labels(&part_chips(&xian_ni, 60279)),
            ["1 сезон", "ONA", "ONA"]
        );
        assert_eq!(next_season(&xian_ni, 55809), None);
        assert!(part_chips(&[part(1, "tv", 2020, 12, "released")], 1).is_empty());
    }

    #[test]
    fn a_second_part_belongs_to_its_season() {
        let t = "Реинкарнация безработного: История о приключениях в другом мире";
        let mushoku = vec![
            named(39535, t, 2021, 11, "tv", "released"),
            named(45576, &format!("{t}. Часть 2"), 2021, 12, "tv", "released"),
            named(
                50360,
                &format!("{t}. Эрис - убийца гоблинов"),
                2022,
                1,
                "special",
                "released",
            ),
            named(
                55818,
                &format!("{t} 2 - Маг-хранитель Фитц"),
                2023,
                1,
                "tv_special",
                "released",
            ),
            named(51179, &format!("{t} 2"), 2023, 12, "tv", "released"),
            named(
                55888,
                &format!("{t} 2. Часть 2"),
                2024,
                12,
                "tv",
                "released",
            ),
            named(59193, &format!("{t} 3"), 2026, 14, "tv", "released"),
            named(65077, &format!("{t} 3. Часть 2"), 2027, 0, "tv", "anons"),
        ];
        assert_eq!(
            labels(&part_chips(&mushoku, 51179)),
            [
                "1 сезон",
                "1 сезон, часть 2",
                "Спешл",
                "Спешл",
                "2 сезон",
                "2 сезон, часть 2",
                "3 сезон",
                "3 сезон, часть 2"
            ]
        );
        assert_eq!(
            next_season(&mushoku, 51179),
            Some((55888, "2 сезон, часть 2".into()))
        );
        assert_eq!(
            next_season(&mushoku, 55888),
            Some((59193, "3 сезон".into()))
        );
        let kaguya = vec![
            named(1, "Kaguya-sama Part 2", 2020, 12, "tv", "released"),
            named(2, "Kaguya-sama 2", 2021, 12, "tv", "released"),
        ];
        assert_eq!(labels(&part_chips(&kaguya, 1)), ["1 сезон", "2 сезон"]);
    }

    #[test]
    fn parts_are_read_in_russian_romaji_and_english() {
        assert_eq!(cour_of("Атака титанов: Финал. Часть 2"), Some(2));
        assert_eq!(
            cour_of("Mushoku Tensei II: Isekai Ittara Honki Dasu Part 2"),
            Some(2)
        );
        assert_eq!(cour_of("Kusuriya no Hitorigoto 2nd Cour"), Some(2));
        assert_eq!(cour_of("Shingeki no Kyojin: The Final Season Part 1"), None);
        assert_eq!(cour_of("Реинкарнация безработного 2"), None);
        assert_eq!(cour_of("Горькая участь 2"), None);
    }

    #[test]
    fn next_season_waits_until_it_airs() {
        assert_eq!(
            next_season(&frieren(), 52991),
            Some((59978, "2 сезон".into()))
        );
        assert_eq!(next_season(&frieren(), 59978), None);
    }
}
