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
        let s = if v < 10. {
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
pub fn watch_order(parts: &[Part], shown: i64, accent: Hsla, on_part: OnId) -> Option<Div> {
    if parts.len() < 3 {
        return None;
    }
    let rows = parts.iter().enumerate().map(|(i, p)| {
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
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(9.))
            .rounded(px(12.))
            .cursor_pointer()
            .when(here, |d| d.bg(accent.opacity(0.14)))
            .when(!here, |d| d.hover(|s| s.bg(theme::glass())))
            .on_click(move |_, window, cx| open(id, window, cx))
            .child(
                div()
                    .w(px(26.))
                    .flex_none()
                    .text_color(if here {
                        accent
                    } else {
                        rgb(theme::TEXT_3).into()
                    })
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child((i + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(if p.main_line {
                                theme::TEXT
                            } else {
                                theme::TEXT_2
                            }))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(p.name.clone()),
                    )
                    .child(muted(facts)),
            )
            .when(!p.main_line, |d| d.child(muted("можно пропустить")))
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
                .children(rows),
        ),
    )
}

/// How viewers on Shikimori rated it and where it is in their lists.
pub fn viewers(v: &Viewers, accent: Hsla) -> Option<Div> {
    let votes = v.votes();
    if votes == 0 {
        return None;
    }
    let top = v.scores.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f32;
    let bars = v.scores.iter().map(|(score, n)| {
        let share = *n as f32 / votes as f32;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .w(px(18.))
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_3))
                    .child(score.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(8.))
                    .rounded_full()
                    .bg(theme::glass())
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(*n as f32 / top))
                            .rounded_full()
                            .bg(accent.opacity(0.35 + 0.65 * (*score as f32 / 10.))),
                    ),
            )
            .child(
                div()
                    .w(px(44.))
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_3))
                    .child(format!("{:.0}%", share * 100.)),
            )
    });
    let label = |s: &str| match s {
        "watching" => "Смотрят",
        "completed" => "Посмотрели",
        "planned" => "В планах",
        "on_hold" => "Отложили",
        "dropped" => "Бросили",
        _ => "",
    };
    let statuses = v
        .statuses
        .iter()
        .filter(|(s, _)| !label(s).is_empty())
        .map(|(s, n)| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(short_count(*n)),
                )
                .child(muted(label(s)))
        });
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
                .child(div().flex().flex_col().gap(px(6.)).children(bars))
                .child(div().flex().flex_wrap().gap(px(28.)).children(statuses)),
        ),
    )
}

/// Frames of the episodes in a row; a click opens one over the whole window.
pub fn shots(
    list: &[(PathBuf, String)],
    row: &Row,
    on_open: Rc<dyn Fn(String, PathBuf, &mut Window, &mut App)>,
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
        .map(|(i, (thumb, full))| {
            let open = on_open.clone();
            let (full, thumb_path) = (full.clone(), thumb.clone());
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
                .on_click(move |_, window, cx| open(full.clone(), thumb_path.clone(), window, cx))
                .child(
                    img(thumb.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover),
                )
                .into_any_element()
        })
        .collect();
    let items = spaced(shown, list.len(), W, GAP, items);
    Some(block("Кадры").child(row.render("shots", px(GAP), (4., 20., 10.), items, window, cx)))
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
pub fn reviews(list: &[Review], all: String, accent: Hsla) -> Option<Div> {
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
        div()
            .id(("review", i))
            .flex_1()
            .min_w(px(320.))
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
            .on_click(move |_, _, cx| cx.open_url(&url))
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
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(rgb(theme::TEXT_2))
                    .line_clamp(6)
                    .text_ellipsis()
                    .child(r.text.clone()),
            )
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
