//! trace.moe: which anime, episode and moment a screenshot is from. Anonymous use is limited
//! (100 searches a month at the time of writing), so the app sends a frame only when asked.

use serde::Deserialize;

use crate::error::AppError;
use crate::net::HttpClient;

const URL: &str = "https://api.trace.moe/search?anilistInfo&cutBorders";
/// trace.moe's own advice: below about 0.9 a match is usually wrong.
const MIN_SIMILARITY: f64 = 0.87;
/// With nothing that sure, the best guesses down to this are offered as maybes.
const MAYBE_SIMILARITY: f64 = 0.7;
const MAX_MAYBES: usize = 2;
const MAX_HITS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// MyAnimeList id, which is also the Shikimori id.
    pub mal_id: i64,
    pub episode: Option<i64>,
    /// Second of the episode the frame is from.
    pub at: f64,
    pub similarity: f64,
    /// A still of the matched moment, hosted by trace.moe.
    pub image: Option<String>,
    /// Similar enough to trust; otherwise one of the best guesses.
    pub sure: bool,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    error: String,
    #[serde(default)]
    result: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    anilist: Option<Anilist>,
    /// A number, a list of numbers or a string: only a plain number is an episode.
    episode: Option<serde_json::Value>,
    at: f64,
    similarity: f64,
    image: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Anilist {
    id_mal: Option<i64>,
    #[serde(default)]
    is_adult: bool,
}

/// The likely titles, best first: one hit per title, adult ones left out, weak matches only
/// when nothing is sure.
pub fn parse(body: &str) -> Result<Vec<Hit>, String> {
    let reply: Reply =
        serde_json::from_str(body).map_err(|_| "trace.moe ответил непонятно".to_string())?;
    if !reply.error.is_empty() {
        return Err(reply.error);
    }
    // What came back, so a search that finds nothing can be told apart from one that failed.
    for r in reply.result.iter().take(5) {
        tracing::info!(
            "trace.moe: mal {:?}, similarity {:.3}, episode {:?}",
            r.anilist.as_ref().and_then(|a| a.id_mal),
            r.similarity,
            r.episode
        );
    }
    let mut hits: Vec<Hit> = Vec::new();
    for r in reply.result {
        let Some(a) = r.anilist else { continue };
        let Some(mal_id) = a.id_mal.filter(|_| !a.is_adult) else {
            continue;
        };
        if r.similarity < MAYBE_SIMILARITY || hits.iter().any(|h| h.mal_id == mal_id) {
            continue;
        }
        hits.push(Hit {
            mal_id,
            episode: r.episode.and_then(|e| e.as_i64()),
            at: r.at,
            similarity: r.similarity,
            image: r.image,
            sure: r.similarity >= MIN_SIMILARITY,
        });
    }
    hits.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    if hits.iter().any(|h| h.sure) {
        hits.retain(|h| h.sure);
        hits.truncate(MAX_HITS);
    } else {
        hits.truncate(MAX_MAYBES);
    }
    Ok(hits)
}

/// Sends a frame (JPEG or PNG bytes) to trace.moe.
pub async fn search(http: &HttpClient, image: Vec<u8>) -> Result<Vec<Hit>, AppError> {
    let err = |m: String| AppError::new("Trace", m);
    let res = http
        .inner()
        .post(URL)
        .header("Content-Type", "image/jpeg")
        .body(image)
        .send()
        .await
        .map_err(|e| err(format!("trace.moe не отвечает: {e}")))?;
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    match status {
        200 => parse(&body).map_err(err),
        402 => Err(err(
            "Бесплатные поиски trace.moe на этот месяц закончились".into()
        )),
        413 => Err(err("Кадр слишком большой".into())),
        429 => Err(err(
            "trace.moe просит подождать: слишком много поисков подряд".into(),
        )),
        _ => Err(err(parse(&body)
            .err()
            .unwrap_or_else(|| format!("trace.moe: HTTP {status}")))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from a real reply.
    const REPLY: &str = r#"{"frameCount":1736134508,"error":"","result":[
      {"anilist":{"id":21034,"idMal":29787,"isAdult":false},"filename":"x.mp4","episode":1,"from":272.1,"at":279.7795,"to":282.6,"similarity":0.9922,"video":"v","image":"https://api.trace.moe/image/a"},
      {"anilist":{"id":21034,"idMal":29787,"isAdult":false},"episode":2,"at":10.0,"similarity":0.95,"image":"https://api.trace.moe/image/b"},
      {"anilist":{"id":1,"idMal":1,"isAdult":false},"episode":[3,4],"at":50.0,"similarity":0.93,"image":null},
      {"anilist":{"id":2,"idMal":2,"isAdult":true},"episode":1,"at":5.0,"similarity":0.99},
      {"anilist":{"id":3,"idMal":null,"isAdult":false},"episode":1,"at":5.0,"similarity":0.98},
      {"anilist":{"id":4,"idMal":4,"isAdult":false},"episode":1,"at":5.0,"similarity":0.5}
    ]}"#;

    #[test]
    fn one_hit_per_title_best_first_without_adult_or_weak_matches() {
        let hits = parse(REPLY).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.mal_id).collect::<Vec<_>>(),
            [29787, 1]
        );
        assert_eq!(hits[0].episode, Some(1));
        assert_eq!(hits[0].at, 279.7795);
        assert_eq!(
            hits[1].episode, None,
            "a range of episodes is no single episode"
        );
        assert!(hits.iter().all(|h| h.sure));
    }

    #[test]
    fn with_nothing_sure_the_best_guesses_are_maybes() {
        let reply = r#"{"error":"","result":[
          {"anilist":{"idMal":7,"isAdult":false},"episode":1,"at":1.0,"similarity":0.81},
          {"anilist":{"idMal":8,"isAdult":false},"episode":1,"at":1.0,"similarity":0.78},
          {"anilist":{"idMal":9,"isAdult":false},"episode":1,"at":1.0,"similarity":0.75},
          {"anilist":{"idMal":10,"isAdult":false},"episode":1,"at":1.0,"similarity":0.6}
        ]}"#;
        let hits = parse(reply).unwrap();
        assert_eq!(hits.iter().map(|h| h.mal_id).collect::<Vec<_>>(), [7, 8]);
        assert!(hits.iter().all(|h| !h.sure));
    }

    #[test]
    fn an_error_reply_is_an_error() {
        assert_eq!(
            parse(r#"{"error":"Search queue is full","result":[]}"#),
            Err("Search queue is full".into())
        );
        assert!(parse("<html>").is_err());
    }
}
