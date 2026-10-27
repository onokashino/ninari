//! The title page's sections past "О тайтле": the order to watch the franchise in, what viewers
//! think of it, frames of its episodes, its songs and reviews.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Div, FontWeight, Hsla, Window, div, img, prelude::*, px, rgb};
use ninari_core::catalog::details::{Part, Viewers};
use ninari_core::catalog::extras::{Review, Theme};

use crate::format::kind_label;
use crate::scroll_row::{Row, spaced};
use crate::theme;

type OnId = Rc<dyn Fn(i64, &mut Window, &mut App)>;

fn block(heading: &str) -> Div {
    div().flex().flex_col().gap(px(16.)).min_w(px(0.)).child(
        div()
            .text_size(px(18.))
            .font_weight(FontWeight::EXTRA_BOLD)
            .child(heading.to_string()),
    )
}

fn muted(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .text_color(rgb(theme::TEXT_3))
        .child(text.into())
}

/// "13,7 тыс", "1,2 млн".
pub fn short_count(n: i64) -> String {
    let one = |v: f64, unit: &str| {
        let s = if v < 100. {
            format!("{v:.1}")
        } else {
            format!("{v:.0}")
        };
        format!("{} {unit}", s.trim_end_matches(".0").replace('.', ","))
    };
    match n {
        n if n >= 1_000_000 => one(n as f64 / 1e6, "млн"),
        n if n >= 1_000 => one(n as f64 / 1e3, "тыс"),
        n => n.to_string(),
    }
}

/// The franchise in the order it came out: the story's line first, the rest marked as aside.
/// Parts shown before "Ещё".
const ORDER_SHOWN: usize = 6;

