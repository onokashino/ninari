//! A poster card's details beside it on a longer look, as Shikimori shows them: next to the card
//! rather than at the pointer, staying while the pointer moves into them, with the title's place
//! in the library to see and change.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, BoxShadow, ElementId, FontWeight, Global, Pixels, Window, anchored,
    deferred, div, point, prelude::*, px, rgb, rgba,
};
use ninari_core::feeds::AnimeCard;

use crate::core::Core;
use crate::data::title;
use crate::format::{kind_label, library_status};
use crate::theme;

const SHOW_AFTER: Duration = Duration::from_millis(350);
/// Room to move from the card into the details without them going.
const HIDE_AFTER: Duration = Duration::from_millis(200);
const WIDTH: f32 = 340.;
const STATUSES: [&str; 5] = ["watching", "planned", "completed", "on_hold", "dropped"];

#[derive(Default)]
struct HoverCard {
    shown: Option<Shown>,
    /// The card under the pointer, if any.
    over_card: Option<ElementId>,
    over_details: bool,
    /// Since when the pointer is over neither the card nor the details.
    away: Option<Instant>,
}

struct Shown {
    key: ElementId,
    /// Given by the card itself once it is the one under the pointer: cards do not carry a copy
    /// of themselves on every frame for the one that may be hovered.
    card: Option<AnimeCard>,
    since: Instant,
    /// The card on the last frame.
    bounds: Option<Bounds<Pixels>>,
    /// The title's status in the library, read once shown; `Some(None)` when not in it.
    status: Option<Option<String>>,
}

impl Global for HoverCard {}

/// The pointer entered or left card `key`.
pub fn hover(key: &ElementId, on: bool, cx: &mut App) {
    let state = cx.default_global::<HoverCard>();
    if on {
        if state.shown.as_ref().is_none_or(|s| &s.key != key) {
            state.shown = Some(Shown {
                key: key.clone(),
                card: None,
                since: Instant::now(),
                bounds: None,
                status: None,
            });
        }
        state.over_card = Some(key.clone());
        later(SHOW_AFTER, cx, |_| {});
    } else {
        if state.over_card.as_ref() == Some(key) {
            state.over_card = None;
        }
        hide_later(cx);
    }
}

/// Takes the details away at once: the page scrolled, so the card is no longer under the pointer
/// though the pointer did not move (the next move finds the card under it again).
pub fn hide(cx: &mut App) {
    if let Some(state) = cx.try_global::<HoverCard>()
        && (state.shown.is_some() || state.over_card.is_some())
    {
        let state = cx.default_global::<HoverCard>();
        state.shown = None;
        state.over_card = None;
        state.over_details = false;
    }
}

fn hide_later(cx: &mut App) {
    later(HIDE_AFTER, cx, |state| {
        if state.over_card.is_none() && !state.over_details {
            state.shown = None;
        }
    });
}

