//! What the user watched, added up: time, episodes, streaks, months, genres and studios.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::catalog::calendar::civil_from_days;
use crate::catalog::Anime;
use crate::library::{EpisodeProgress, LibraryEntry};

const DAY: i64 = 86_400;
/// The length of an episode whose duration the app never learned (a list imported from Shikimori).
const ESTIMATED_EPISODE: i64 = 24 * 60;

/// The day number of a moment on the user's clock; `offset_min` is minutes east of UTC.
pub fn local_day(ts: i64, offset_min: i64) -> i64 {
    (ts + offset_min * 60).div_euclid(DAY)
}

/// The current and the best run of days in a row with something watched. Until the day is over
/// the current run counts from yesterday, so an evening show does not break it in the morning.
pub fn streaks(days: &[i64], today: i64) -> (i64, i64) {
    let days: BTreeSet<i64> = days.iter().copied().collect();
    let mut best = 0;
    let mut run = 0;
    let mut prev: Option<i64> = None;
    for &d in &days {
        run = if prev == Some(d - 1) { run + 1 } else { 1 };
        best = best.max(run);
        prev = Some(d);
    }
    let current = match prev {
        Some(last) if last == today || last == today - 1 => run,
        _ => 0,
    };
    (current, best)
}

/// Things watched per month over the last twelve, oldest first, empty months included.
pub fn months(times: &[i64], now: i64, offset_min: i64) -> Vec<(String, i64)> {
    let (y, m, _) = civil_from_days(local_day(now, offset_min));
    let index = |y: i64, m: i64| y * 12 + (m - 1);
    let last = index(y, m);
    let mut counts: HashMap<i64, i64> = HashMap::new();
    for &t in times {
        let (ty, tm, _) = civil_from_days(local_day(t, offset_min));
        let i = index(ty, tm);
        if i > last - 12 && i <= last {
            *counts.entry(i).or_default() += 1;
        }
    }
    (last - 11..=last)
        .map(|i| {
            (
                format!("{}-{:02}", i / 12, i % 12 + 1),
                counts.get(&i).copied().unwrap_or(0),
            )
        })
        .collect()
}

/// The `n` most frequent values, most frequent first; ties keep the order they first appeared in.
pub fn top_counts(values: impl Iterator<Item = String>, n: usize) -> Vec<(String, i64)> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, i64> = HashMap::new();
    for v in values {
        if !counts.contains_key(&v) {
            order.push(v.clone());
        }
        *counts.entry(v).or_default() += 1;
    }
    order.sort_by_key(|v| std::cmp::Reverse(counts[v]));
    order
        .into_iter()
        .take(n)
        .map(|v| (v.clone(), counts[&v]))
        .collect()
}

const STATUSES: [&str; 5] = ["watching", "planned", "completed", "on_hold", "dropped"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub episodes: i64,
    pub seconds: i64,
    /// Some episode lengths were guessed.
    pub estimated: bool,
    pub completed: i64,
    /// Library titles per status, in a fixed order.
    pub statuses: Vec<(String, i64)>,
    pub current_streak: i64,
    pub best_streak: i64,
    /// Episodes watched per month over the last year, as ("2026-09", count).
    pub months: Vec<(String, i64)>,
    pub genres: Vec<(String, i64)>,
    pub studios: Vec<(String, i64)>,
}

/// The statistics of a library; genres and studios come from titles being watched or finished.
pub fn build(
    progress: &[EpisodeProgress],
    entries: &[LibraryEntry],
    titles: &[Anime],
    now: i64,
    offset_min: i64,
) -> Stats {
    let rows: Vec<(bool, f64, f64)> = progress
        .iter()
        .map(|p| (p.watched, p.duration_sec, p.position_sec))
        .collect();
    let (seconds, estimated) = watch_time(&rows);
    let watched: Vec<&EpisodeProgress> = progress.iter().filter(|p| p.watched).collect();
    // A row without a time (0) is no day of watching.
    let days: Vec<i64> = progress
        .iter()
        .filter(|p| p.updated_at > 1)
        .map(|p| local_day(p.updated_at, offset_min))
        .collect();
    let (current_streak, best_streak) = streaks(&days, local_day(now, offset_min));
    let times: Vec<i64> = watched
        .iter()
        .filter(|p| p.updated_at > 1)
        .map(|p| p.updated_at)
        .collect();
    let tasted: Vec<&Anime> = entries
        .iter()
        .filter(|e| e.status == "watching" || e.status == "completed")
        .filter_map(|e| titles.iter().find(|a| a.id == e.shikimori_id))
        .collect();
    Stats {
        episodes: watched.len() as i64,
        seconds,
        estimated,
        completed: entries.iter().filter(|e| e.status == "completed").count() as i64,
        statuses: STATUSES
            .iter()
            .map(|s| {
                (
                    s.to_string(),
                    entries.iter().filter(|e| e.status == *s).count() as i64,
                )
            })
            .collect(),
        current_streak,
        best_streak,
        months: months(&times, now, offset_min),
        genres: top_counts(tasted.iter().flat_map(|a| a.genres.clone()), 8),
        studios: top_counts(tasted.iter().flat_map(|a| a.studios.clone()), 6),
    }
}