pub fn watch_order(
    parts: &[Part],
    shown: i64,
    all: bool,
    accent: Hsla,
    on_part: OnId,
    on_more: Rc<dyn Fn(&mut Window, &mut App)>,
) -> Option<Div> {
    if parts.len() < 3 {
        return None;
    }
    let more = parts.len().saturating_sub(ORDER_SHOWN);
    let rows = parts
        .iter()
        .take(if all { parts.len() } else { ORDER_SHOWN })
        .enumerate()
        .map(|(i, p)| {
            let here = p.id == shown;
            let open = on_part.clone();
            let id = p.id;
            let facts = [
                Some(kind_label(Some(&p.kind))).filter(|k| !k.is_empty()),
                p.year.map(|y| y.to_string()),
                (p.episodes > 1).then(|| format!("{} эп.", p.episodes)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            div()
                .id(("order", i))
                .px(px(14.))
                .py(px(8.))
                .rounded(px(12.))
                .line_height(px(20.))
                .cursor_pointer()
                .when(here, |d| d.bg(accent.opacity(0.14)))
                .when(!here, |d| d.hover(|s| s.bg(theme::glass())))
                .on_click(move |_, window, cx| open(id, window, cx))
                // One text for the number, the name and the facts, coloured in parts: as nested
                // divs a row was six layout nodes.
                .child({
                    let number = format!("{}   ", i + 1);
                    let skip = if p.main_line {
                        ""
                    } else {
                        " · можно пропустить"
                    };
                    let text = format!("{number}{}\n{facts}{skip}", p.name);
                    let name_end = number.len() + p.name.len();
                    let muted = gpui::HighlightStyle {
                        color: Some(rgb(theme::TEXT_3).into()),
                        ..Default::default()
                    };
                    gpui::StyledText::new(text.clone()).with_highlights([
                        (
                            0..number.len(),
                            gpui::HighlightStyle {
                                color: Some(if here {
                                    accent
                                } else {
                                    rgb(theme::TEXT_3).into()
                                }),
                                font_weight: Some(FontWeight::EXTRA_BOLD),
                                ..Default::default()
                            },
                        ),
                        (
                            number.len()..name_end,
                            gpui::HighlightStyle {
                                color: Some(
                                    rgb(if p.main_line {
                                        theme::TEXT
                                    } else {
                                        theme::TEXT_2
                                    })
                                    .into(),
                                ),
                                font_weight: Some(FontWeight::BOLD),
                                ..Default::default()
                            },
                        ),
                        (name_end..text.len(), muted),
                    ])
                })
        });
    Some(
        block("Порядок просмотра").child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(px(8.))
                .rounded(theme::R_L)
                .bg(theme::glass())
                .border_1()
                .border_color(theme::line())
                .children(rows)
                .when(more > 0, |d| {
                    d.child(
                        div()
                            .id("order-more")
                            .px(px(14.))
                            .py(px(6.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent)
                            .hover(|s| s.underline())
                            .on_click(move |_, window, cx| on_more(window, cx))
                            .child(if all {
                                "Свернуть".to_string()
                            } else {
                                format!("Ещё {more}")
                            }),
                    )
                }),
        ),
    )
}

/// How viewers on Shikimori rated it and where it is in their lists.
pub fn viewers(v: &Viewers, accent: Hsla) -> Option<Div> {
    let votes = v.votes();
    if votes == 0 {
        return None;
    }
    // Ten bars are one painted element and the numbers two texts of a line each: as rows of
    // divs they were fifty layout nodes laid out on every frame of a scroll.
    const LINE: f32 = 18.;
    let top = v.scores.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f32;
    let shares: Vec<(f32, f32)> = v
        .scores
        .iter()
        .map(|(score, n)| (*n as f32 / top, *score as f32 / 10.))
        .collect();
    let rows = shares.len() as f32;
    let column = |lines: Vec<String>, w: f32| {
        div()
            .w(px(w))
            .flex_none()
            .text_size(px(12.))
            .line_height(px(LINE))
            .text_color(rgb(theme::TEXT_3))
            .child(lines.join("\n"))
    };
    let track = theme::glass();
    let bars = gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for (i, (fill_share, strength)) in shares.iter().enumerate() {
                let y = bounds.top() + px(LINE * i as f32 + (LINE - 8.) / 2.);
                let full = gpui::Bounds::new(
                    gpui::point(bounds.left(), y),
                    gpui::size(bounds.size.width, px(8.)),
                );
                window.paint_quad(gpui::fill(full, track).corner_radii(px(4.)));
                let part = gpui::Bounds::new(
                    full.origin,
                    gpui::size((bounds.size.width * *fill_share).max(px(8.)), px(8.)),
                );
                window.paint_quad(
                    gpui::fill(part, accent.opacity(0.35 + 0.65 * strength)).corner_radii(px(4.)),
                );
            }
        },
    )
    .flex_1()
    .h(px(LINE * rows));
    let histogram = div()
        .flex()
        .gap(px(10.))
        .child(column(
            v.scores.iter().map(|(s, _)| s.to_string()).collect(),
            18.,
        ))
        .child(bars)
        .child(column(
            v.scores
                .iter()
                .map(|(_, n)| format!("{:.0}%", *n as f32 / votes as f32 * 100.))
                .collect(),
            44.,
        ));
    let label = |s: &str| match s {
        "watching" => "Смотрят",
        "completed" => "Посмотрели",
        "planned" => "В планах",
        "on_hold" => "Отложили",
        "dropped" => "Бросили",
        _ => "",
    };
    // One text, the counts in bold: as a column per status it was fifteen layout nodes.
    let mut line = String::new();
    let mut bold = Vec::new();
    for (status, n) in v.statuses.iter().filter(|(s, _)| !label(s).is_empty()) {
        if !line.is_empty() {
            line.push_str("     ");
        }
        let count = short_count(*n).replace(' ', "\u{a0}");
        bold.push((
            line.len()..line.len() + count.len(),
            gpui::HighlightStyle {
                font_weight: Some(FontWeight::EXTRA_BOLD),
                color: Some(rgb(theme::TEXT).into()),
                ..Default::default()
            },
        ));
        // No break inside "4,7 тыс бросили".
        line.push_str(&format!("{count} {}", label(status).to_lowercase()).replace(' ', "\u{a0}"));
    }
    let statuses = div()
        .text_size(px(14.))
        .text_color(rgb(theme::TEXT_3))
        .child(gpui::StyledText::new(line).with_highlights(bold));
    Some(
        block("Зрители").child(
            div()
                .flex()
                .flex_col()
                .gap(px(18.))
                .px(px(24.))
                .py(px(18.))
                .rounded(theme::R_L)
                .bg(theme::glass())
                .border_1()
                .border_color(theme::line())
                .child(muted(format!("{} оценок на Shikimori", short_count(votes))))
                .child(histogram)
                .child(statuses),
        ),
    )
}

