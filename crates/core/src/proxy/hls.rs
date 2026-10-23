//! HLS playlist rewriting: every URI in a playlist is replaced so that the player keeps
//! requesting segments, keys and nested playlists through the local proxy.

pub use url::Url;

pub fn is_playlist(content_type: Option<&str>, url: &str) -> bool {
    if content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("mpegurl")) {
        return true;
    }
    Url::parse(url)
        .map(|u| u.path().to_ascii_lowercase().ends_with(".m3u8"))
        .unwrap_or(false)
}

/// A playlist sent as `application/octet-stream` or `text/plain` is recognized by its start.
pub fn looks_like_playlist(head: &[u8]) -> bool {
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    head.trim_ascii_start().starts_with(b"#EXTM3U")
}

/// Only web URLs go through the proxy; `skd:` and `data:` keys stay as they are.
fn web(u: Url) -> Option<Url> {
    matches!(u.scheme(), "http" | "https").then_some(u)
}

fn rewrite_uri_attr(line: &str, base: &Url, make: &dyn Fn(&Url) -> String) -> String {
    let Some(start) = line.find("URI=\"") else {
        return line.to_string();
    };
    let value_start = start + 5;
    let Some(len) = line[value_start..].find('"') else {
        return line.to_string();
    };
    let value = &line[value_start..value_start + len];
    match base.join(value).ok().and_then(web) {
        Some(u) => format!(
            "{}{}{}",
            &line[..value_start],
            make(&u),
            &line[value_start + len..]
        ),
        None => line.to_string(),
    }
}

