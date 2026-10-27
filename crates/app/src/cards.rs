//! Title cards: the poster in its frame, the card in a row and today's episode.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, Div, ElementId, FontWeight, Hsla, ObjectFit, Pixels, Window, div,
    img, prelude::*, px, rgb,
};
use ninari_core::feeds::{AnimeCard, ScheduleItem};

use crate::OnClick;
use crate::card_element::PosterCard;
use crate::clock::local_time;
use crate::data::title;
use crate::format::{kind_label, library_status};
use crate::hover_card;
use crate::motion::{self, rise};
use crate::posters::{missing, poster_path};
use crate::theme::{self, EASE};

pub const POSTER_W: f32 = 150.;

/// The poster in a frame of `w`: a downloaded one fades in, the first letter stands in when
/// the title has none, and a thin edge lies over the picture.
pub fn poster_frame(c: &AnimeCard, w: Pixels, radius: Pixels, cx: &mut App) -> Div {
    let frame = div()
        .relative()
        .flex_none()
        .w(w)
        .h(w * 1.5)
        .rounded(radius)
        .overflow_hidden()
        .bg(rgb(theme::SURFACE_2));
    let frame = match poster_path(c, cx) {
        Some(path) => {
            let picture = img(path)
                .size_full()
                .rounded(radius)
                .object_fit(ObjectFit::Cover);
            if c.poster_path.is_some() {
                frame.child(picture)
            } else {
                frame.child(picture.with_animation(
                    ("poster-in", c.anime.id as u64),
                    Animation::new(Duration::from_millis(420)).with_easing(theme::ease),
                    |picture, t| picture.opacity(t),
                ))
            }
        }
        None if missing(c, cx) => frame.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(40.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(rgb(theme::TEXT_3))
                .child(title(&c.anime).chars().next().unwrap_or(' ').to_string()),
        ),
        None => frame,
    };
    frame.child(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(radius)
            .border_1()
            .border_color(theme::line()),
    )
}

#[derive(Default)]
pub struct CardText {
    /// Replaces the kind and year under the title.
    pub note: Option<String>,
    /// How much of the episode is watched, a bar over the poster.
    pub fraction: Option<f64>,
    /// A short mark on the poster, like "+2".
    pub badge: Option<String>,
}

/// A poster card in a row. `list` names the row, so the same title in two rows lifts alone.
pub fn poster_card(
    c: &AnimeCard,
    list: &'static str,
    index: usize,
    text: CardText,
    accent: Hsla,
    on_click: OnClick,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    poster_card_sized(c, list, index, text, POSTER_W, accent, on_click, window, cx)
}

/// The same card `width` wide, for grids that stretch their columns. One element that paints
/// itself (card_element): pages hold a hundred of these.
pub fn poster_card_sized(
    c: &AnimeCard,
    list: &'static str,
    index: usize,
    text: CardText,
    width: f32,
    accent: Hsla,
    on_click: OnClick,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let key = ElementId::NamedInteger(list.into(), c.anime.id as u64);
    let lift = motion::get(key.clone(), window, cx);
    hover_card::provide(&key, c, cx);
    let meta: String = match text.note {
        Some(note) => note,
        None => [
            Some(kind_label(c.anime.kind.as_deref())).filter(|k| !k.is_empty()),
            c.anime.year.map(|y| y.to_string()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("   "),
    };
    let poster = poster_path(c, cx);
    let initial = (poster.is_none() && missing(c, cx)).then(|| {
        title(&c.anime)
            .chars()
            .next()
            .unwrap_or(' ')
            .to_string()
            .into()
    });
    let card = PosterCard {
        key,
        width,
        height: crate::lazy_grid::row_height(width) - 12.,
        lift,
        accent,
        poster,
        initial,
        title: title(&c.anime).into(),
        meta: meta.into(),
        score: c
            .anime
            .score
            .map(|s| format!("★ {}", format!("{s:.1}").replace('.', ",")).into()),
        badge: text.badge.map(Into::into),
        fraction: text.fraction.map(|f| f as f32),
        on_click: Some(on_click),
    };
    rise(card, ("rise", c.anime.id as u64), index, 560, 38, 14.)
}

/// One of today's episodes: the time, the poster, the title and the user's status.
pub fn today_card(
    it: &ScheduleItem,
    at: i64,
    now: i64,
    index: usize,
    accent: Hsla,
    on_click: OnClick,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let key = ElementId::NamedInteger(
        "today".into(),
        (it.card.anime.id as u64) << 16 | it.episode as u64,
    );
    let hover = motion::get(key.clone(), window, cx);
    let aired = at <= now;
    let card = div()
        .id(key.clone())
        .group("today")
        .cursor_pointer()
        .on_click(on_click)
        .on_hover(move |hovered, window, cx| {
            motion::set(key.clone(), *hovered, Duration::from_millis(260), EASE, cx);
            window.refresh();
        })
        .w(px(260.))
        .flex_none()
        .relative()
        .top(px(-2. * hover))
        .flex()
        .items_center()
        .gap(px(12.))
        .pl(px(10.))
        .pr(px(14.))
        .py(px(10.))
        .rounded(theme::R_L)
        .bg(theme::glass())
        .hover(|s| s.bg(theme::glass_strong()))
        .border_1()
        .border_color(theme::line())
        .child(poster_frame(&it.card, px(58.), px(10.), cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w(px(0.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .text_size(px(18.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .when(aired, |d| d.text_color(rgb(theme::TEXT_2)))
                        .child(local_time(at))
                        .when(aired, |d| {
                            d.child(
                                div()
                                    .text_color(rgb(theme::TEXT_3))
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("вышла"),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(17.))
                        .font_weight(FontWeight::BOLD)
                        .line_clamp(2)
                        .text_ellipsis()
                        .group_hover("today", move |s| {
                            s.text_color(theme::mix(rgb(theme::TEXT), accent, 0.45))
                        })
                        .child(title(&it.card.anime)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_color(rgb(theme::TEXT_3))
                        .text_size(px(12.))
                        .child(format!("Серия {}", it.episode))
                        .children(it.status.as_deref().map(library_status).map(|s| {
                            div()
                                .px(px(8.))
                                .py(px(1.))
                                .rounded_full()
                                .bg(accent.opacity(0.22))
                                .text_color(theme::mix(rgb(theme::TEXT), accent, 0.8))
                                .font_weight(FontWeight::BOLD)
                                .child(s)
                        })),
                ),
        );
    rise(
        card,
        ("today-rise", it.card.anime.id as u64),
        index,
        460,
        32,
        8.,
    )
}
