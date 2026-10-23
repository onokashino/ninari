//! The mouse wheel glides the page instead of jumping a notch at a time, like Lenis (lerp 0.13):
//! the wheel moves a target and the page eases towards it every frame, at the same pace whatever
//! the refresh rate. A touchpad scrolls smoothly on its own and is left alone; so is anything over
//! the page, like an open list.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    HitboxBehavior, IntoElement, Pixels, ScrollEvent, ScrollHandle, Window, canvas, prelude::*, px,
};

/// Lenis's lerp of 0.13 a frame at 60 Hz, per second.
const PACE: f32 = 0.13 * 60.;

#[derive(Clone)]
pub struct Glide {
    /// Where the page is heading (a scroll offset, so zero or below).
    target: Rc<Cell<Option<Pixels>>>,
    last: Rc<Cell<Instant>>,
}

impl Default for Glide {
    fn default() -> Self {
        Self {
            target: Rc::default(),
            last: Rc::new(Cell::new(Instant::now())),
        }
    }
}

impl Glide {
    /// Leaving the page or touching the touchpad ends a glide where it is.
    pub fn stop(&self) {
        self.target.set(None);
    }

    /// Moves `scroll` on by the time since the last frame; called while drawing.
    pub fn step(&self, scroll: &ScrollHandle, window: &mut Window) {
        let Some(target) = self.target.get() else {
            return;
        };
        let now = Instant::now();
        let dt = (now - self.last.replace(now)).as_secs_f32().min(0.1);
        let target = target.clamp(-scroll.max_offset().y, px(0.));
        let y = scroll.offset().y;
        let next = y + (target - y) * (1. - (-PACE * dt).exp());
        if (target - next).abs() < px(0.5) {
            scroll.set_offset(gpui::point(scroll.offset().x, target));
            self.target.set(None);
        } else {
            scroll.set_offset(gpui::point(scroll.offset().x, next));
            self.target.set(Some(target));
            window.request_animation_frame();
        }
    }

    /// A layer over the page that takes the mouse wheel for `scroll`: put it last among the
    /// page's siblings, the size of the page.
    pub fn catcher(&self, scroll: ScrollHandle) -> impl IntoElement {
        let glide = self.clone();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let glide = glide.clone();
                let scroll = scroll.clone();
                window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                    if !phase.capture() || !hitbox.is_hovered(window) {
                        return;
                    }
                    if e.delta.precise() {
                        glide.stop();
                        return;
                    }
                    let dy = e.delta.pixel_delta(window.line_height()).y;
                    let max = scroll.max_offset().y;
                    let from = match glide.target.get() {
                        Some(t) => t,
                        None => {
                            glide.last.set(Instant::now());
                            scroll.offset().y
                        }
                    };
                    glide.target.set(Some((from + dy).clamp(-max, px(0.))));
                    cx.stop_propagation();
                    window.refresh();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}
