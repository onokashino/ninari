//! Which of a source's streams to play. The user picks a quality once; it acts as a cap for
//! every next episode, since sources offer different steps (1080/720/480, 720/480/360...).

use crate::extensions::types::Stream;

/// Qualities the source offers, best first, each once.
pub fn offered(streams: &[Stream]) -> Vec<u32> {
    let mut out: Vec<u32> = streams.iter().filter_map(|s| s.quality).collect();
    out.sort_unstable_by(|a, b| b.cmp(a));
    out.dedup();
    out
}

/// Index of the stream to play: the best one within `cap` (`None` means the best overall).
/// When nothing fits, the lowest known quality. `streams` are ordered best first.
pub fn pick(streams: &[Stream], cap: Option<u32>) -> usize {
    let Some(cap) = cap else { return 0 };
    let known = || {
        streams
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i, s.quality?)))
    };
    known()
        .filter(|&(_, q)| q <= cap)
        .max_by_key(|&(i, q)| (q, std::cmp::Reverse(i)))
        .or_else(|| known().min_by_key(|&(i, q)| (q, i)))
        .map_or(0, |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(kind: &str, quality: Option<u32>) -> Stream {
        Stream {
            url: format!("https://cdn.example/{kind}/{quality:?}"),
            kind: kind.into(),
            quality,
            headers: Default::default(),
        }
    }

    fn cvh() -> Vec<Stream> {
        vec![
            stream("hls", Some(1080)),
            stream("mp4", Some(1080)),
            stream("mp4", Some(720)),
            stream("mp4", Some(480)),
            stream("mp4", None),
        ]
    }

    #[test]
    fn offers_each_known_quality_once_best_first() {
        assert_eq!(offered(&cvh()), vec![1080, 720, 480]);
        assert!(offered(&[stream("mp4", None)]).is_empty());
    }

    #[test]
    fn picks_the_best_stream_within_the_cap() {
        let s = cvh();
        assert_eq!(pick(&s, None), 0, "auto takes the best, HLS first");
        assert_eq!(pick(&s, Some(1080)), 0);
        assert_eq!(pick(&s, Some(720)), 2);
        assert_eq!(
            pick(&s, Some(600)),
            3,
            "a cap between steps takes the next lower one"
        );
    }

    #[test]
    fn falls_back_to_the_lowest_known_quality_when_nothing_fits() {
        let s = vec![
            stream("hls", Some(720)),
            stream("hls", Some(480)),
            stream("mp4", None),
        ];
        assert_eq!(pick(&s, Some(360)), 1);
        assert_eq!(pick(&[stream("mp4", None)], Some(360)), 0);
    }
}
