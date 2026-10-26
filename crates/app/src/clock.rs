//! Times on the user's clock: the schedule shows local days and hours.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Seconds the local time zone is ahead of UTC at `unix`.
#[cfg(unix)]
pub fn utc_offset(unix: i64) -> i64 {
    let t = unix as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return 0;
    }
    tm.tm_gmtoff as i64
}

/// Windows' C library has no `tm_gmtoff`: the local date and time read as if UTC, minus `unix`.
#[cfg(windows)]
pub fn utc_offset(unix: i64) -> i64 {
    unsafe extern "C" {
        fn _localtime64_s(tm: *mut libc::tm, time: *const i64) -> i32;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { _localtime64_s(&mut tm, &unix) } != 0 {
        return 0;
    }
    let days = days_from_civil(
        tm.tm_year as i64 + 1900,
        tm.tm_mon as i64 + 1,
        tm.tm_mday as i64,
    );
    days * 86400 + tm.tm_hour as i64 * 3600 + tm.tm_min as i64 * 60 + tm.tm_sec as i64 - unix
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar (Howard Hinnant's).
#[cfg_attr(not(windows), allow(dead_code))]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "18:00" on the user's clock.
pub fn local_time(unix: i64) -> String {
    let s = (unix + utc_offset(unix)).rem_euclid(86400);
    format!("{:02}:{:02}", s / 3600, s / 60 % 60)
}

const WEEKDAYS: [&str; 7] = ["чт", "пт", "сб", "вс", "пн", "вт", "ср"];
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

/// "чт, 23 октября" for the local day of `unix + offset`.
pub fn day_label(unix: i64, offset: i64) -> String {
    let days = (unix + offset).div_euclid(86400);
    let (_, m, d) = ninari_core::catalog::calendar::civil_from_days(days);
    // 1970-01-01 was a Thursday.
    format!(
        "{}, {d} {}",
        WEEKDAYS[days.rem_euclid(7) as usize],
        MONTHS[m as usize - 1]
    )
}

const WEEKDAYS_LONG: [&str; 7] = [
    "четверг",
    "пятница",
    "суббота",
    "воскресенье",
    "понедельник",
    "вторник",
    "среда",
];

/// The local day number (days since 1970-01-01) of `unix`.
pub fn local_day(unix: i64, offset: i64) -> i64 {
    (unix + offset).div_euclid(86400)
}

/// "Пятница, 23 октября", with the year when it is not this one.
pub fn long_day_label(day: i64, this_year: i64) -> String {
    let (y, m, d) = ninari_core::catalog::calendar::civil_from_days(day);
    let weekday = WEEKDAYS_LONG[day.rem_euclid(7) as usize];
    let year = if y == this_year {
        String::new()
    } else {
        format!(" {y}")
    };
    capitalize(&format!("{weekday}, {d} {}{year}", MONTHS[m as usize - 1]))
}

/// "Пт, 23 октября" for a local day number.
pub fn short_day_label(day: i64) -> String {
    let (_, m, d) = ninari_core::catalog::calendar::civil_from_days(day);
    capitalize(&format!(
        "{}, {d} {}",
        WEEKDAYS[day.rem_euclid(7) as usize],
        MONTHS[m as usize - 1]
    ))
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The year of a local day number.
pub fn year_of(day: i64) -> i64 {
    ninari_core::catalog::calendar::civil_from_days(day).0
}

/// "Следующая серия чт, 23 октября, 18:00" on the user's clock.
pub fn next_episode_label(iso: Option<&str>) -> Option<String> {
    let at = ninari_core::catalog::calendar::unix_of(iso?)?;
    Some(format!(
        "Следующая серия {}, {}",
        day_label(at, utc_offset(at)),
        local_time(at)
    ))
}

/// Episodes airing on the local day of `now`, the followed titles first, each group by time. `at`
/// is an item's Unix time, `offset` the zone's offset at a time.
pub fn today_picks<'a, T>(
    items: &'a [T],
    at: impl Fn(&T) -> Option<i64>,
    followed: impl Fn(&T) -> bool,
    now: i64,
    offset: impl Fn(i64) -> i64,
    limit: usize,
) -> Vec<&'a T> {
    let day = |t: i64| (t + offset(t)).div_euclid(86400);
    let today = day(now);
    let mut picks: Vec<(&T, i64)> = items
        .iter()
        .filter_map(|it| at(it).filter(|t| day(*t) == today).map(|t| (it, t)))
        .collect();
    picks.sort_by_key(|(it, t)| (!followed(it), *t));
    picks.into_iter().take(limit).map(|(it, _)| it).collect()
}

/// The library statuses whose titles the user keeps up with.
pub fn is_followed(status: Option<&str>) -> bool {
    matches!(status, Some("watching" | "planned" | "on_hold"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_count_days_from_the_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 10, 23) * 86400, 1_792_713_600);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }

    #[test]
    fn today_is_the_local_day_with_followed_titles_first() {
        // 2026-10-23 12:00 UTC; the zone is UTC+4.
        let now = 1_792_756_800;
        let hour = 3600;
        let items = [
            ("late tonight", now + 10 * hour, false), // 02:00 tomorrow locally
            ("evening", now + 6 * hour, false),       // 22:00
            ("morning", now - 7 * hour, true),        // 09:00, followed
            ("early", now - 11 * hour, false),        // 05:00
            ("yesterday", now - 17 * hour, true),     // 23:00 yesterday
        ];
        let picks = today_picks(&items, |it| Some(it.1), |it| it.2, now, |_| 4 * hour, 10);
        let names: Vec<&str> = picks.iter().map(|it| it.0).collect();
        assert_eq!(names, ["morning", "early", "evening"]);
        assert_eq!(
            today_picks(&items, |it| Some(it.1), |it| it.2, now, |_| 4 * hour, 1).len(),
            1
        );
    }

    #[test]
    fn day_labels_are_russian_and_local() {
        // 2026-10-23 is a Friday; 22:00 UTC is already Saturday at UTC+4.
        assert_eq!(day_label(1_792_756_800, 0), "пт, 23 октября");
        assert_eq!(
            day_label(1_792_756_800 + 10 * 3600, 4 * 3600),
            "сб, 24 октября"
        );
    }

    #[test]
    fn long_labels_name_the_weekday_and_an_other_year() {
        let day = local_day(1_792_756_800, 0);
        assert_eq!(long_day_label(day, 2026), "Пятница, 23 октября");
        assert_eq!(long_day_label(day, 2027), "Пятница, 23 октября 2026");
        assert_eq!(short_day_label(day), "Пт, 23 октября");
    }

    #[test]
    fn followed_means_watching_planned_or_on_hold() {
        assert!(is_followed(Some("planned")));
        assert!(!is_followed(Some("completed")));
        assert!(!is_followed(None));
    }
}
