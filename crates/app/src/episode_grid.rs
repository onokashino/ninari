//! The episode tiles: blocks of a hundred for long titles, the one to continue breathing in the
//! accent, watched ones tinted, fillers and recaps marked.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnchoredPositionMode, Animation, AnimationExt, App, BoxShadow, Div, FontWeight, Hsla, Pixels,
    Window, anchored, deferred, div, point, prelude::*, px, rgb, rgba,
};
use ninari_core::catalog::jikan::EpisodeMark;

use crate::assets::icon;
use crate::episodes::{Tile, TileState, ranges, tile_hint};
use crate::motion;
use crate::theme::{self, EASE};

const TILE_MIN: f32 = 56.;
const GAP: f32 = 8.;

pub type OnNumber = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// A small chip, like the block picker's.
pub fn chip(id: impl Into<gpui::ElementId>, selected: bool, accent: Hsla) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(14.))
        .py(px(7.))
        .rounded_full()
        .border_1()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .map(|d| {
            if selected {
                d.bg(accent.opacity(0.2))
                    .border_color(accent.opacity(0.65))
                    .text_color(rgb(theme::TEXT))
            } else {
                d.bg(theme::glass())
                    .border_color(theme::line())
                    .text_color(rgb(theme::TEXT_2))
                    .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
            }
        })
}

/// `block` is the hundred on screen (the one with the episode to continue when `None`);
/// `width` is the room the grid has; `enter` the direction and the ms since a season switch
/// brought this grid in, for the tiles' wave.
pub fn episode_grid(
    tiles: &[Tile],
    marks: &[EpisodeMark],
    block: Option<usize>,
    width: Pixels,
    accent: Hsla,
    enter: Option<(f32, u64)>,
    on_play: OnNumber,
    on_block: OnNumber,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let blocks = ranges(tiles.len() as i64);
    let current = tiles
        .iter()
        .find(|t| t.state == TileState::Current)
        .map_or(1, |t| t.n);
    let block = block.unwrap_or_else(|| {
        blocks
            .iter()
            .position(|(from, to)| (*from..=*to).contains(&current))
            .unwrap_or(0)
    });
    let visible = match blocks.get(block) {
        Some((from, to)) => &tiles[(*from - 1) as usize..*to as usize],
        None => tiles,
    };
    let room = f32::from(width);
    let columns = ((room + GAP) / (TILE_MIN + GAP)).floor().max(1.);
    let tile_w = (room - GAP * (columns - 1.)) / columns;
    let marked = marks.iter().any(|m| m.filler || m.recap);

    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .when(!blocks.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(blocks.iter().enumerate().map(|(i, (from, to))| {
                        let pick = on_block.clone();
                        chip(("block", i), i == block, accent)
                            .on_click(move |_, window, cx| pick(i as i64, window, cx))
                            .child(format!("{from}-{to}"))
                    })),
            )
        })
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(GAP))
                .children(visible.iter().enumerate().map(|(i, t)| {
                    let wave = enter.map(|(dir, ms)| (dir, crate::season_switch::tile_t(ms, i)));
                    tile(t, marks, tile_w, accent, wave, on_play.clone(), window, cx)
                })),
        )
        .when(marked, |d| {
            let dot = |color: u32| div().size(px(6.)).rounded_full().bg(rgb(color));
            d.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .text_color(rgb(theme::TEXT_3))
                    .text_size(px(13.))
                    .child(dot(theme::WARN))
                    .child("филлер")
                    .child(dot(theme::TEXT_3))
                    .child("пересказ, по данным MyAnimeList"),
            )
        })
}

