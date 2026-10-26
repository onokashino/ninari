//! Title cards: the poster in its frame, the card in a row and today's episode.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, BoxShadow, Div, ElementId, FontWeight, Hsla, ObjectFit, Pixels,
    Window, div, img, point, prelude::*, px, rgb, rgba,
};
use ninari_core::feeds::{AnimeCard, ScheduleItem};

use crate::OnClick;
use crate::clock::local_time;
use crate::data::title;
use crate::format::{kind_label, library_status};
use crate::motion::{self, rise};
use crate::posters::{missing, poster};
use crate::theme::{self, EASE, SPRING};

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
    let frame = match poster(c, cx) {
        Some((path, _)) => {
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

/// The same card `width` wide, for grids that stretch their columns.
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
    let meta: Vec<String> = match text.note {
        Some(note) => vec![note],
        None => [
            Some(kind_label(c.anime.kind.as_deref())).filter(|k| !k.is_empty()),
            c.anime.year.map(|y| y.to_string()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    let frame = poster_frame(c, px(width), theme::R_M, cx)
        .top(px(-6. * lift))
        // Only on a lifted card: a blurred shadow costs the GPU even when it is invisible, and a
        // page has a hundred cards.
        .when(lift > 0.01, |f| {
            f.shadow(vec![BoxShadow {
                color: accent.opacity(0.6 * lift.clamp(0., 1.)),
                offset: point(px(0.), px(16.)),
                blur_radius: px(32.),
                spread_radius: px(-14.),
                inset: false,
            }])
        })
        .children(text.badge.map(|b| {
            div()
                .absolute()
                .top(px(8.))
                .left(px(8.))
                .px(px(9.))
                .py(px(3.))
                .rounded_full()
                .bg(accent)
                .text_color(rgb(theme::ACCENT_CONTRAST))
                .text_size(px(12.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .shadow(vec![BoxShadow {
                    color: rgba(0x00000059).into(),
                    offset: point(px(0.), px(4.)),
                    blur_radius: px(14.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .child(b)
        }))
        // Shikimori's score in the corner, as on the title's page.
        .children(c.anime.score.map(|score| {
            div()
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(7.))
                .py(px(2.))
                .rounded_full()
                .bg(rgba(0x0c0c12cc))
                .text_size(px(12.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(rgb(theme::TEXT))
                .child(div().text_color(rgb(0xf5c451)).child("★"))
                .child(format!("{score:.1}").replace('.', ","))
        }))
        .children(text.fraction.filter(|f| *f > 0.).map(|f| {
            div()
                .absolute()
                .left(px(8.))
                .bottom(px(8.))
                .h(px(4.))
                .w(px((f.min(1.) as f32 * width).min(width - 16.)))
                .rounded_full()
                .bg(accent)
                .shadow(vec![BoxShadow {
                    color: rgba(0x00000059).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
        }));
    let card = div()
        .id(key.clone())
        .tooltip({
            let anime = c.anime.clone();
            move |_, cx| cx.new(|_| CardTip(anime.clone())).into()
        })
        .tooltip_show_delay(Duration::from_millis(650))
        .group("card")
        .cursor_pointer()
        .on_click(on_click)
        .on_hover(move |hovered, window, cx| {
            motion::set(
                key.clone(),
                *hovered,
                Duration::from_millis(380),
                SPRING,
                cx,
            );
            window.refresh();
        })
        .w(px(width))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(frame)
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(17.))
                .font_weight(FontWeight::BOLD)
                .line_clamp(3)
                .text_ellipsis()
                .group_hover("card", move |s| {
                    s.text_color(theme::mix(rgb(theme::TEXT), accent, 0.45))
                })
                .child(title(&c.anime)),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .mt(px(-4.))
                .text_color(rgb(theme::TEXT_3))
                .text_size(px(12.))
                // A long note ("после «…»") stays within the card.
                .children(
                    meta.into_iter()
                        .map(|m| div().min_w(px(0.)).line_clamp(2).text_ellipsis().child(m)),
                ),
        );
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

/// What a poster card tells on a longer look: the score, the kind, year and length, the genres
/// and the start of the description.
struct CardTip(ninari_core::catalog::Anime);

impl Render for CardTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let a = &self.0;
        let episodes = if a.episodes > 0 {
            a.episodes
        } else {
            a.episodes_aired
        };
        let facts: Vec<String> = [
            a.score
                .map(|s| format!("★ {}", format!("{s:.1}").replace('.', ","))),
            Some(kind_label(a.kind.as_deref())).filter(|k| !k.is_empty()),
            a.year.map(|y| y.to_string()),
            (episodes > 1).then(|| format!("{episodes} эп.")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let original = Some(a.name.clone()).filter(|n| Some(n) != a.russian.as_ref());
        div()
            .w(px(320.))
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .rounded(theme::R_M)
            .bg(rgba(0x16161ef5))
            .border_1()
            .border_color(theme::line())
            .shadow(vec![BoxShadow {
                color: rgba(0x00000080).into(),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
                inset: false,
            }])
            .text_size(px(13.))
            .text_color(rgb(theme::TEXT_2))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(rgb(theme::TEXT))
                    .child(title(a)),
            )
            .children(original.map(|n| {
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_3))
                    .child(n)
            }))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(facts.join("  ·  ")),
            )
            .when(!a.genres.is_empty(), |d| {
                d.child(div().text_size(px(12.)).child(a.genres.join(", ")))
            })
            .children(a.description.clone().map(|text| {
                div()
                    .mt(px(4.))
                    .line_height(px(18.))
                    .line_clamp(5)
                    .text_ellipsis()
                    .child(text)
            }))
    }
}