/// Runs `then` on the state after `wait`, and redraws.
fn later(wait: Duration, cx: &mut App, then: impl FnOnce(&mut HoverCard) + 'static) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        cx.update(|cx| {
            then(cx.default_global::<HoverCard>());
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Gives the details the card's data once it is the one under the pointer: cards do not carry a
/// copy of themselves on every frame for the one that may be hovered.
pub fn provide(key: &ElementId, card: &AnimeCard, cx: &mut App) {
    let Some(shown) = cx.try_global::<HoverCard>().and_then(|s| s.shown.as_ref()) else {
        return;
    };
    if &shown.key == key
        && shown.card.is_none()
        && let Some(s) = cx.default_global::<HoverCard>().shown.as_mut()
    {
        s.card = Some(card.clone());
    }
}

/// Where card `key` is this frame, kept while its details are up.
pub fn place(key: &ElementId, bounds: Bounds<Pixels>, cx: &mut App) {
    if let Some(s) = cx
        .try_global::<HoverCard>()
        .and_then(|s| s.shown.as_ref())
        .filter(|s| &s.key == key)
        .map(|_| ())
        .and(cx.default_global::<HoverCard>().shown.as_mut())
    {
        s.bounds = Some(bounds);
    }
}

/// For cards made of divs: an empty layer over the card that tells the details where it is.
pub fn anchor(key: &ElementId, cx: &App) -> Option<AnyElement> {
    let shown = cx.try_global::<HoverCard>()?.shown.as_ref()?;
    if &shown.key != key {
        return None;
    }
    let key = key.clone();
    Some(
        gpui::canvas(
            move |bounds, _, cx| place(&key, bounds, cx),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element(),
    )
}

/// What happens from the details: open the title, or note that the library changed.
pub struct Actions {
    pub open: Rc<dyn Fn(i64, &mut Window, &mut App)>,
    pub changed: Rc<dyn Fn(&mut Window, &mut App)>,
}

/// The details of the card the pointer rests on, beside it; Root draws them over everything.
pub fn overlay(actions: Actions, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (bounds, since, over_details) = {
        let state = cx.try_global::<HoverCard>()?;
        let shown = state.shown.as_ref()?;
        (shown.bounds?, shown.since, state.over_details)
    };
    if since.elapsed() < SHOW_AFTER {
        return None;
    }
    // The card can move or go from under a still pointer (a season switch, a list rebuilt), and
    // then no "left" comes: what decides is where the pointer is now.
    let on_card = bounds.contains(&window.mouse_position());
    let state = cx.default_global::<HoverCard>();
    if on_card || over_details {
        state.away = None;
    } else {
        match state.away {
            Some(t) if t.elapsed() >= HIDE_AFTER => {
                state.shown = None;
                state.over_card = None;
                state.away = None;
                return None;
            }
            Some(_) => {}
            None => {
                state.away = Some(Instant::now());
                // A frame after the grace, to check again even if nothing else moves.
                later(HIDE_AFTER, cx, |_| {});
            }
        }
    }
    let id = cx
        .try_global::<HoverCard>()?
        .shown
        .as_ref()?
        .card
        .as_ref()?
        .anime
        .id;
    let unread = cx
        .try_global::<HoverCard>()?
        .shown
        .as_ref()?
        .status
        .is_none();
    if unread {
        let status = Core::get(cx)
            .library
            .get_entry(id)
            .ok()
            .flatten()
            .map(|e| e.status)
            .filter(|s| STATUSES.contains(&s.as_str()));
        if let Some(s) = cx.default_global::<HoverCard>().shown.as_mut() {
            s.status = Some(status);
        }
    }
    let shown = cx.try_global::<HoverCard>()?.shown.as_ref()?;
    let (card, status) = (shown.card.clone()?, shown.status.clone().flatten());
    let details = details(&card, status, actions, cx);
    // Beside the card where there is room on its right, else on its left.
    let room_right = window.viewport_size().width - bounds.right() > px(WIDTH + 24.);
    let x = if room_right {
        bounds.right() + px(12.)
    } else {
        bounds.left() - px(WIDTH + 12.)
    };
    Some(
        deferred(
            anchored()
                .position(point(x, bounds.top()))
                .snap_to_window_with_margin(px(12.))
                .child(details),
        )
        .with_priority(3)
        .into_any_element(),
    )
}

fn details(
    card: &AnimeCard,
    status: Option<String>,
    actions: Actions,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let a = &card.anime;
    let accent = theme::Accent::colors(cx).0;
    let id = a.id;
    let episodes = if a.episodes > 0 {
        a.episodes
    } else {
        a.episodes_aired
    };
    let fact = |label: &'static str, value: String| {
        div()
            .flex()
            .gap(px(8.))
            .child(div().text_color(rgb(theme::TEXT_3)).child(label))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_color(rgb(theme::TEXT))
                    .child(value),
            )
    };
    let kind = [
        Some(kind_label(a.kind.as_deref())).filter(|k| !k.is_empty()),
        a.year.map(|y| y.to_string()),
        match a.status.as_deref() {
            Some("ongoing") => Some("онгоинг".to_string()),
            Some("anons") => Some("анонс".to_string()),
            Some("released") => Some("вышел".to_string()),
            _ => None,
        },
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let open = actions.open.clone();
    let statuses = STATUSES.map(|st| {
        let on = status.as_deref() == Some(st);
        let changed = actions.changed.clone();
        div()
            .id((
                "hover-status",
                st.len() as u64 * 7 + st.as_bytes()[0] as u64,
            ))
            .cursor_pointer()
            .px(px(10.))
            .py(px(5.))
            .rounded_full()
            .text_size(px(12.))
            .font_weight(FontWeight::BOLD)
            .map(|d| {
                if on {
                    d.bg(accent).text_color(rgb(theme::ACCENT_CONTRAST))
                } else {
                    d.bg(theme::glass())
                        .text_color(rgb(theme::TEXT_2))
                        .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
                }
            })
            .on_click(move |_, window, cx| {
                // A second click on the current status takes the title out of the library.
                let set = (!on).then_some(st);
                set_status(id, set, changed.clone(), window, cx);
            })
            .child(library_status(st))
    });
    div()
        .id("hover-details")
        .occlude()
        .w(px(WIDTH))
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .rounded(theme::R_L)
        .bg(rgba(0x17171ffc))
        .border_1()
        .border_color(theme::line())
        .shadow(vec![BoxShadow {
            color: rgba(0x00000099).into(),
            offset: point(px(0.), px(16.)),
            blur_radius: px(40.),
            spread_radius: px(0.),
            inset: false,
        }])
        .text_size(px(13.))
        .text_color(rgb(theme::TEXT_2))
        .on_hover(|on, _, cx| {
            cx.default_global::<HoverCard>().over_details = *on;
            if !*on {
                hide_later(cx);
            }
        })
        .child(
            div()
                .id("hover-title")
                .cursor_pointer()
                .text_size(px(16.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(rgb(theme::TEXT))
                .hover(move |s| s.text_color(accent))
                .on_click(move |_, window, cx| {
                    cx.default_global::<HoverCard>().shown = None;
                    open(id, window, cx);
                })
                .child(title(a)),
        )
        .children(
            Some(a.name.clone())
                .filter(|n| Some(n) != a.russian.as_ref())
                .map(|n| {
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_3))
                        .child(n)
                }),
        )
        .children(a.description.clone().map(|text| {
            div()
                .line_height(px(18.))
                .line_clamp(4)
                .text_ellipsis()
                .child(text)
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .mt(px(4.))
                .when(!kind.is_empty(), |d| d.child(fact("Тип", kind)))
                .when(episodes > 0, |d| {
                    d.child(fact("Эпизоды", episodes.to_string()))
                })
                .when(!a.studios.is_empty(), |d| {
                    d.child(fact("Студия", a.studios.join(", ")))
                })
                .when(!a.genres.is_empty(), |d| {
                    d.child(fact("Жанры", a.genres.join(", ")))
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .mt(px(6.))
                .children(statuses),
        )
        .children(a.score.map(|score| {
            div()
                .flex()
                .items_baseline()
                .justify_end()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_3))
                        .child("оценка"),
                )
                .child(
                    div()
                        .text_size(px(22.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(rgb(theme::TEXT))
                        .child(format!("{score:.2}").replace('.', ",")),
                )
        }))
}

/// Writes the title's status, or takes it out of the library with `None`, then tells the pages.
fn set_status(
    id: i64,
    status: Option<&'static str>,
    changed: Rc<dyn Fn(&mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(s) = cx.default_global::<HoverCard>().shown.as_mut() {
        s.status = Some(status.map(str::to_string));
    }
    let job = Core::get(cx).run(move |c| async move {
        match status {
            Some(st) => c.library.set_status(id, st)?,
            None => c.library.remove(id)?,
        }
        // What changed goes to the list on Shikimori too, when signed in.
        c.account.push(id);
        Ok(())
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        if let Err(e) = job.await {
            tracing::warn!("library: {e:#}");
        }
        handle.update(cx, |_, window, cx| changed(window, cx)).ok();
    })
    .detach();
    window.refresh();
}