/// `wave` is the direction of a season switch and how far in this tile is.
fn tile(
    t: &Tile,
    marks: &[EpisodeMark],
    width: f32,
    accent: Hsla,
    wave: Option<(f32, f32)>,
    on_play: OnNumber,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let n = t.n;
    let open = !matches!(t.state, TileState::Missing | TileState::Upcoming);
    let key = ("tile", n as u64);
    let lift = if open {
        motion::get(key, window, cx)
    } else {
        0.
    };
    let found = marks.iter().find(|m| m.number == n);
    let hint = tile_hint(n, found);
    let mark = found.and_then(|m| match (m.filler, m.recap) {
        (true, _) => Some(theme::WARN),
        (_, true) => Some(theme::TEXT_3),
        _ => None,
    });
    let el = div()
        .id(key)
        .relative()
        .top(px(-2. * lift))
        .when_some(wave, |d, (dir, t)| {
            let e = EASE.at(t);
            d.opacity(e).left(px(dir * 14. * (1. - e)))
        })
        .w(px(width))
        .h(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(12.))
        .overflow_hidden()
        .border_1()
        .font_weight(FontWeight::BOLD)
        .map(|d| match t.state {
            TileState::Watched => d
                .bg(accent.opacity(0.12))
                .border_color(rgba(0))
                .text_color(theme::mix(rgb(theme::TEXT_3), accent, 0.7)),
            TileState::Current => d
                .bg(accent.opacity(0.22))
                .border_2()
                .border_color(accent)
                .text_color(rgb(theme::TEXT)),
            TileState::Available => d
                .bg(theme::glass())
                .border_color(theme::line())
                .text_color(rgb(theme::TEXT)),
            TileState::Missing => d.border_color(theme::line()).text_color(rgb(theme::TEXT_3)),
            TileState::Upcoming => d
                .border_color(rgba(0xffffff0a))
                .text_color(rgb(theme::TEXT_3))
                .opacity(0.5),
        })
        .when(open, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(theme::glass_strong()))
                .on_hover(move |hovered, window, cx| {
                    motion::set(key, *hovered, theme::T_FAST, EASE, cx);
                    window.refresh();
                })
                .on_click(move |_, window, cx| on_play(n, window, cx))
        })
        .child(n.to_string())
        .when(t.fraction > 0. && t.fraction < 1., |d| {
            // Inset and rounded: GPUI clips to the tile's box, not its rounded corners.
            d.child(
                div()
                    .absolute()
                    .left(px(6.))
                    .bottom(px(5.))
                    .h(px(3.))
                    .w(px((width - 12.) * t.fraction as f32))
                    .rounded_full()
                    .bg(accent),
            )
        })
        .children(mark.map(|color| {
            div()
                .absolute()
                .top(px(6.))
                .left(px(6.))
                .size(px(6.))
                .rounded_full()
                .bg(rgb(color))
        }))
        .when(t.state == TileState::Watched, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(4.))
                    .right(px(5.))
                    .child(icon("check", 11.).text_color(accent)),
            )
        });
    // MyAnimeList's name of the episode over the tile while it is hovered.
    let popup = hint.filter(|_| lift > 0.01).map(|hint| {
        // Local positions count from where the anchor sits, so it sits at the tile's corner.
        div().absolute().top_0().left_0().child(deferred(
            anchored()
                .position_mode(AnchoredPositionMode::Local)
                .position(point(px(0.), px(-46.)))
                .child(
                    div()
                        .opacity(lift)
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(rgb(theme::SURFACE_2))
                        .border_1()
                        .border_color(theme::line())
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .whitespace_nowrap()
                        .child(hint),
                ),
        ))
    });
    let el = if t.state == TileState::Current {
        // The tile to continue breathes: a glow in the accent swells and fades.
        el.with_animation(
            ("breathe", n as u64),
            // A slow glow needs few frames; the window is not redrawn at full rate for it.
            Animation::new(Duration::from_millis(2600))
                .repeat()
                .with_max_fps(30.),
            move |el, t| {
                let swell = (t * std::f32::consts::PI).sin();
                el.shadow(vec![BoxShadow {
                    color: accent.opacity(0.45 * swell),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(18.),
                    spread_radius: px(0.),
                    inset: false,
                }])
            },
        )
        .into_any_element()
    } else {
        el.into_any_element()
    };
    div()
        .relative()
        .child(el)
        .children(popup)
        .into_any_element()
}
