//! A horizontal row the mouse can drag and fling, like a touch screen. A press that barely moves
//! stays a click; the click that ends a drag opens nothing. The wheel scrolls the page, a
//! touchpad's sideways swipe the row. The right edge fades out while there is more to scroll to;
//! the left one does not, as a fade to the page's colour shows as a dark band over its glow.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, App, ElementId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollHandle, Window, canvas, div, linear_color_stop, linear_gradient, point, prelude::*, px,
    rgb, rgba,
};

use crate::motion;
use crate::theme;

/// A press that moves less than this is a click, not a drag.
const DRAG_THRESHOLD: f32 = 6.;
/// Only the moves of this last stretch before letting go set the speed of the glide.
const SPEED_WINDOW_MS: f32 = 100.;
/// How much speed a glide keeps every 16 ms.
const FRICTION: f32 = 0.94;
const STOP_SPEED: f32 = 0.05;
const FADE: f32 = 64.;

#[derive(Default)]
struct Drag {
    /// Where the press started and how far the row was scrolled then.
    press: Option<(Pixels, Pixels)>,
    dragging: bool,
    samples: Vec<(Pixels, Instant)>,
    /// Speed in px per ms towards the end of the row, and when it last moved the row.
    glide: Option<(f32, Instant)>,
}

/// One row's scroll and drag, kept by its page so the row is where it was left.
#[derive(Clone, Default)]
pub struct Row {
    scroll: ScrollHandle,
    drag: Rc<RefCell<Drag>>,
}

/// Scrolls `scroll` so its left edge is `left` px into the row, within the row.
fn scroll_to(scroll: &ScrollHandle, left: Pixels) {
    let max = scroll.max_offset().x;
    let y = scroll.offset().y;
    scroll.set_offset(point(-left.clamp(px(0.), max), y));
}

impl Row {
    /// Which of `count` items `width` wide, `gap` apart, lie in or next to the row's visible
    /// part. Only those are worth building: give them to `render` through `spaced`.
    pub fn visible(&self, count: usize, width: f32, gap: f32, window: &Window) -> Range<usize> {
        let left = f32::from(-self.scroll.offset().x);
        let screen = f32::from(window.viewport_size().width);
        visible_items(left, screen, count, width + gap)
    }

    /// Glides the row about `distance` px on (back when negative), as a flick would.
    pub fn glide_by(&self, distance: f32, window: &mut Window) {
        // A glide at v px/ms covers 16 v / (1 - FRICTION) before it stops.
        let speed = distance * (1. - FRICTION) / 16.;
        self.drag.borrow_mut().glide = Some((speed, Instant::now()));
        window.refresh();
    }

    /// Moves a glide on by the time since the last frame.
    fn step_glide(&self, window: &mut Window) {
        let mut drag = self.drag.borrow_mut();
        let Some((speed, last)) = drag.glide else {
            return;
        };
        let now = Instant::now();
        let ms = (now - last).as_secs_f32() * 1000.;
        let left = -self.scroll.offset().x;
        scroll_to(&self.scroll, left + px(speed * ms));
        let speed = speed * FRICTION.powf(ms / 16.);
        let stuck = -self.scroll.offset().x == left && ms > 0.;
        if speed.abs() <= STOP_SPEED || stuck {
            drag.glide = None;
        } else {
            drag.glide = Some((speed, now));
            window.request_animation_frame();
        }
    }

