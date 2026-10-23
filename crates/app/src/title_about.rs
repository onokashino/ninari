//! The lower half of a title's page: "О тайтле" with the main characters and the videos.

use ninari_core::catalog::details::{HeroView, TitleDetails, Video};
use ninari_core::feeds::AnimeCard;
use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, BoxShadow, Div, FontWeight, Hsla, ObjectFit, Pixels, Window, canvas, div, img,
    point, prelude::*, px, rgb, rgba, white,
};

use crate::assets::icon;
use crate::episode_grid::OnNumber;
use crate::facts::{
    age_label, aired_range, origin_label, rating_label, season_label, status_label,
};
use crate::format::{kind_label, plural};
use crate::motion;
use crate::scroll_row::Row;
use crate::theme::{self, EASE};
use crate::thumbs::youtube_thumb;

/// The rows shown before "Ещё", by label, in this order of preference.
const MAIN: [&str; 5] = ["Выход", "Студия", "Первоисточник", "Рейтинг", "Режиссёр"];

/// Indices of the rows shown at once: the first five of `MAIN` the title has, in the
/// table's order; the rest go under "Ещё".
fn main_rows(labels: &[&str]) -> Vec<usize> {
    let mut picked: Vec<usize> = MAIN
        .iter()
        .filter_map(|m| labels.iter().position(|l| l == m))
        .take(5)
        .collect();
    picked.sort();
    picked
}

/// "Ещё" under the facts: how far it is open (0..1, eased by the caller), the hidden rows'
/// height as last measured, and what a click does.
pub struct More {
    pub open: f32,
    pub opened: bool,
    pub height: Rc<Cell<Pixels>>,
    pub toggle: OnToggle,
}

pub type OnToggle = Rc<dyn Fn(&mut Window, &mut App)>;

/// A titled block: a heading over its content.
fn block(heading: &str) -> Div {
    div().flex().flex_col().gap(px(16.)).min_w(px(0.)).child(
        div()
            .text_size(px(18.))
            .font_weight(FontWeight::EXTRA_BOLD)
            .child(heading.to_string()),
    )
}

