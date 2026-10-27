//! What the title page shows past the catalog's data, read from the sites' pages (neither has it
//! in an API): reviews from Shikimori, opening and ending songs and episode synopses from
//! MyAnimeList.

use serde::Serialize;

/// A review on Shikimori.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Review {
    pub author: String,
    /// "positive", "neutral" or "negative".
    pub opinion: String,
    /// "3 октября 2026", as the page writes it.
    pub date: String,
    pub votes_for: i64,
    pub votes_against: i64,
    pub text: String,
    pub url: String,
}

/// An opening or ending song.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Theme {
    pub opening: bool,
    pub title: String,
    pub artist: Option<String>,
    /// "eps 1-14", when the page says.
    pub episodes: Option<String>,
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)?;
    Some(&s[i..i + j])
}

/// Text of an HTML fragment: line breaks kept, tags dropped, entities read.
fn plain(html: &str) -> String {
    let with_breaks = html
        .replace("<br>", "\n")
        .replace("<br />", "\n")
        .replace("<br/>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in with_breaks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    out.lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Reviews from a Shikimori title's reviews page.
pub fn parse_reviews(html: &str) -> Vec<Review> {
    html.split("b-review-topic")
        .skip(1)
        .filter_map(|a| {
            let author = between(a, "data-user_nickname=\"", "\"")?.to_string();
            let url = between(a, "data-url=\"", "\"")?.to_string();
            let opinion = ["positive", "neutral", "negative"]
                .into_iter()
                .find(|o| a.contains(&format!("class=\"opinion {o}\"")))
                .unwrap_or("neutral")
                .to_string();
            let date = between(a, "<time", "</time>")
                .and_then(|t| t.split('>').nth(1))
                .unwrap_or_default()
                .trim()
                .to_string();
            let votes = |class: &str| {
                between(a, &format!("class=\"{class}\">"), "<")
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(0)
            };
            let body = between(a, "class=\"body\" itemprop=\"text\">", "</div>")?;
            Some(Review {
                author: plain(&author),
                opinion,
                date,
                votes_for: votes("votes-for"),
                votes_against: votes("votes-against"),
                text: plain(body),
                url,
            })
        })
        .collect()
}

/// Opening and ending songs from a MyAnimeList title page.
pub fn parse_themes(html: &str) -> Vec<Theme> {
    let mut out = Vec::new();
    for (opening, marker) in [
        (true, "js-theme-songs opnening"),
        (false, "js-theme-songs ending"),
    ] {
        let Some(start) = html.find(marker) else {
            continue;
        };
        // The section ends where the next one (or the page's next block) begins.
        let section = &html[start..];
        let end = section[1..]
            .find("theme-songs js-theme-songs")
            .map_or(section.len().min(20_000), |e| e + 1);
        for cell in section[..end].split("<td width=\"84%\">").skip(1) {
            let cell = cell.split("</td>").next().unwrap_or_default();
            let cell = cell.split("<input").next().unwrap_or_default();
            let text = plain(cell);
            // `1: "Title" by Artist (eps 1-12)`, the number optional.
            let text = text
                .split_once(": ")
                .filter(|(n, _)| n.len() <= 4)
                .map_or(text.as_str(), |(_, rest)| rest)
                .trim();
            let Some(title) = between(text, "\"", "\"") else {
                continue;
            };
            let rest = text
                .split_once(&format!("\"{title}\""))
                .map_or("", |(_, r)| r);
            // "by Yorushika (ヨルシカ) (eps 17-28)": the episodes are the bracket that says so.
            let (artist, episodes) = match rest.rfind("(eps") {
                Some(i) => (
                    &rest[..i],
                    between(&rest[i..], "(", ")").map(|e| e.trim().to_string()),
                ),
                None => (rest, None),
            };
            let artist = artist
                .trim()
                .strip_prefix("by ")
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty());
            out.push(Theme {
                opening,
                title: title.trim().to_string(),
                artist,
                episodes,
            });
        }
    }
    out
}

/// The synopsis on a MyAnimeList episode page, without its "(Source: …)" line.
pub fn parse_synopsis(html: &str) -> Option<String> {
    let block = between(html, "<h2 class=\"fw-b\">Synopsis</h2>", "</div>")?;
    let text = plain(block);
    let text = text
        .lines()
        .filter(|l| !l.starts_with("(Source") && !l.starts_with("(Written"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    (!text.is_empty() && !text.contains("No synopsis")).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_is_read_from_its_article() {
        let html = r#"<article class="b-topic b-review-topic is-review"
    data-url="https://shikimori.io/animes/52991-x/reviews/140423"
    data-user_id="1" data-user_nickname="пакост" id="646564">
    <div class="b-status-line"><span class="votes-for">11</span><span>&nbsp;/&nbsp;</span><span class="votes-against">23</span>
    <time data-no-tooltip="true" datetime="2026-10-03">3 октября 2026</time></div>
    <div class="review-info"><div class="opinion neutral"></div></div>
    <div><div class="body" itemprop="text">Красиво, но <br>пусто.<br><strong>Итог:</strong> смотреть можно.</div><footer></footer></div>
    </article>"#;
        let r = parse_reviews(html);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].author, "пакост");
        assert_eq!(r[0].opinion, "neutral");
        assert_eq!(r[0].date, "3 октября 2026");
        assert_eq!((r[0].votes_for, r[0].votes_against), (11, 23));
        assert_eq!(r[0].text, "Красиво, но\nпусто.\nИтог: смотреть можно.");
    }

    #[test]
    fn songs_are_read_with_artist_and_episodes() {
        let html = r#"<div class="theme-songs js-theme-songs opnening"><table><tr><td width="8%"></td>
<td width="84%"><span class="theme-song-index">1:</span>&nbsp;"Yuusha"<span class="theme-song-artist"> by YOASOBI</span>&nbsp;<span class="theme-song-episode">(eps 1-16)</span><input type="hidden" value="" /></td></tr></table></div>
<div class="theme-songs js-theme-songs ending"><table><tr><td width="8%"></td>
<td width="84%">&quot;bliss&quot; by milet (eps Special: 1-4)<input type="hidden" /></td></tr></table></div>"#;
        assert_eq!(
            parse_themes(html),
            vec![
                Theme {
                    opening: true,
                    title: "Yuusha".into(),
                    artist: Some("YOASOBI".into()),
                    episodes: Some("eps 1-16".into())
                },
                Theme {
                    opening: false,
                    title: "bliss".into(),
                    artist: Some("milet".into()),
                    episodes: Some("eps Special: 1-4".into())
                },
            ]
        );
    }

    #[test]
    fn a_synopsis_drops_its_source_line() {
        let html = r#"<div class="pt8 pb8"> <h2 class="fw-b">Synopsis</h2> After defeating the Demon King, they return.<br /> <br /> (Source: Official site) </div>"#;
        assert_eq!(
            parse_synopsis(html).as_deref(),
            Some("After defeating the Demon King, they return.")
        );
        assert_eq!(
            parse_synopsis("<h2 class=\"fw-b\">Synopsis</h2> No synopsis yet.</div>"),
            None
        );
    }
}
