//! The catalog's filters: genres included or excluded, chips that add and remove a value, seasons,
//! and the ready-made collections.

use ninari_core::catalog::browse::BrowseFilter;
use ninari_core::catalog::calendar::civil_from_days;
use ninari_core::catalog::season_of;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Include,
    Exclude,
}

pub fn genre_mark(f: &BrowseFilter, id: i64) -> Option<Mark> {
    if f.genres.contains(&id) {
        Some(Mark::Include)
    } else if f.exclude.contains(&id) {
        Some(Mark::Exclude)
    } else {
        None
    }
}

/// One click includes a genre, the next excludes it, the third clears it.
pub fn cycle_genre(f: &mut BrowseFilter, id: i64) {
    match genre_mark(f, id) {
        Some(Mark::Include) => {
            f.genres.retain(|&g| g != id);
            f.exclude.push(id);
        }
        Some(Mark::Exclude) => f.exclude.retain(|&g| g != id),
        None => f.genres.push(id),
    }
}

/// A chip adds its value, or takes it away when it is there.
pub fn toggle(values: &mut Vec<String>, value: &str) {
    match values.iter().position(|v| v == value) {
        Some(i) => {
            values.remove(i);
        }
        None => values.push(value.to_string()),
    }
}

const SEASONS: [(&str, &str); 4] = [
    ("winter", "Зима"),
    ("spring", "Весна"),
    ("summer", "Лето"),
    ("fall", "Осень"),
];

/// Shikimori's season of a moment: winter is January to March, and so on.
pub fn season_at(unix: i64) -> String {
    let (year, month, _) = civil_from_days(unix.div_euclid(86400));
    season_of(year, month)
}

fn season_index(name: &str) -> Option<i64> {
    SEASONS
        .iter()
        .position(|(n, _)| *n == name)
        .map(|i| i as i64)
}

/// "summer_2024" and the like, not a year or years.
pub fn is_named_season(season: &str) -> bool {
    season
        .split_once('_')
        .is_some_and(|(name, _)| season_index(name).is_some())
}

pub fn shift_season(season: &str, delta: i64) -> String {
    let (name, year) = season.split_once('_').unwrap_or((season, "0"));
    let index = year.parse::<i64>().unwrap_or(0) * 4 + season_index(name).unwrap_or(0) + delta;
    format!(
        "{}_{}",
        SEASONS[index.rem_euclid(4) as usize].0,
        index.div_euclid(4)
    )
}

/// "summer_2024", "2024" or "2015_2020" as people say it.
pub fn season_title(season: &str) -> String {
    match season.split_once('_') {
        None => format!("{season} год"),
        Some((a, b)) => match SEASONS.iter().find(|(n, _)| *n == a) {
            Some((_, name)) => format!("{name} {b}"),
            None => format!("{a}-{b}"),
        },
    }
}

/// The years a season of "2024" or "2015_2020" spans.
pub fn years(season: &str) -> Option<(i64, i64)> {
    let (a, b) = season.split_once('_').unwrap_or((season, season));
    Some((a.parse().ok()?, b.parse().ok()?))
}

pub fn years_season(a: i64, b: i64) -> String {
    if a == b {
        a.to_string()
    } else {
        format!("{}_{}", a.min(b), a.max(b))
    }
}

pub struct Preset {
    pub key: &'static str,
    pub label: String,
    pub filter: BrowseFilter,
}

/// Ready-made collections, each just a set of filters; `season` is the one on now.
pub fn presets(season: &str) -> Vec<Preset> {
    let with = |change: &dyn Fn(&mut BrowseFilter)| {
        let mut f = BrowseFilter::default();
        change(&mut f);
        f
    };
    vec![
        Preset {
            key: "popular",
            label: "Популярное".into(),
            filter: BrowseFilter::default(),
        },
        Preset {
            key: "top",
            label: "Топ за всё время".into(),
            filter: with(&|f| f.order = Some("ranked".into())),
        },
        Preset {
            key: "season",
            label: season_title(season),
            filter: with(&|f| f.season = Some(season.into())),
        },
        Preset {
            key: "ongoing",
            label: "Онгоинги".into(),
            filter: with(&|f| f.statuses = vec!["ongoing".into()]),
        },
        Preset {
            key: "anons",
            label: "Анонсы".into(),
            filter: with(&|f| f.statuses = vec!["anons".into()]),
        },
    ]
}

