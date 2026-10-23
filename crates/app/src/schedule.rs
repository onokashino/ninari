//! The week's schedule by day and the history by day, on the user's clock.

use crate::clock::{long_day_label, short_day_label, year_of};

pub struct Day<'a, T> {
    /// The local day number.
    pub day: i64,
    pub label: String,
    pub items: Vec<&'a T>,
}

/// The next seven days from today, each with its episodes by time. `at` is an item's Unix time
/// and `local` turns a Unix time into a local day number.
pub fn schedule_days<'a, T>(
    items: &'a [T],
    at: impl Fn(&T) -> Option<i64>,
    now: i64,
    local: impl Fn(i64) -> i64,
) -> Vec<Day<'a, T>> {
    let today = local(now);
    let mut days: Vec<Day<'a, T>> = (0..7)
        .map(|i| Day {
            day: today + i,
            label: match i {
                0 => "Сегодня".into(),
                1 => "Завтра".into(),
                _ => short_day_label(today + i),
            },
            items: Vec::new(),
        })
        .collect();
    let mut sorted: Vec<(&T, i64)> = items
        .iter()
        .filter_map(|it| at(it).map(|t| (it, t)))
        .collect();
    sorted.sort_by_key(|(_, t)| *t);
    for (it, t) in sorted {
        if let Some(d) = days.iter_mut().find(|d| d.day == local(t)) {
            d.items.push(it);
        }
    }
    days
}

/// "Серия 13 из 46", the last one as the finale, or "Серия 5" when the total is unknown.
pub fn episode_line(episode: i64, total: i64) -> String {
    if total <= 0 {
        format!("Серия {episode}")
    } else if episode == total {
        format!("Серия {episode} из {total}, финал")
    } else {
        format!("Серия {episode} из {total}")
    }
}

/// Episodes, newest first, split into local days: "Сегодня", "Вчера", then dates.
pub fn history_days<'a, T>(
    items: &'a [T],
    at: impl Fn(&T) -> i64,
    now: i64,
    local: impl Fn(i64) -> i64,
) -> Vec<Day<'a, T>> {
    let today = local(now);
    let mut days: Vec<Day<'a, T>> = Vec::new();
    for it in items {
        let day = local(at(it));
        if days.last().is_none_or(|d| d.day != day) {
            let label = match today - day {
                0 => "Сегодня".into(),
                1 => "Вчера".into(),
                _ => long_day_label(day, year_of(today)),
            };
            days.push(Day {
                day,
                label,
                items: Vec::new(),
            });
        }
        if let Some(d) = days.last_mut() {
            d.items.push(it);
        }
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_756_800; // Friday, 23 October 2026, 12:00 UTC
    const H: i64 = 3600;

    fn utc(t: i64) -> i64 {
        t.div_euclid(86400)
    }

    #[test]
    fn the_week_starts_today_and_sorts_by_time() {
        let items = [
            ("late", NOW + 30 * H),
            ("soon", NOW + H),
            ("past week", NOW + 8 * 24 * H),
            ("morning", NOW - 3 * H),
        ];
        let days = schedule_days(&items, |it| Some(it.1), NOW, utc);
        assert_eq!(days.len(), 7);
        assert_eq!(days[0].label, "Сегодня");
        assert_eq!(days[1].label, "Завтра");
        assert_eq!(days[2].label, "Вс, 25 октября");
        fn names<'a>(d: &Day<'a, (&'static str, i64)>) -> Vec<&'static str> {
            d.items.iter().map(|i| i.0).collect()
        }
        assert_eq!(names(&days[0]), ["morning", "soon"]);
        assert_eq!(names(&days[1]), ["late"]);
    }

    #[test]
    fn the_finale_is_named() {
        assert_eq!(episode_line(12, 12), "Серия 12 из 12, финал");
        assert_eq!(episode_line(3, 12), "Серия 3 из 12");
        assert_eq!(episode_line(5, 0), "Серия 5");
    }

    #[test]
    fn history_groups_by_day_newest_first() {
        let items = [NOW - H, NOW - 2 * H, NOW - 30 * H, NOW - 400 * 24 * H];
        let days = history_days(&items, |t| *t, NOW, utc);
        let labels: Vec<&str> = days.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["Сегодня", "Вчера", "Четверг, 18 сентября 2025"]);
        assert_eq!(days[0].items.len(), 2);
    }
}
