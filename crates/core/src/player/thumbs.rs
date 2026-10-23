//! Seek-bar preview frames. HLS segments start with a keyframe, so for each sampled segment
//! we download only its first kilobytes and let mpv decode one small JPEG from them.

pub use url::Url;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub url: Url,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    /// Variant playlists with their bandwidth.
    Master(Vec<(u64, Url)>),
    Media(Vec<Segment>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Thumb {
    pub t: f64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbsEvent {
    pub shikimori_id: i64,
    pub episode: i64,
    pub items: Vec<Thumb>,
}

fn attr(line: &str, name: &str) -> Option<String> {
    let start = line.find(&format!("{name}="))? + name.len() + 1;
    let rest = &line[start..];
    let end = rest.find(',').unwrap_or(rest.len());
    Some(rest[..end].trim_matches('"').to_string())
}

pub fn parse_playlist(body: &str, base: &Url) -> Playlist {
    let mut variants = Vec::new();
    let mut segments = Vec::new();
    let mut pending_bw: Option<u64> = None;
    let mut pending_dur: Option<f64> = None;
    let mut clock = 0.0;
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(info) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_bw = Some(
                attr(info, "BANDWIDTH")
                    .and_then(|b| b.parse().ok())
                    .unwrap_or(u64::MAX),
            );
        } else if let Some(info) = line.strip_prefix("#EXTINF:") {
            pending_dur = info.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if !line.starts_with('#') {
            let Ok(url) = base.join(line) else { continue };
            if let Some(bw) = pending_bw.take() {
                variants.push((bw, url));
            } else if let Some(d) = pending_dur.take() {
                segments.push(Segment { start: clock, url });
                clock += d;
            }
        }
    }
    if variants.is_empty() {
        Playlist::Media(segments)
    } else {
        Playlist::Master(variants)
    }
}

/// Up to `max` segments spread evenly over the episode, with their indexes.
pub fn sample(segments: &[Segment], max: usize) -> Vec<(usize, &Segment)> {
    if segments.len() <= max {
        return segments.iter().enumerate().collect();
    }
    let step = segments.len() as f64 / max as f64;
    (0..max)
        .map(|i| {
            let idx = (i as f64 * step).floor() as usize;
            (idx, &segments[idx])
        })
        .collect()
}

/// How much of a segment to download to be sure the first keyframe is complete.
pub fn prefix_bytes(quality: Option<u32>) -> usize {
    match quality {
        Some(q) if q <= 480 => 96 * 1024,
        Some(q) if q <= 720 => 192 * 1024,
        _ => 384 * 1024,
    }
}

/// A preview set is saved as final only when nearly every frame was made; otherwise the next
/// playback of the episode fills in the missing ones.
pub fn complete_enough(done: usize, planned: usize) -> bool {
    planned > 0 && done * 10 >= planned * 9
}

/// Touched whenever an episode's previews are used; pruning goes by it.
pub const USED_MARK: &str = ".used";

pub fn mark_used(dir: &std::path::Path) {
    let _ = std::fs::write(dir.join(USED_MARK), b"");
}

/// Keeps the previews of the `keep` most recently played episodes and removes the rest.
pub fn prune(root: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| {
            let used = std::fs::metadata(p.join(USED_MARK))
                .or_else(|_| std::fs::metadata(&p))
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (used, p)
        })
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
    for (_, dir) in dirs.into_iter().skip(keep) {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::debug!("cannot remove old previews {}: {e}", dir.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example/v/index.m3u8").unwrap()
    }

    #[test]
    fn a_preview_set_is_kept_only_when_nearly_complete() {
        assert!(complete_enough(150, 150));
        assert!(complete_enough(136, 150));
        assert!(
            !complete_enough(40, 150),
            "a flaky network left most frames missing"
        );
        assert!(!complete_enough(0, 0));
    }

    #[test]
    fn old_episode_previews_are_removed_first() {
        let root = tempfile::tempdir().unwrap();
        let t0 = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for (i, name) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("00000.jpg"), b"x").unwrap();
            mark_used(&dir);
            let at = t0 + std::time::Duration::from_secs(60 * i as u64);
            std::fs::File::options()
                .write(true)
                .open(dir.join(USED_MARK))
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        prune(root.path(), 3);
        let mut left: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["c", "d", "e"]);
    }

    #[test]
    fn parses_master_playlist_variants() {
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080\nhigh/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=854x480\nlow/index.m3u8\n";
        match parse_playlist(body, &base()) {
            Playlist::Master(v) => {
                assert_eq!(v.len(), 2);
                let lowest = v.iter().min_by_key(|(bw, _)| *bw).unwrap();
                assert_eq!(lowest.1.as_str(), "https://cdn.example/v/low/index.m3u8");
            }
            Playlist::Media(_) => panic!("expected a master playlist"),
        }
    }

    #[test]
    fn parses_media_playlist_with_start_times() {
        let body = "#EXTM3U\n#EXT-X-TARGETDURATION:12\n#EXTINF:11.5,\nseg0.ts\n#EXTINF:8.5,\nhttps://other.example/seg1.ts\n#EXTINF:10,\nseg2.ts\n#EXT-X-ENDLIST\n";
        match parse_playlist(body, &base()) {
            Playlist::Media(segs) => {
                let starts: Vec<f64> = segs.iter().map(|s| s.start).collect();
                assert_eq!(starts, vec![0.0, 11.5, 20.0]);
                assert_eq!(segs[0].url.as_str(), "https://cdn.example/v/seg0.ts");
                assert_eq!(segs[1].url.as_str(), "https://other.example/seg1.ts");
            }
            Playlist::Master(_) => panic!("expected a media playlist"),
        }
    }

    #[test]
    fn samples_evenly_and_keeps_short_lists() {
        let segs: Vec<Segment> = (0..300)
            .map(|i| Segment {
                start: i as f64 * 10.0,
                url: base(),
            })
            .collect();
        let picked = sample(&segs, 120);
        assert_eq!(picked.len(), 120);
        assert_eq!(picked[0].0, 0);
        assert!(picked.windows(2).all(|w| w[1].0 > w[0].0));
        assert_eq!(sample(&segs[..40], 120).len(), 40);
    }

    #[test]
    fn prefix_size_grows_with_quality() {
        assert!(prefix_bytes(Some(480)) < prefix_bytes(Some(720)));
        assert!(prefix_bytes(Some(720)) < prefix_bytes(Some(1080)));
        assert_eq!(prefix_bytes(None), prefix_bytes(Some(1080)));
    }
}
