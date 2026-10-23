//! AniSkip: openings and endings people marked on episodes, by MyAnimeList id (= Shikimori id).
//! A mark fits only a video of about the same length: a dub that adds a logo shifts everything.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::extensions::types::SkipTimes;
use crate::net::{FetchRequest, HttpClient};

const BASE: &str = "https://api.aniskip.com/v2";
/// How far our video's length may be from the marked one's.
const TOLERANCE: f64 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub kind: String,
    pub start: f64,
    pub end: f64,
    /// Length of the video the mark was made on.
    pub length: f64,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    results: Vec<Result>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Result {
    interval: Interval,
    skip_type: String,
    episode_length: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Interval {
    start_time: f64,
    end_time: f64,
}

pub fn parse(body: &str) -> Vec<Mark> {
    serde_json::from_str::<Reply>(body)
        .map(|r| {
            r.results
                .into_iter()
                .filter(|r| r.interval.end_time > r.interval.start_time)
                .map(|r| Mark {
                    kind: r.skip_type,
                    start: r.interval.start_time,
                    end: r.interval.end_time,
                    length: r.episode_length,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The opening and ending marked on a video as long as ours, the closest in length if several.
pub fn pick(marks: &[Mark], duration: f64) -> SkipTimes {
    let best = |kinds: &[&str]| {
        marks
            .iter()
            .filter(|m| kinds.contains(&m.kind.as_str()))
            .filter(|m| (m.length - duration).abs() <= TOLERANCE && m.end <= duration + 1.0)
            .min_by(|a, b| {
                (a.length - duration)
                    .abs()
                    .total_cmp(&(b.length - duration).abs())
            })
            .map(|m| (m.start, m.end))
    };
    SkipTimes {
        opening: best(&["op", "mixed-op"]),
        ending: best(&["ed", "mixed-ed"]),
    }
}

pub struct AniSkip {
    http: HttpClient,
    cache: Mutex<HashMap<(i64, i64), Vec<Mark>>>,
}

impl AniSkip {
    pub fn new(http: HttpClient) -> Arc<Self> {
        Arc::new(Self {
            http,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// The marks of an episode; nothing when AniSkip is down (not kept, so it is asked again).
    pub async fn marks(&self, id: i64, episode: i64) -> Vec<Mark> {
        if let Some(m) = self.cache.lock().unwrap().get(&(id, episode)) {
            return m.clone();
        }
        let url = format!(
            "{BASE}/skip-times/{id}/{episode}?types[]=op&types[]=ed&types[]=mixed-op&types[]=mixed-ed&episodeLength=0"
        );
        match self.http.fetch(FetchRequest::get(url)).await {
            // 404 is "nothing marked yet": an answer worth keeping too.
            Ok(r) if r.status == 200 || r.status == 404 => {
                let marks = parse(&r.body);
                self.cache
                    .lock()
                    .unwrap()
                    .insert((id, episode), marks.clone());
                marks
            }
            Ok(r) => {
                tracing::debug!("aniskip {id}/{episode}: HTTP {}", r.status);
                Vec::new()
            }
            Err(e) => {
                tracing::debug!("aniskip {id}/{episode}: {}", e.message);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRIEREN_5: &str = r#"{"found":true,"results":[
      {"interval":{"startTime":153.346,"endTime":243.346},"skipType":"op","skipId":"a","episodeLength":1469.9685},
      {"interval":{"startTime":1370,"endTime":1460},"skipType":"ed","skipId":"b","episodeLength":1471},
      {"interval":{"startTime":160,"endTime":250},"skipType":"op","skipId":"c","episodeLength":1478}
    ],"message":"Successfully found skip times","statusCode":200}"#;

    #[test]
    fn marks_come_from_the_reply() {
        let marks = parse(FRIEREN_5);
        assert_eq!(marks.len(), 3);
        assert_eq!(
            marks[1],
            Mark {
                kind: "ed".into(),
                start: 1370.0,
                end: 1460.0,
                length: 1471.0
            }
        );
        assert!(parse(r#"{"found":false,"results":[],"statusCode":404}"#).is_empty());
        assert!(parse("<html>").is_empty());
    }

    #[test]
    fn a_mark_is_used_only_on_a_video_of_its_length_the_closest_one_first() {
        let marks = parse(FRIEREN_5);
        assert_eq!(
            pick(&marks, 1470.0),
            SkipTimes {
                opening: Some((153.346, 243.346)),
                ending: Some((1370.0, 1460.0))
            }
        );
        // 1477 s: only the second opening is close enough, the ending video is 6 s shorter.
        assert_eq!(
            pick(&marks, 1477.0),
            SkipTimes {
                opening: Some((160.0, 250.0)),
                ending: None
            }
        );
        // A dub with a 30-second logo: nothing fits, nothing is skipped by mistake.
        assert_eq!(pick(&marks, 1500.0), SkipTimes::default());
    }
}
