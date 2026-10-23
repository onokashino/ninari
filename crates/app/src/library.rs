//! Sorting and searching the library.

use ninari_core::feeds::LibraryItem;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Recent,
    Added,
    Name,
    Year,
}

pub const SORTS: [(Sort, &str, &str); 4] = [
    (Sort::Recent, "recent", "Недавно смотрел"),
    (Sort::Added, "added", "Недавно добавленные"),
    (Sort::Name, "name", "По названию"),
    (Sort::Year, "year", "Сначала новые"),
];

const EN: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,.~QWERTYUIOP{}ASDFGHJKL:\"ZXCVBNM<>";
const RU: &str = "ёйцукенгшщзхъфывапролджэячсмитьбюЁЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭЯЧСМИТЬБЮ";

/// Text typed with the wrong layout on: "ahbhty" becomes "фрирен" and the other way round.
pub fn swap_layout(text: &str) -> String {
    text.chars()
        .map(|c| {
            if let Some(i) = EN.chars().position(|e| e == c) {
                RU.chars().nth(i).unwrap_or(c)
            } else if let Some(i) = RU.chars().position(|r| r == c) {
                EN.chars().nth(i).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// The query as typed and, when it differs, in the other layout.
pub fn query_variants(query: &str) -> Vec<String> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let swapped = swap_layout(q);
    if swapped == q {
        vec![q.to_string()]
    } else {
        vec![q.to_string(), swapped]
    }
}

fn title(item: &LibraryItem) -> String {
    let a = &item.card.anime;
    a.russian
        .clone()
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| a.name.clone())
        .to_lowercase()
}

/// Descending, with missing values last.
fn desc(a: Option<i64>, b: Option<i64>) -> std::cmp::Ordering {
    b.unwrap_or(i64::MIN).cmp(&a.unwrap_or(i64::MIN))
}

pub fn sort(items: &mut [&LibraryItem], by: Sort) {
    items.sort_by(|a, b| match by {
        Sort::Recent => desc(a.last_watched_at, b.last_watched_at)
            .then(desc(Some(a.entry.added_at), Some(b.entry.added_at))),
        Sort::Added => desc(Some(a.entry.added_at), Some(b.entry.added_at)),
        Sort::Name => title(a).cmp(&title(b)),
        Sort::Year => desc(a.card.anime.year, b.card.anime.year).then(title(a).cmp(&title(b))),
    });
}

/// Titles any of whose names contains the query, typed in either layout.
pub fn matches(item: &LibraryItem, variants: &[String]) -> bool {
    if variants.is_empty() {
        return true;
    }
    let a = &item.card.anime;
    let variants: Vec<String> = variants.iter().map(|v| v.to_lowercase()).collect();
    [&a.russian, &Some(a.name.clone()), &a.english]
        .into_iter()
        .flatten()
        .chain(a.synonyms.iter())
        .map(|n| n.to_lowercase())
        .any(|n| variants.iter().any(|v| n.contains(v.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ninari_core::catalog::Anime;
    use ninari_core::feeds::AnimeCard;
    use ninari_core::library::LibraryEntry;

    fn item(id: i64, russian: &str, added: i64, watched_at: Option<i64>, year: i64) -> LibraryItem {
        LibraryItem {
            entry: LibraryEntry {
                shikimori_id: id,
                status: "watching".into(),
                chosen_dub_key: None,
                notify: false,
                added_at: added,
                updated_at: 0,
                score: None,
                note: None,
            },
            card: AnimeCard {
                anime: Anime {
                    id,
                    name: format!("name {id}"),
                    russian: Some(russian.into()),
                    english: None,
                    japanese: None,
                    synonyms: vec![],
                    kind: None,
                    status: None,
                    episodes: 12,
                    episodes_aired: 12,
                    year: Some(year),
                    next_episode_at: None,
                    poster_url: None,
                    genres: vec![],
                    studios: vec![],
                    studio_ids: vec![],
                    description: None,
                },
                poster_path: None,
                palette: None,
            },
            watched: 0,
            last_watched_at: watched_at,
        }
    }

    fn items() -> Vec<LibraryItem> {
        vec![
            item(1, "Фрирен", 30, Some(100), 2023),
            item(2, "Атака титанов", 10, Some(300), 2013),
            item(3, "Ванпанчмен", 20, None, 2015),
        ]
    }

    fn sorted(by: Sort) -> Vec<i64> {
        let all = items();
        let mut list: Vec<&LibraryItem> = all.iter().collect();
        sort(&mut list, by);
        list.iter().map(|i| i.card.anime.id).collect()
    }

    #[test]
    fn sorts_by_each_order() {
        assert_eq!(sorted(Sort::Recent), [2, 1, 3]);
        assert_eq!(sorted(Sort::Added), [1, 3, 2]);
        assert_eq!(sorted(Sort::Name), [2, 3, 1]);
        assert_eq!(sorted(Sort::Year), [1, 3, 2]);
    }

    #[test]
    fn finds_any_name_in_either_layout() {
        let all = items();
        let found = |q: &str| -> Vec<i64> {
            let v = query_variants(q);
            all.iter()
                .filter(|i| matches(i, &v))
                .map(|i| i.card.anime.id)
                .collect()
        };
        assert_eq!(found("фри"), [1]);
        assert_eq!(found("fnfrf"), [2]);
        assert_eq!(found("NAME 3"), [3]);
        assert_eq!(found("").len(), 3);
        assert_eq!(swap_layout("ahbhty"), "фрирен");
    }
}
