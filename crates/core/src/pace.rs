//! How dub teams keep up with a title on the air: which teams carry it, how far each has got, and
//! how long after the Japanese broadcast an episode usually turned up in them. Nobody publishes
//! dub schedules; this comes from when the app itself first saw each episode in a source
//! (`episode_index.first_seen_at`), so it knows only titles it has looked at.

use std::collections::HashMap;

use crate::db::Db;
use crate::error::AppError;

const WEEK: i64 = 7 * 86_400;
/// Seen later than this after airing, an episode was found by a first look back, not as it came.
const FRESH: i64 = 3 * 86_400;

#[derive(Debug, Clone, PartialEq)]
pub struct DubPace {
    pub key: String,
    /// The furthest episode out in this dub.
    pub episodes: i64,
    /// The usual time from the broadcast to the dub, when it was seen coming.
    pub lag_hours: Option<i64>,
}

/// (dub key, episode, first seen) of each title among `ids`; subtitles are not dubs and are left out.
pub fn seen(db: &Db, ids: &[i64]) -> Result<HashMap<i64, Vec<(String, i64, i64)>>, AppError> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT dub_key, episode, MIN(first_seen_at) FROM episode_index
         WHERE shikimori_id = ?1 AND dub_key NOT LIKE '%-sub'
         GROUP BY dub_key, episode",
    )?;
    let mut out = HashMap::new();
    for &id in ids {
        let rows: Vec<(String, i64, i64)> = stmt
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        if !rows.is_empty() {
            out.insert(id, rows);
        }
    }
    Ok(out)
}

/// Each team's pace from `rows`, for a title whose episode `next` airs at `next_at` (weekly, so
/// episode k aired `next - k` weeks before). Furthest first, then quickest.
pub fn pace(rows: &[(String, i64, i64)], next: i64, next_at: i64) -> Vec<DubPace> {
    // What a team had when the app first looked was found then, not as it came out.
    let mut first: HashMap<&str, i64> = HashMap::new();
    for (key, _, seen) in rows {
        let f = first.entry(key.as_str()).or_insert(*seen);
        *f = (*f).min(*seen);
    }
    let mut by_dub: HashMap<&str, (i64, Vec<i64>)> = HashMap::new();
    for (key, episode, seen) in rows {
        let found_later = *seen > first[key.as_str()] + 60;
        let e = by_dub.entry(key.as_str()).or_default();
        e.0 = e.0.max(*episode);
        let aired = next_at - (next - episode) * WEEK;
        let lag = seen - aired;
        if found_later && *episode < next && (0..=FRESH).contains(&lag) {
            e.1.push(lag);
        }
    }
    let mut out: Vec<DubPace> = by_dub
        .into_iter()
        .map(|(key, (episodes, mut lags))| {
            lags.sort_unstable();
            DubPace {
                key: key.to_string(),
                episodes,
                lag_hours: (!lags.is_empty()).then(|| (lags[lags.len() / 2] + 1800) / 3600),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.episodes
            .cmp(&a.episodes)
            .then(
                a.lag_hours
                    .unwrap_or(i64::MAX)
                    .cmp(&b.lag_hours.unwrap_or(i64::MAX)),
            )
            .then(a.key.cmp(&b.key))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_team_seen_coming_gets_its_usual_delay_and_a_first_look_back_does_not_count() {
        let next_at = 100 * WEEK;
        let aired = |k: i64| next_at - (5 - k) * WEEK;
        let rows = vec![
            // Found all at once long after: no delay known.
            ("old".to_string(), 1, aired(4) + 3600),
            ("old".to_string(), 2, aired(4) + 3600),
            // First looked at a day after episode 2; then seen 7 h and 6 h after each broadcast.
            ("quick".to_string(), 2, aired(2) + 24 * 3600),
            ("quick".to_string(), 3, aired(3) + 7 * 3600),
            ("quick".to_string(), 4, aired(4) + 6 * 3600),
        ];
        assert_eq!(
            pace(&rows, 5, next_at),
            vec![
                DubPace {
                    key: "quick".into(),
                    episodes: 4,
                    lag_hours: Some(7)
                },
                DubPace {
                    key: "old".into(),
                    episodes: 2,
                    lag_hours: None
                },
            ]
        );
    }
}