/// Seconds watched from (watched, duration, position) rows, and whether any duration was guessed.
pub fn watch_time(rows: &[(bool, f64, f64)]) -> (i64, bool) {
    let mut seconds = 0;
    let mut estimated = false;
    for &(watched, duration, position) in rows {
        seconds += if !watched {
            position as i64
        } else if duration > 0.0 {
            duration as i64
        } else {
            estimated = true;
            ESTIMATED_EPISODE
        };
    }
    (seconds, estimated)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn title(id: i64, genres: &[&str], studio: &str) -> Anime {
        let genres: Vec<String> = genres
            .iter()
            .map(|g| format!(r#"{{"russian":"{g}"}}"#))
            .collect();
        let body = format!(
            r#"{{"data":{{"animes":[{{"id":"{id}","name":"x","genres":[{}],"studios":[{{"name":"{studio}"}}]}}]}}}}"#,
            genres.join(",")
        );
        crate::catalog::parse_animes(&body).unwrap().remove(0)
    }

    fn entry(id: i64, status: &str) -> LibraryEntry {
        LibraryEntry {
            shikimori_id: id,
            status: status.into(),
            chosen_dub_key: None,
            notify: true,
            added_at: 0,
            updated_at: 0,
            score: None,
            note: None,
        }
    }

    fn watched(id: i64, episode: i64, at: i64) -> EpisodeProgress {
        EpisodeProgress {
            shikimori_id: id,
            episode,
            position_sec: 1440.0,
            duration_sec: 1440.0,
            watched: true,
            dub_key: None,
            extension_id: None,
            updated_at: at,
        }
    }

    #[test]
    fn stats_add_up_the_library_and_take_tastes_from_what_was_watched() {
        let now = 20_711 * DAY;
        let entries = [
            entry(1, "completed"),
            entry(2, "watching"),
            entry(3, "planned"),
        ];
        let titles = [
            title(1, &["Драма", "Фэнтези"], "Madhouse"),
            title(2, &["Драма"], "MAPPA"),
            title(3, &["Комедия"], "Kyoto"),
        ];
        let progress = [
            watched(1, 1, now - DAY),
            watched(1, 2, now),
            watched(2, 1, now),
        ];
        let s = build(&progress, &entries, &titles, now, 0);
        assert_eq!(s.episodes, 3);
        assert_eq!(s.seconds, 3 * 1440);
        assert_eq!(s.completed, 1);
        assert_eq!(s.current_streak, 2);
        assert_eq!(
            s.genres,
            vec![("Драма".to_string(), 2), ("Фэнтези".to_string(), 1)]
        );
        assert!(
            s.studios.iter().all(|(st, _)| st != "Kyoto"),
            "a planned title is no taste yet"
        );
        assert_eq!(
            s.statuses
                .iter()
                .map(|(k, n)| (k.as_str(), *n))
                .collect::<Vec<_>>(),
            [
                ("watching", 1),
                ("planned", 1),
                ("completed", 1),
                ("on_hold", 0),
                ("dropped", 0)
            ]
        );
    }

    #[test]
    fn streaks_count_days_in_a_row_and_the_current_one_survives_until_tomorrow() {
        // Days 10, 11, 12, then 20 and 21; today is 21.
        let days = [10, 11, 12, 20, 21, 21];
        assert_eq!(streaks(&days, 21), (2, 3));
        assert_eq!(
            streaks(&days, 22),
            (2, 3),
            "nothing watched today yet: the streak still holds"
        );
        assert_eq!(streaks(&days, 23), (0, 3));
        assert_eq!(streaks(&[], 5), (0, 0));
    }

    #[test]
    fn local_days_follow_the_users_clock() {
        // 00:30 UTC is still the previous day three hours west of Greenwich.
        assert_eq!(local_day(10 * DAY + 1800, 0), 10);
        assert_eq!(local_day(10 * DAY + 1800, -180), 9);
        assert_eq!(local_day(10 * DAY - 1800, 240), 10);
    }

    #[test]
    fn months_go_back_a_year_with_empty_ones_kept() {
        // 2026-09-15 and 2026-08-01 and one from two years ago.
        let sep = 20_711 * DAY; // 2026-09-15
        let aug = 20_666 * DAY; // 2026-08-01
        let old = 19_900 * DAY;
        let m = months(&[sep, sep, aug, old], sep, 0);
        assert_eq!(m.len(), 12);
        assert_eq!(m.last().unwrap(), &("2026-09".to_string(), 2));
        assert_eq!(m[m.len() - 2], ("2026-08".to_string(), 1));
        assert_eq!(m[0].0, "2025-10");
        assert_eq!(m.iter().map(|(_, n)| n).sum::<i64>(), 3);
    }

    #[test]
    fn the_top_keeps_the_most_frequent_first() {
        let top = top_counts(
            ["Драма", "Фэнтези", "Драма", "Комедия", "Драма", "Фэнтези"]
                .iter()
                .map(|s| s.to_string()),
            2,
        );
        assert_eq!(
            top,
            vec![("Драма".to_string(), 3), ("Фэнтези".to_string(), 2)]
        );
    }

    #[test]
    fn watch_time_uses_the_known_duration_or_an_estimate() {
        let (seconds, estimated) = watch_time(&[
            (true, 1440.0, 0.0),
            (true, 0.0, 0.0),
            (false, 1400.0, 600.0),
        ]);
        assert_eq!(seconds, 1440 + 24 * 60 + 600);
        assert!(estimated);
        assert_eq!(watch_time(&[(true, 1440.0, 1440.0)]), (1440, false));
    }
}
