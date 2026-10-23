//! DASH manifest rewriting. Segment URLs in an MPD are templates (`$Number$`,
//! `$RepresentationID$`) the player fills in, so they cannot be encoded one by one like HLS
//! URIs. Every base folder becomes a proxy prefix instead, and the player appends the filled-in
//! template to it.

pub use url::Url;

pub fn is_dash(content_type: Option<&str>, url: &str) -> bool {
    content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("dash+xml"))
        || Url::parse(url)
            .map(|u| u.path().to_ascii_lowercase().ends_with(".mpd"))
            .unwrap_or(false)
}

const SEGMENT_ATTRS: [&str; 3] = ["media", "initialization", "sourceURL"];

/// `dir` turns an upstream folder (or file, for single-file representations) into the proxy
/// prefix that serves it.
pub fn rewrite_mpd(body: &str, manifest: &Url, dir: &dyn Fn(&Url) -> String) -> String {
    let period = body.find("<Period").unwrap_or(body.len());
    let top_value = base_urls(body)
        .into_iter()
        .find(|b| b.start < period)
        .map(|b| b.value);
    // Relative segments resolve against the top base, or the manifest's own folder.
    let top = top_value
        .as_deref()
        .and_then(|v| manifest.join(v).ok())
        .unwrap_or_else(|| manifest.join("./").unwrap_or_else(|_| manifest.clone()));

    let mut out = String::with_capacity(body.len() + 256);
    let mut last = 0;
    for b in base_urls(body) {
        let parent = if b.start < period { manifest } else { &top };
        if let Some(abs) = parent.join(&b.value).ok().filter(is_web) {
            out.push_str(&body[last..b.value_start]);
            out.push_str(&dir(&abs));
            last = b.value_end;
        }
    }
    out.push_str(&body[last..]);
    let mut out = rewrite_absolute_templates(&out, dir);

    if top_value.is_none() {
        if let Some(open_end) = out
            .find("<MPD")
            .and_then(|i| out[i..].find('>').map(|j| i + j + 1))
        {
            out.insert_str(open_end, &format!("<BaseURL>{}</BaseURL>", dir(&top)));
        }
    }
    out
}

fn is_web(u: &Url) -> bool {
    matches!(u.scheme(), "http" | "https")
}

struct BaseUrl {
    start: usize,
    value_start: usize,
    value_end: usize,
    value: String,
}

fn base_urls(body: &str) -> Vec<BaseUrl> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = body[from..].find("<BaseURL") {
        let start = from + i;
        let Some(open_end) = body[start..].find('>').map(|j| start + j + 1) else {
            break;
        };
        let Some(close) = body[open_end..].find("</BaseURL>").map(|j| open_end + j) else {
            break;
        };
        found.push(BaseUrl {
            start,
            value_start: open_end,
            value_end: close,
            value: body[open_end..close].trim().replace("&amp;", "&"),
        });
        from = close;
    }
    found
}

/// Absolute segment templates: the folder before the first `$` goes through the proxy, the
/// templated rest stays for the player to fill in.
fn rewrite_absolute_templates(body: &str, dir: &dyn Fn(&Url) -> String) -> String {
    let mut out = body.to_string();
    for attr in SEGMENT_ATTRS {
        let needle = format!("{attr}=\"");
        let mut from = 0;
        while let Some(i) = out[from..].find(&needle) {
            let value_start = from + i + needle.len();
            let Some(len) = out[value_start..].find('"') else {
                break;
            };
            let value = out[value_start..value_start + len].replace("&amp;", "&");
            let cut = value.find('$').unwrap_or(value.len());
            let folder_end = value[..cut].rfind('/').map_or(0, |j| j + 1);
            let replaced = Url::parse(&value[..folder_end])
                .ok()
                .filter(is_web)
                .map(|folder| format!("{}{}", dir(&folder), &value[folder_end..]));
            match replaced {
                Some(r) => {
                    out.replace_range(value_start..value_start + len, &r);
                    from = value_start + r.len();
                }
                None => from = value_start + len,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <BaseURL>video/</BaseURL>
      <SegmentTemplate media="seg-$Number$.m4s" initialization="init.mp4" startNumber="1"/>
      <Representation id="1080" bandwidth="5000000"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4">
      <SegmentTemplate media="https://cdn2.example/a/$RepresentationID$/$Number$.m4s" initialization="https://cdn2.example/a/$RepresentationID$/init.mp4"/>
      <Representation id="aac" bandwidth="128000"/>
    </AdaptationSet>
  </Period>
</MPD>"#;

    fn dir(u: &Url) -> String {
        format!("D[{u}]")
    }

    #[test]
    fn dash_segments_go_through_the_proxy() {
        let base = Url::parse("https://cdn.example/dash/manifest.mpd?t=1").unwrap();
        let out = rewrite_mpd(MPD, &base, &dir);
        assert!(
            out.contains("<BaseURL>D[https://cdn.example/dash/]</BaseURL>"),
            "the manifest folder becomes the base of relative segments: {out}"
        );
        assert!(
            out.contains("<BaseURL>D[https://cdn.example/dash/video/]</BaseURL>"),
            "{out}"
        );
        assert!(
            out.contains(r#"media="D[https://cdn2.example/a/]$RepresentationID$/$Number$.m4s""#),
            "{out}"
        );
        assert!(
            out.contains(
                r#"initialization="D[https://cdn2.example/a/]$RepresentationID$/init.mp4""#
            ),
            "{out}"
        );
        assert!(
            out.contains(r#"media="seg-$Number$.m4s""#),
            "relative templates stay relative"
        );
    }

    #[test]
    fn a_top_level_base_url_is_rewritten_not_duplicated() {
        let mpd = "<MPD><BaseURL>https://cdn3.example/x/</BaseURL><Period><AdaptationSet><BaseURL>v/</BaseURL></AdaptationSet></Period></MPD>";
        let out = rewrite_mpd(mpd, &Url::parse("https://cdn.example/m.mpd").unwrap(), &dir);
        assert_eq!(out.matches("<BaseURL>").count(), 2, "{out}");
        assert!(
            out.contains("<BaseURL>D[https://cdn3.example/x/]</BaseURL>"),
            "{out}"
        );
        assert!(
            out.contains("<BaseURL>D[https://cdn3.example/x/v/]</BaseURL>"),
            "nested bases resolve against the top one: {out}"
        );
    }

    #[test]
    fn detects_dash_manifests() {
        assert!(is_dash(Some("application/dash+xml"), "https://x/a"));
        assert!(is_dash(None, "https://x/v/manifest.mpd?x=1"));
        assert!(!is_dash(Some("video/mp4"), "https://x/v/seg.m4s"));
    }
}