/// The facts table and the main characters, side by side when there is room for both.
/// `genres` and `studios` are their links.
pub fn about(
    card: &AnimeCard,
    details: Option<&TitleDetails>,
    width: Pixels,
    genres: Option<AnyElement>,
    studios: Option<AnyElement>,
    accent: Hsla,
    more: More,
    on_hero: OnNumber,
) -> Div {
    let a = &card.anime;
    let episodes = if a.status.as_deref() == Some("ongoing") {
        let total = if a.episodes > 0 {
            a.episodes.to_string()
        } else {
            "?".into()
        };
        Some(format!("{} из {total}", a.episodes_aired))
    } else if a.episodes > 0 {
        Some(format!(
            "{} {}",
            a.episodes,
            plural(a.episodes, "серия", "серии", "серий")
        ))
    } else {
        None
    };
    let duration = details.and_then(|d| d.duration).filter(|d| *d > 0);
    let rating = details.and_then(|d| d.rating.as_deref());
    let mut rows: Vec<(String, AnyElement)> = Vec::new();
    let mut text = |label: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            rows.push((label.into(), div().child(v).into_any_element()));
        }
    };
    text("Тип", Some(kind_label(a.kind.as_deref())));
    text(
        "Эпизоды",
        episodes.map(|e| match duration {
            Some(d) => format!("{e}, по {d} мин"),
            None => e,
        }),
    );
    text(
        "Статус",
        status_label(a.status.as_deref()).map(|(s, _)| s.to_string()),
    );
    text(
        "Сезон",
        details
            .and_then(|d| d.season.as_deref())
            .and_then(season_label),
    );
    text(
        "Выход",
        details.and_then(|d| {
            aired_range(
                d.aired_on.as_deref(),
                d.released_on.as_deref(),
                a.status.as_deref(),
            )
        }),
    );
    text(
        "Первоисточник",
        details
            .and_then(|d| d.origin.as_deref())
            .and_then(origin_label)
            .map(Into::into),
    );
    if let Some(name) = rating.and_then(rating_label) {
        rows.push((
            "Рейтинг".into(),
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(name)
                .children(rating.and_then(age_label).map(|age| {
                    div()
                        .px(px(7.))
                        .py(px(1.))
                        .rounded(px(6.))
                        .bg(theme::glass_strong())
                        .text_size(px(12.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(age)
                }))
                .into_any_element(),
        ));
    }
    let mut text = |label: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            rows.push((label.into(), div().child(v).into_any_element()));
        }
    };
    text(
        "Оценка",
        details
            .and_then(|d| d.score)
            .filter(|s| *s > 0.)
            .map(|s| format!("{s:.2}").replace('.', ",")),
    );
    if let Some(g) = genres {
        rows.push(("Жанры".into(), g));
    }
    if let Some(s) = studios {
        rows.push(("Студия".into(), s));
    }
    for staff in details.map_or(&[][..], |d| &d.staff[..]) {
        rows.push((
            staff.role.clone(),
            div().child(staff.names.join(", ")).into_any_element(),
        ));
    }

    let labels: Vec<&str> = rows.iter().map(|(l, _)| l.as_str()).collect();
    let main = main_rows(&labels);
    let row = |label: String, value: AnyElement| {
        div()
            .flex()
            .gap(px(24.))
            .child(
                div()
                    .w(px(140.))
                    .flex_none()
                    .text_color(rgb(theme::TEXT_3))
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value),
            )
    };
    let (mut shown, mut hidden) = (Vec::new(), Vec::new());
    for (i, (label, value)) in rows.into_iter().enumerate() {
        if main.contains(&i) {
            shown.push(row(label, value));
        } else {
            hidden.push(row(label, value));
        }
    }
    // One paragraph, as the page's <p> collapsed the line breaks.
    let description = a
        .description
        .as_deref()
        .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "));
    let rest = hidden.len();
    let measured = more.height.clone();
    let toggle = more.toggle.clone();
    let facts = block("О тайтле").child(
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(24.))
            .py(px(16.))
            .rounded(theme::R_L)
            .bg(theme::glass())
            .border_1()
            .border_color(theme::line())
            .text_size(px(13.))
            .children(description.map(|d| {
                div()
                    .mb(px(6.))
                    .text_size(px(14.))
                    .line_height(px(22.))
                    .text_color(rgb(theme::TEXT_2))
                    .child(d)
            }))
            .children(shown)
            .when(rest > 0, |d| {
                // The hidden rows keep their height inside a box that eases from none to it.
                d.child(
                    div()
                        .mt(px(-10.))
                        .h(more.height.get() * more.open)
                        .overflow_hidden()
                        .child(
                            div()
                                .flex_none()
                                .relative()
                                .flex()
                                .flex_col()
                                .gap(px(10.))
                                .pt(px(10.))
                                .children(hidden)
                                .child(
                                    canvas(
                                        move |b, _, _| measured.set(b.size.height),
                                        |_, _, _, _| {},
                                    )
                                    .absolute()
                                    .size_full(),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("about-more")
                        .cursor_pointer()
                        .mt(px(2.))
                        .text_color(accent)
                        .font_weight(FontWeight::BOLD)
                        .hover(|s| s.underline())
                        .on_click(move |_, window, cx| toggle(window, cx))
                        .child(if more.opened {
                            "Свернуть".to_string()
                        } else {
                            format!("Ещё {rest}")
                        }),
                )
            }),
    );

    // Two columns from 340px each, as `repeat(auto-fit, minmax(340px, 1fr))` with a 32px gap.
    let side_by_side = width >= px(340. * 2. + 32.);
    let column = if side_by_side {
        (width - px(32.)) / 2.
    } else {
        width
    };
    let heroes = match details {
        Some(d) if d.heroes.is_empty() => None,
        Some(d) => Some(heroes(&d.heroes, column, on_hero)),
        None => Some(
            block("Главные герои").child(div().flex().gap(px(12.)).children((0..3).map(|_| {
                div()
                    .flex_1()
                    .h(px(72.))
                    .rounded(px(16.))
                    .bg(theme::glass())
            }))),
        ),
    };
    div()
        .flex()
        .when(!side_by_side, |d| d.flex_col())
        .items_start()
        .gap(px(32.))
        .child(facts.flex_1())
        .children(heroes.map(|h| h.flex_1()))
}

fn portrait(path: Option<&str>, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(theme::SURFACE_2))
        .border_1()
        .border_color(theme::line())
        .children(path.map(|p| {
            img(std::path::PathBuf::from(p))
                .size_full()
                .aspect_square()
                .rounded_full()
                .object_fit(ObjectFit::Cover)
        }))
}