/// Popularity is the default order, so it is left out when comparing.
fn plain(f: &BrowseFilter) -> BrowseFilter {
    let mut f = f.clone();
    if f.order.as_deref() == Some("popularity") {
        f.order = None;
    }
    f
}

pub fn preset_of(f: &BrowseFilter, season: &str) -> Option<&'static str> {
    let f = plain(f);
    presets(season)
        .into_iter()
        .find(|p| plain(&p.filter) == f)
        .map(|p| p.key)
}

/// How many filters are set, the order aside: each genre, type and so on counts.
pub fn active_filters(f: &BrowseFilter) -> usize {
    f.genres.len()
        + f.exclude.len()
        + f.kinds.len()
        + f.statuses.len()
        + f.ratings.len()
        + f.durations.len()
        + [f.season.is_some(), f.score.is_some(), f.studio.is_some()]
            .into_iter()
            .filter(|&set| set)
            .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_genre_is_included_then_excluded_then_cleared() {
        let mut f = BrowseFilter::default();
        cycle_genre(&mut f, 10);
        assert_eq!(f.genres, [10]);
        assert_eq!(genre_mark(&f, 10), Some(Mark::Include));
        cycle_genre(&mut f, 10);
        assert!(f.genres.is_empty());
        assert_eq!(f.exclude, [10]);
        assert_eq!(genre_mark(&f, 10), Some(Mark::Exclude));
        cycle_genre(&mut f, 10);
        assert_eq!(f, BrowseFilter::default());
        assert_eq!(genre_mark(&f, 10), None);
    }

    #[test]
    fn chips_add_and_remove_their_value() {
        let mut v = Vec::new();
        toggle(&mut v, "tv");
        toggle(&mut v, "movie");
        assert_eq!(v, ["tv", "movie"]);
        toggle(&mut v, "tv");
        assert_eq!(v, ["movie"]);
    }

    #[test]
    fn seasons_around_a_date_and_their_names() {
        // 5 October 2026.
        assert_eq!(season_at(1_791_158_400), "fall_2026");
        assert_eq!(season_at(1_768_003_200), "winter_2026");
        assert_eq!(shift_season("fall_2026", -1), "summer_2026");
        assert_eq!(shift_season("winter_2026", -1), "fall_2025");
        assert_eq!(shift_season("fall_2026", 1), "winter_2027");
        assert_eq!(season_title("summer_2024"), "Лето 2024");
        assert_eq!(season_title("2024"), "2024 год");
        assert_eq!(season_title("2015_2020"), "2015-2020");
        assert!(is_named_season("summer_2024"));
        assert!(!is_named_season("2015_2020"));
        assert_eq!(years("2015_2020"), Some((2015, 2020)));
        assert_eq!(years("2024"), Some((2024, 2024)));
        assert_eq!(years("summer_2024"), None);
        assert_eq!(years_season(2020, 2015), "2015_2020");
        assert_eq!(years_season(2020, 2020), "2020");
    }

    #[test]
    fn a_collection_is_recognised_by_its_filters() {
        let labels: Vec<String> = presets("fall_2026").into_iter().map(|p| p.label).collect();
        assert_eq!(
            labels,
            [
                "Популярное",
                "Топ за всё время",
                "Осень 2026",
                "Онгоинги",
                "Анонсы"
            ]
        );
        let mut f = BrowseFilter::default();
        assert_eq!(preset_of(&f, "fall_2026"), Some("popular"));
        f.order = Some("popularity".into());
        assert_eq!(preset_of(&f, "fall_2026"), Some("popular"));
        f.order = Some("ranked".into());
        assert_eq!(preset_of(&f, "fall_2026"), Some("top"));
        f.genres = vec![10];
        assert_eq!(preset_of(&f, "fall_2026"), None);
        let season = BrowseFilter {
            season: Some("fall_2026".into()),
            ..Default::default()
        };
        assert_eq!(preset_of(&season, "fall_2026"), Some("season"));
    }

    #[test]
    fn filters_in_use_are_counted_apart_from_the_order() {
        let mut f = BrowseFilter {
            order: Some("ranked".into()),
            ..Default::default()
        };
        assert_eq!(active_filters(&f), 0);
        f.genres = vec![10];
        f.exclude = vec![9];
        f.kinds = vec!["tv".into()];
        f.score = Some(7);
        f.studio = Some(569);
        assert_eq!(active_filters(&f), 5);
    }
}