    /// The row with `children` laid out `gap` apart. `room` is space around the cards (top,
    /// sides, bottom) for a hovered card's lift and glow, given back by negative margins so
    /// the row stays aligned with the page.
    pub fn render(
        &self,
        id: impl Into<ElementId>,
        gap: Pixels,
        room: (f32, f32, f32),
        children: impl IntoIterator<Item = AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let id = id.into();
        self.step_glide(window);
        let left = -self.scroll.offset().x;
        let max = self.scroll.max_offset().x;
        let edge = |name: &str| ElementId::NamedChild(Arc::new(id.clone()), name.into());
        motion::set(
            edge("after"),
            left < max - px(1.),
            theme::T_MED,
            theme::EASE,
            cx,
        );
        let after = motion::get(edge("after"), window, cx);
        let (top, side, bottom) = room;
        let fade = |angle: f32, width: f32| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(FADE * width))
                .bg(linear_gradient(
                    angle,
                    linear_color_stop(rgb(theme::INK), 0.),
                    linear_color_stop(rgba(theme::INK << 8), 1.),
                ))
        };

        let drag = self.drag.clone();
        let scroll = self.scroll.clone();
        let on_down = move |e: &MouseDownEvent, _: &mut Window, _: &mut App| {
            let mut d = drag.borrow_mut();
            *d = Drag {
                press: Some((e.position.x, -scroll.offset().x)),
                samples: vec![(e.position.x, Instant::now())],
                ..Drag::default()
            };
        };

        let drag = self.drag.clone();
        let scroll = self.scroll.clone();
        // Moves and the release are followed over the whole window, as a pointer capture does.
        let follow = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                if drag.borrow().press.is_none() {
                    return;
                }
                let (d, s) = (drag.clone(), scroll.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, _, window, _| {
                    let mut d = d.borrow_mut();
                    let Some((x0, left0)) = d.press else { return };
                    if e.pressed_button != Some(MouseButton::Left) {
                        d.press = None;
                        return;
                    }
                    let dx = e.position.x - x0;
                    if !d.dragging && dx.abs() <= px(DRAG_THRESHOLD) {
                        return;
                    }
                    d.dragging = true;
                    d.samples.push((e.position.x, Instant::now()));
                    scroll_to(&s, left0 - dx);
                    window.refresh();
                });
                let d = drag.clone();
                // Registered after the cards, so their click is armed by now and stopping the
                // event here keeps it from firing.
                window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                    if !phase.capture() || e.button != MouseButton::Left {
                        return;
                    }
                    let mut d = d.borrow_mut();
                    if d.press.take().is_none() || !d.dragging {
                        return;
                    }
                    cx.stop_propagation();
                    let now = Instant::now();
                    let recent: Vec<_> = d
                        .samples
                        .iter()
                        .filter(|(_, t)| (now - *t).as_secs_f32() * 1000. <= SPEED_WINDOW_MS)
                        .collect();
                    if let (Some(first), Some(last)) = (recent.first(), recent.last()) {
                        let ms = ((last.1 - first.1).as_secs_f32() * 1000.).max(1.);
                        let speed = -f32::from(last.0 - first.0) / ms;
                        if recent.len() >= 2 && speed.abs() > STOP_SPEED {
                            d.glide = Some((speed, now));
                        }
                    }
                    window.refresh();
                });
            },
        )
        .absolute()
        .size_full();

        div()
            .relative()
            .mt(px(-top))
            .mx(px(-side))
            .mb(px(-bottom))
            .child(
                div()
                    .id(id)
                    .overflow_x_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
                    .on_mouse_down(MouseButton::Left, on_down)
                    .flex()
                    .gap(gap)
                    .pt(px(top))
                    .px(px(side))
                    .pb(px(bottom))
                    .children(children),
            )
            .when(after > 0., |row| row.child(fade(270., after).right_0()))
            .child(follow)
            .into_any_element()
    }
}

fn visible_items(left: f32, screen: f32, count: usize, step: f32) -> Range<usize> {
    // Two cards either side: enough for a frame of a fling, and every card built costs.
    let first = (left / step).floor().max(2.) as usize - 2;
    let last = ((left + screen) / step).ceil().max(0.) as usize + 2;
    first.min(count)..last.min(count)
}

/// The built items of `shown` with empty boxes standing for the rest of the `count`, so the row
/// keeps its whole length and scroll: a long row costs what its visible part does.
pub fn spaced(
    shown: Range<usize>,
    count: usize,
    width: f32,
    gap: f32,
    items: Vec<AnyElement>,
) -> Vec<AnyElement> {
    let space = |n: usize| {
        (n > 0).then(|| {
            div()
                .flex_none()
                .w(px(n as f32 * (width + gap) - gap))
                .into_any_element()
        })
    };
    space(shown.start)
        .into_iter()
        .chain(items)
        .chain(space(count - shown.end.min(count)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::visible_items;

    #[test]
    fn a_row_builds_only_what_is_near_its_visible_part() {
        // Items 150 + 24 apart, a window 1600 wide.
        assert_eq!(visible_items(0., 1600., 40, 174.), 0..12);
        assert_eq!(visible_items(3000., 1600., 40, 174.), 15..29);
        assert_eq!(visible_items(0., 1600., 5, 174.), 0..5);
    }
}