/// A click on a hero opens the character's page.
fn heroes(list: &[HeroView], width: Pixels, on_hero: OnNumber) -> Div {
    // Cards from 230px, as `repeat(auto-fill, minmax(230px, 1fr))` with a 12px gap.
    let columns = ((f32::from(width) + 12.) / 242.).floor().max(1.);
    let card_w = (f32::from(width) - 12. * (columns - 1.)) / columns;
    block("Главные герои").child(div().flex().flex_wrap().gap(px(12.)).children(
        list.iter().enumerate().map(|(i, h)| {
            div()
                .id(("hero", i))
                .relative()
                .when_some(h.id, |d, id| {
                    let open = on_hero.clone();
                    d.cursor_pointer()
                        .hover(|s| s.bg(theme::glass_strong()).top(px(-2.)))
                        .on_click(move |_, window, cx| open(id, window, cx))
                })
                .w(px(card_w))
                .flex()
                .items_center()
                .gap(px(12.))
                .pl(px(10.))
                .pr(px(14.))
                .py(px(10.))
                .rounded(theme::R_L)
                .bg(theme::glass())
                .border_1()
                .border_color(theme::line())
                .child(portrait(h.image.as_deref(), 56.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .min_w(px(0.))
                        .child(div().font_weight(FontWeight::BOLD).child(h.name.clone()))
                        .children(h.voice.clone().map(|voice| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_color(rgb(theme::TEXT_3))
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(portrait(h.voice_image.as_deref(), 20.))
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(voice),
                                )
                        })),
                )
        }),
    ))
}

/// Trailers, then openings and endings, each a row of thumbnails; a click opens the video on
/// YouTube. `rows` keeps the two rows' scroll.
pub fn videos(
    list: &[Video],
    rows: &[Row; 2],
    accent: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> Option<Div> {
    if list.is_empty() {
        return None;
    }
    let mut section = block("Видео");
    for (i, (group, label)) in [("trailer", "Трейлеры"), ("music", "Опенинги и эндинги")]
        .into_iter()
        .enumerate()
    {
        let items: Vec<&Video> = list.iter().filter(|v| v.group == group).collect();
        if items.is_empty() {
            continue;
        }
        let cards: Vec<AnyElement> = items
            .iter()
            .map(|v| video(v, accent, window, cx).into_any_element())
            .collect();
        section = section.child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .text_color(rgb(theme::TEXT_2))
                        .text_size(px(13.))
                        .font_weight(FontWeight::BOLD)
                        .child(label),
                )
                .child(rows[i].render(("videos", i), px(16.), (6., 20., 14.), cards, window, cx)),
        );
    }
    Some(section)
}

fn video(v: &Video, accent: Hsla, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    let key = gpui::ElementId::Name(format!("video-{}", v.youtube).into());
    let lift = motion::get(key.clone(), window, cx);
    let url = format!("https://www.youtube.com/watch?v={}", v.youtube);
    let thumb = youtube_thumb(&v.youtube, cx);
    div()
        .id(key.clone())
        .group("video")
        .cursor_pointer()
        .w(px(240.))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .on_hover(move |hovered, window, cx| {
            motion::set(
                key.clone(),
                *hovered,
                std::time::Duration::from_millis(260),
                EASE,
                cx,
            );
            window.refresh();
        })
        .on_click(move |_, _, cx| cx.open_url(&url))
        .child(
            div()
                .relative()
                .top(px(-3. * lift))
                .w(px(240.))
                .h(px(135.))
                .rounded(theme::R_M)
                .overflow_hidden()
                .bg(rgb(theme::SURFACE_2))
                .border_1()
                .border_color(theme::line())
                .children(thumb.map(|p| {
                    img(p)
                        .size_full()
                        .rounded(theme::R_M)
                        .object_fit(ObjectFit::Cover)
                }))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(44.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgba(0x101018b3))
                                .opacity(0.85)
                                .group_hover("video", |s| s.opacity(1.))
                                .shadow(vec![BoxShadow {
                                    color: accent.opacity(0.35 * lift.clamp(0., 1.)),
                                    offset: point(px(0.), px(0.)),
                                    blur_radius: px(18.),
                                    spread_radius: px(0.),
                                    inset: false,
                                }])
                                .child(icon("play", 20.).text_color(white())),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .text_ellipsis()
                .child(v.name.clone()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_main_rows_and_the_rest_under_more() {
        let labels = [
            "Тип",
            "Эпизоды",
            "Выход",
            "Первоисточник",
            "Рейтинг",
            "Студия",
            "Режиссёр",
            "Музыка",
        ];
        assert_eq!(main_rows(&labels), [2, 3, 4, 5, 6]);
        assert_eq!(main_rows(&["Тип", "Студия"]), [1]);
    }
}