/// Frames of the episodes in a row; a click opens one over the whole window.
pub fn shots(
    list: &[(PathBuf, String)],
    row: &Row,
    on_open: Rc<dyn Fn(usize, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Div> {
    if list.is_empty() {
        return None;
    }
    const W: f32 = 260.;
    const GAP: f32 = 12.;
    let shown = row.visible(list.len(), W, GAP, window);
    let items = list[shown.clone()]
        .iter()
        .enumerate()
        .map(|(i, (thumb, _))| {
            let open = on_open.clone();
            let index = shown.start + i;
            div()
                .id(("shot", shown.start + i))
                .flex_none()
                .w(px(W))
                .h(px(W * 9. / 16.))
                .rounded(theme::R_M)
                .overflow_hidden()
                .cursor_pointer()
                .bg(rgb(theme::SURFACE_2))
                .border_1()
                .border_color(theme::line())
                .hover(|s| s.border_color(rgb(theme::TEXT_3)))
                .on_click(move |_, window, cx| open(index, window, cx))
                .child(
                    img(thumb.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover),
                )
                .into_any_element()
        })
        .collect();
    let items = spaced(shown, list.len(), W, GAP, items);
    let arrow = |id: &'static str, name: &'static str, dir: f32| {
        let row = row.clone();
        div()
            .id(id)
            .size(px(34.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(theme::glass())
            .hover(|s| s.bg(theme::glass_strong()))
            .on_click(move |_, window, _| {
                let page = f32::from(window.viewport_size().width) * 0.65;
                row.glide_by(dir * page, window);
            })
            .child(crate::assets::icon(name, 18.).text_color(rgb(theme::TEXT)))
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .min_w(px(0.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(18.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Кадры"),
                    )
                    .child(arrow("shots-back", "back", -1.))
                    .child(arrow("shots-on", "chevron", 1.)),
            )
            .child(row.render("shots", px(GAP), (4., 20., 10.), items, window, cx)),
    )
}

/// Openings and endings; a click looks the song up on YouTube.
pub fn themes(list: &[Theme], accent: Hsla) -> Option<Div> {
    if list.is_empty() {
        return None;
    }
    let column = |opening: bool, heading: &'static str| {
        let songs: Vec<&Theme> = list.iter().filter(|t| t.opening == opening).collect();
        (!songs.is_empty()).then(|| {
            div()
                .flex_1()
                .min_w(px(260.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(muted(heading))
                .children(songs.into_iter().enumerate().map(move |(i, t)| {
                    let query = format!("{} {}", t.artist.clone().unwrap_or_default(), t.title);
                    div()
                        .id((if opening { "op" } else { "ed" }, i))
                        .flex()
                        .flex_col()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::glass()))
                        .on_click(move |_, _, cx| {
                            let mut url = url::Url::parse("https://www.youtube.com/results")
                                .expect("static url");
                            url.query_pairs_mut().append_pair("search_query", &query);
                            cx.open_url(url.as_str());
                        })
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .hover(move |s| s.text_color(accent))
                                .child(t.title.clone()),
                        )
                        .child(muted(
                            [t.artist.clone(), t.episodes.clone()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · "),
                        ))
                }))
        })
    };
    Some(
        block("Музыка").child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(24.))
                .children(column(true, "Опенинги"))
                .children(column(false, "Эндинги")),
        ),
    )
}

/// The first reviews from Shikimori, a click opens one there.
pub fn reviews(
    list: &[Review],
    all: String,
    open: Option<usize>,
    on_toggle: Rc<dyn Fn(usize, &mut Window, &mut App)>,
    accent: Hsla,
) -> Option<Div> {
    if list.is_empty() {
        return None;
    }
    let cards = list.iter().take(4).enumerate().map(|(i, r)| {
        let (label, color) = match r.opinion.as_str() {
            "positive" => ("Положительный", theme::OK),
            "negative" => ("Отрицательный", theme::DANGER),
            _ => ("Нейтральный", theme::TEXT_3),
        };
        let url = r.url.clone();
        let opened = open == Some(i);
        let toggle = on_toggle.clone();
        div()
            .id(("review", i))
            .map(|d| {
                if opened {
                    d.w_full()
                } else {
                    d.flex_1().min_w(px(320.))
                }
            })
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(20.))
            .py(px(16.))
            .rounded(theme::R_L)
            .bg(theme::glass())
            .border_1()
            .border_color(theme::line())
            .cursor_pointer()
            .hover(|s| s.border_color(rgb(theme::TEXT_3)))
            .on_click(move |_, window, cx| toggle(i, window, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(div().font_weight(FontWeight::BOLD).child(r.author.clone()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(color))
                            .child(label),
                    )
                    .child(muted(r.date.clone()))
                    .child(muted(format!("+{} / -{}", r.votes_for, r.votes_against))),
            )
            .map(|d| {
                if opened {
                    // The whole text only for the one open: paragraphs as the author wrote them.
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .text_size(px(14.))
                            .line_height(px(22.))
                            .text_color(rgb(theme::TEXT))
                            .children(
                                r.full
                                    .split('\n')
                                    .filter(|p| !p.trim().is_empty())
                                    .map(|p| div().child(p.to_string())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(16.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::BOLD)
                            .child(div().text_color(accent).child("Свернуть"))
                            .child(
                                div()
                                    .id(("review-site", i))
                                    .text_color(rgb(theme::TEXT_3))
                                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        cx.open_url(&url);
                                    })
                                    .child("Обсуждение на Shikimori"),
                            ),
                    )
                } else {
                    d.child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(rgb(theme::TEXT_2))
                            .line_clamp(6)
                            .text_ellipsis()
                            .child(r.text.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent)
                            .child("Читать целиком"),
                    )
                }
            })
    });
    Some(
        block("Отзывы")
            .child(div().flex().flex_wrap().gap(px(16.)).children(cards))
            .child(
                div()
                    .id("reviews-all")
                    .cursor_pointer()
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(accent)
                    .hover(|s| s.underline())
                    .on_click(move |_, _, cx| cx.open_url(&all))
                    .child("Все отзывы на Shikimori"),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_short() {
        assert_eq!(short_count(950), "950");
        assert_eq!(short_count(13_692), "13,7 тыс");
        assert_eq!(short_count(106_098), "106 тыс");
        assert_eq!(short_count(1_200_000), "1,2 млн");
        assert_eq!(short_count(2_000), "2 тыс");
    }
}
