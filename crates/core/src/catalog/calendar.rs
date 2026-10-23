//! Shikimori's release calendar: which episode of which ongoing airs when (Japanese broadcast
//! time, with its offset, so the UI shows it in the user's own time zone).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex as AsyncMutex;

use super::anilist::AniList;
use super::Catalog;
use crate::db::now_ts;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarEntry {
    pub anime_id: i64,
    pub episode: i64,
    /// ISO 8601 with offset, as Shikimori sends it.
    pub at: String,
}

#[derive(Deserialize)]
struct Raw {
    next_episode: Option<i64>,
    next_episode_at: Option<String>,
    anime: RawAnime,
}

#[derive(Deserialize)]
struct RawAnime {
    id: i64,
}

/// Entries of `/api/calendar`; announcements without a date or episode are left out.
pub fn parse_calendar(json: &str) -> Result<Vec<CalendarEntry>, AppError> {
    let raw: Vec<Raw> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .filter_map(|r| {
            Some(CalendarEntry {
                anime_id: r.anime.id,
                episode: r.next_episode?,
                at: r.next_episode_at?,
            })
        })
        .collect())
}

/// Days since 1970-01-01 of a civil date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "2026-09-30T04:00:00Z" for a Unix time.
pub fn iso_utc(unix: i64) -> String {
    let (y, m, d) = civil_from_days(unix.div_euclid(86400));
    let s = unix.rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// Unix time of "YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)"; None when it does not parse.
pub fn unix_of(iso: &str) -> Option<i64> {
    let num = |a: usize, b: usize| iso.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, s) = (
        num(0, 4)?,
        num(5, 7)?,
        num(8, 10)?,
        num(11, 13)?,
        num(14, 16)?,
        num(17, 19)?,
    );
    let zone = iso[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone {
        "Z" | "" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let hours: i64 = z.get(1..3)?.parse().ok()?;
            let mins: i64 = z.get(4..6)?.parse().ok()?;
            sign * (hours * 3600 + mins * 60)
        }
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - offset)
}

/// One schedule from two sources: `first` wins when both list the same episode.
pub fn merge_schedule(first: Vec<CalendarEntry>, extra: Vec<CalendarEntry>) -> Vec<CalendarEntry> {
    let mut out = first;
    for e in extra {
        if !out
            .iter()
            .any(|o| o.anime_id == e.anime_id && o.episode == e.episode)
        {
            out.push(e);
        }
    }
    out.sort_by_key(|e| unix_of(&e.at).unwrap_or(i64::MAX));
    out
}

const SCHEDULE_TTL: Duration = Duration::from_secs(30 * 60);

/// The release schedule: AniList's airing list (it keeps episodes that already aired today,
/// Chinese and Korean titles too) filled up with Shikimori's calendar, which only knows the
/// next episode of each title.
pub struct Schedule {
    catalog: Arc<Catalog>,
    anilist: Arc<AniList>,
    cache: AsyncMutex<Option<(Instant, Vec<CalendarEntry>)>>,
}

impl Schedule {
    pub fn new(catalog: Arc<Catalog>, anilist: Arc<AniList>) -> Self {
        Self {
            catalog,
            anilist,
            cache: AsyncMutex::new(None),
        }
    }

    /// From a day and a half ago (today, whatever the viewer's time zone) to eight days ahead.
    pub async fn entries(&self) -> Result<Vec<CalendarEntry>, AppError> {
        if let Some((at, list)) = self.cache.lock().await.as_ref() {
            if at.elapsed() < SCHEDULE_TTL {
                return Ok(list.clone());
            }
        }
        let now = now_ts();
        let airing: Vec<CalendarEntry> = self
            .anilist
            .airing(now - 36 * 3600, now + 8 * 86400)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|a| CalendarEntry {
                anime_id: a.mal_id,
                episode: a.episode,
                at: iso_utc(a.at),
            })
            .collect();
        let calendar = match self.catalog.calendar().await {
            Ok(list) => list,
            Err(e) if !airing.is_empty() => {
                tracing::info!("Shikimori calendar unavailable: {e}");
                Vec::new()
            }
            Err(e) => return Err(e),
        };
        let merged = merge_schedule(airing, calendar);
        *self.cache.lock().await = Some((Instant::now(), merged.clone()));
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test --lib live_schedule -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_schedule() {
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
        let schedule = Schedule::new(catalog.clone(), AniList::new(http, db));
        let entries = schedule.entries().await.unwrap();
        let now = now_ts();
        let today: Vec<_> = entries
            .iter()
            .filter(|e| unix_of(&e.at).is_some_and(|t| (t - now).abs() < 12 * 3600))
            .collect();
        println!(
            "{} entries this week, {} within 12 h of now:",
            entries.len(),
            today.len()
        );
        let titles = catalog
            .get_many(&today.iter().map(|e| e.anime_id).collect::<Vec<_>>())
            .await
            .unwrap();
        for e in today {
            let name = titles
                .iter()
                .find(|a| a.id == e.anime_id)
                .map_or("?".to_string(), |a| {
                    a.russian.clone().unwrap_or(a.name.clone())
                });
            println!("  {} ep {} {}", e.at, e.episode, name);
        }
    }

    #[test]
    fn iso_times_with_offsets_become_unix() {
        assert_eq!(unix_of("2026-09-30T04:00:00Z"), Some(1790740800));
        assert_eq!(unix_of("2026-09-30T07:00:00.000+03:00"), Some(1790740800));
        assert_eq!(unix_of("2026-09-29T20:00:00-08:00"), Some(1790740800));
        assert_eq!(unix_of("вчера"), None);
    }

    #[test]
    fn unix_times_become_utc_iso() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1790740800), "2026-09-30T04:00:00Z");
        assert_eq!(iso_utc(1790769600), "2026-09-30T12:00:00Z");
    }

    fn entry(id: i64, episode: i64, at: &str) -> CalendarEntry {
        CalendarEntry {
            anime_id: id,
            episode,
            at: at.into(),
        }
    }

    #[test]
    fn both_calendars_merge_without_twins() {
        let anilist = vec![
            entry(2, 13, "2026-09-30T02:00:00Z"),
            entry(1, 19, "2026-09-30T08:00:00Z"),
        ];
        let shikimori = vec![
            entry(1, 19, "2026-09-30T08:05:00.000+03:00"),
            entry(3, 1, "2026-09-30T06:00:00.000+03:00"),
        ];
        let merged = merge_schedule(anilist, shikimori);
        let got: Vec<(i64, i64)> = merged.iter().map(|e| (e.anime_id, e.episode)).collect();
        assert_eq!(
            got,
            [(2, 13), (3, 1), (1, 19)],
            "by time, one row per episode"
        );
        assert_eq!(merged[2].at, "2026-09-30T08:00:00Z", "AniList's time wins");
    }
}
