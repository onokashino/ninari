//! The franchise's parts as cards with their posters and how much of each was watched; the
//! one on screen raised in the accent.

use std::rc::Rc;

use gpui::{App, Div, FontWeight, Hsla, Window, div, prelude::*, px, rgb, rgba};
use ninari_core::feeds::AnimeCard;

use crate::cards::poster_frame;
use crate::franchise::PartChip;
use crate::motion;
use crate::theme;

pub type OnPart = Rc<dyn Fn(i64, &mut Window, &mut App)>;

const CARD_W: f32 = 118.;
const PAD: f32 = 6.;

/// A part's card data: its chip, its catalog card when in, and watched / episodes when it is
/// in the library.
pub struct PartCard {
    pub chip: PartChip,
    pub card: Option<AnimeCard>,
    pub watched: Option<(i64, i64)>,
}

pub fn season_cards(
    parts: &[PartCard],
    accent: Hsla,
    on_part: OnPart,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let inner = CARD_W - PAD * 2.;
    let cards: Vec<_> = parts
        .iter()
        .map(|p| {
            let (id, current) = (p.chip.id, p.chip.current);
            // The raise eases, so moving the selection lowers one card as it lifts the other.
            let key = ("part-on", id as u64);
            motion::set(key, current, theme::T_SLOW / 2, theme::EASE, cx);
            let on = motion::get(key, window, cx);
            let watched = p
                .watched
                .filter(|(_, total)| *total > 0)
                .map(|(w, total)| (w.min(total), total));
            let poster = match &p.card {
                Some(c) => poster_frame(c, px(inner), px(8.), cx),
                None => div()
                    .w(px(inner))
                    .h(px(inner * 1.5))
                    .rounded(px(8.))
                    .bg(theme::glass()),
            };
            let on_part = on_part.clone();
            div()
                .id(("part", id as u64))
                .relative()
                .top(px(-3. * on))
                .w(px(CARD_W))
                .p(px(PAD))
                .flex()
                .flex_col()
                .gap(px(2.))
                .rounded(px(12.))
                .border_1()
                .bg(theme::mix(theme::glass(), accent.opacity(0.16), on))
                .border_color(theme::mix(theme::line(), accent.opacity(0.5), on))
                .opacity(0.72 + 0.28 * on)
                .when(!current, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.opacity(1.).top(px(-2.)).bg(theme::glass_strong()))
                        .on_click(move |_, window, cx| on_part(id, window, cx))
                })
                .child(poster)
                .when(!p.chip.season, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(PAD + 6.))
                            .left(px(PAD + 6.))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(6.))
                            .bg(rgba(0x000000aa))
                            .text_size(px(10.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(p.chip.label.clone()),
                    )
                })
                .child(
                    div()
                        .mt(px(6.))
                        .px(px(2.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(if p.chip.season {
                            p.chip.label.clone()
                        } else {
                            p.chip.name.clone()
                        }),
                )
                .child(
                    div()
                        .px(px(2.))
                        .text_size(px(11.))
                        .text_color(rgb(theme::TEXT_3))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(p.chip.sub.clone()),
                )
                .children(watched.map(|(w, total)| {
                    // The bar's share of the row, beside "5/12".
                    const BAR: f32 = 64.;
                    div()
                        .mt(px(4.))
                        .px(px(2.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(BAR))
                                .h(px(3.))
                                .rounded_full()
                                .bg(theme::glass_strong())
                                .child(
                                    div()
                                        .h_full()
                                        .w(px(BAR * w as f32 / total as f32))
                                        .rounded_full()
                                        .bg(accent),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::TEXT_2))
                                .child(format!("{w}/{total}")),
                        )
                }))
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .child("Сезоны"),
        )
        .child(div().flex().flex_wrap().gap(px(10.)).children(cards))
}