/// Attributes of an `#EXT-X-...:` tag; commas inside quoted values (CODECS) do not split.
fn attributes(list: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in list
        .char_indices()
        .chain(std::iter::once((list.len(), ',')))
    {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                if let Some((k, v)) = list[start..i].split_once('=') {
                    out.push((k.trim(), v.trim().trim_matches('"')));
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

/// Some CDNs announce nonsense bandwidths (Aniboom: 130 kbit/s for 1080p) and the player takes
/// the variant with the highest one, so a variant never announces less than a lower resolution.
/// Returns the corrected bandwidth by line number.
fn fair_bandwidths(body: &str) -> std::collections::HashMap<usize, u64> {
    let mut variants: Vec<(usize, u64, u64)> = body
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let attrs = attributes(line.trim().strip_prefix("#EXT-X-STREAM-INF:")?);
            let get = |name: &str| attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
            let height = get("RESOLUTION")?.split('x').nth(1)?.parse().ok()?;
            let bandwidth = get("BANDWIDTH")?.parse().ok()?;
            Some((i, height, bandwidth))
        })
        .collect();
    variants.sort_by_key(|&(_, height, bandwidth)| (height, bandwidth));
    let mut fixed = std::collections::HashMap::new();
    let mut floor = 0;
    for (i, _, bandwidth) in variants {
        let fair = bandwidth.max(floor + 1);
        if fair != bandwidth {
            fixed.insert(i, fair);
        }
        floor = fair;
    }
    fixed
}

fn set_bandwidth(line: &str, value: u64) -> String {
    for sep in [":BANDWIDTH=", ",BANDWIDTH="] {
        if let Some(i) = line.find(sep) {
            let start = i + sep.len();
            let end = line[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map_or(line.len(), |j| start + j);
            return format!("{}{value}{}", &line[..start], &line[end..]);
        }
    }
    line.to_string()
}

pub fn rewrite_m3u8(body: &str, base: &Url, make: &dyn Fn(&Url) -> String) -> String {
    let fair = fair_bandwidths(body);
    let mut out = String::with_capacity(body.len() + 256);
    for (i, line) in body.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // keep blank lines as they are
        } else if let Some(bandwidth) = fair.get(&i) {
            out.push_str(&set_bandwidth(trimmed, *bandwidth));
        } else if trimmed.starts_with('#') {
            out.push_str(&rewrite_uri_attr(trimmed, base, make));
        } else {
            match base.join(trimmed).ok().and_then(web) {
                Some(u) => out.push_str(&make(&u)),
                None => out.push_str(trimmed),
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_all_uris() {
        let src = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10,\nseg0.ts\n#EXTINF:10,\nhttps://cdn.example/abs/seg1.ts\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",URI=\"audio/index.m3u8\"\n\n#EXT-X-ENDLIST\n";
        let base = Url::parse("https://cdn.example/v/1080/index.m3u8?token=1").unwrap();
        let out = rewrite_m3u8(src, &base, &|u: &Url| format!("P[{u}]"));
        assert!(out.starts_with("#EXTM3U\n"));
        assert!(
            out.contains("URI=\"P[https://cdn.example/v/1080/key.bin]\",IV=0x1"),
            "{out}"
        );
        assert!(out.contains("#EXT-X-MAP:URI=\"P[https://cdn.example/v/1080/init.mp4]\""));
        assert!(out.contains("\nP[https://cdn.example/v/1080/seg0.ts]\n"));
        assert!(out.contains("\nP[https://cdn.example/abs/seg1.ts]\n"));
        assert!(out.contains("URI=\"P[https://cdn.example/v/1080/audio/index.m3u8]\""));
        assert!(out.contains("\n#EXT-X-ENDLIST"));
    }

    #[test]
    fn leaves_keys_that_are_not_urls_alone() {
        let src = "#EXTM3U
#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key-1\"
#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,AAAA\"
#EXTINF:4,
seg.ts
";
        let base = Url::parse("https://cdn.example/v/index.m3u8").unwrap();
        let out = rewrite_m3u8(src, &base, &|u: &Url| format!("P[{u}]"));
        assert!(out.contains("URI=\"skd://key-1\""), "{out}");
        assert!(out.contains("URI=\"data:text/plain;base64,AAAA\""), "{out}");
        assert!(out.contains("P[https://cdn.example/v/seg.ts]"));
    }

    #[test]
    fn recognizes_playlists_by_their_first_bytes() {
        assert!(looks_like_playlist(b"#EXTM3U\n#EXT-X-VERSION:3"));
        assert!(looks_like_playlist(b"\xEF\xBB\xBF  #EXTM3U"));
        assert!(!looks_like_playlist(b"\x47\x40\x11"));
        assert!(!looks_like_playlist(b""));
    }

    #[test]
    fn a_higher_resolution_never_announces_less_bandwidth() {
        // Aniboom's real master: the 1080p variant claims 130 kbit/s, so the player picked 720p.
        let src = concat!(
            "#EXTM3U\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=378404,RESOLUTION=640x268,AVERAGE-BANDWIDTH=300000\n",
            "media_0.m3u8\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=861860,RESOLUTION=1280x536\n",
            "media_1.m3u8\n",
            "#EXT-X-STREAM-INF:BANDWIDTH=129968,RESOLUTION=1920x804\n",
            "media_2.m3u8\n",
        );
        let base = Url::parse("https://cdn.example/v/master.m3u8").unwrap();
        let out = rewrite_m3u8(src, &base, &|u: &Url| format!("P[{u}]"));
        let bw = |res: &str| -> u64 {
            let line = out.lines().find(|l| l.contains(res)).unwrap();
            let v = line.split(":BANDWIDTH=").nth(1).unwrap();
            v.split(',').next().unwrap().parse().unwrap()
        };
        assert!(bw("1920x804") > bw("1280x536"), "{out}");
        assert!(bw("1280x536") > bw("640x268"));
        assert_eq!(bw("640x268"), 378404, "sane values stay");
        assert!(out.contains("AVERAGE-BANDWIDTH=300000"));
    }

    #[test]
    fn detects_playlists() {
        assert!(is_playlist(
            Some("application/vnd.apple.mpegurl"),
            "https://x/a"
        ));
        assert!(is_playlist(Some("audio/x-mpegURL"), "https://x/a"));
        assert!(is_playlist(None, "https://x/v/index.m3u8?x=1"));
        assert!(!is_playlist(Some("video/mp2t"), "https://x/v/seg.ts"));
    }
}
