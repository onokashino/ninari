//! The mouse wheel glides the page instead of jumping a notch at a time, like Lenis (lerp 0.13):
//! the wheel moves a target and the page eases towards it every frame, at the same pace whatever
//! the refresh rate. A touchpad scrolls smoothly on its own and is left alone; so is anything over
//! the page, like an open list.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    HitboxBehavior, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, ScrollEvent, ScrollHandle, Window, canvas, div, point, prelude::*, px, rgba,
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

/// Scrolling with the middle button, as in a browser: a press leaves a mark, and the page moves
/// towards the pointer, the faster the farther it is from the mark. Another click ends it, and so
/// does letting go after holding the button and moving.
#[derive(Clone, Default)]
pub struct Autoscroll(Rc<Cell<Option<Auto>>>);

#[derive(Clone, Copy)]
struct Auto {
    mark: Point<Pixels>,
    pointer: Point<Pixels>,
    pressed: Instant,
    last: Instant,
    /// Moved away while the button was still down: letting go ends it.
    dragged: bool,
}

/// No motion this close to the mark, so a still hand keeps the page still.
const DEAD: f32 = 12.;
/// Pixels a second per pixel of distance from the mark.
const SPEED: f32 = 7.;

impl Autoscroll {
    pub fn stop(&self) {
        self.0.set(None);
    }

    /// Moves `scroll` on by the time since the last frame; called while drawing.
    pub fn step(&self, scroll: &ScrollHandle, window: &mut Window) {
        let Some(mut auto) = self.0.get() else {
            return;
        };
        let now = Instant::now();
        let dt = (now - auto.last).as_secs_f32().min(0.1);
        auto.last = now;
        self.0.set(Some(auto));
        let dy = f32::from(auto.pointer.y - auto.mark.y);
        let speed = (dy.abs() - DEAD).max(0.) * SPEED * dy.signum();
        let y = (scroll.offset().y - px(speed * dt)).clamp(-scroll.max_offset().y, px(0.));
        scroll.set_offset(point(scroll.offset().x, y));
        window.request_animation_frame();
    }

    /// The mark where the press was, while scrolling.
    pub fn mark(&self) -> Option<impl IntoElement + use<>> {
        let auto = self.0.get()?;
        let size = px(30.);
        Some(
            div()
                .absolute()
                .left(auto.mark.x - size / 2.)
                .top(auto.mark.y - size / 2.)
                .size(size)
                .rounded_full()
                .border_2()
                .border_color(rgba(0xffffffcc))
                .bg(rgba(0x0c0c12aa))
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(6.)).rounded_full().bg(rgba(0xffffffdd))),
        )
    }

    /// A layer over the page that takes the middle button: put it last among the page's
    /// siblings, the size of the page.
    pub fn layer(&self) -> impl IntoElement + use<> {
        let auto = self.0.clone();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let on_down = auto.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if !phase.capture() {
                        return;
                    }
                    if on_down.get().is_some() {
                        // Any click ends it, and is not a click on what lies under the pointer.
                        on_down.set(None);
                        cx.stop_propagation();
                        window.refresh();
                    } else if e.button == MouseButton::Middle && hitbox.is_hovered(window) {
                        let now = Instant::now();
                        on_down.set(Some(Auto {
                            mark: e.position,
                            pointer: e.position,
                            pressed: now,
                            last: now,
                            dragged: false,
                        }));
                        cx.stop_propagation();
                        window.refresh();
                    }
                });
                let on_move = auto.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, _| {
                    if !phase.capture() {
                        return;
                    }
                    if let Some(mut a) = on_move.get() {
                        a.pointer = e.position;
                        let far = f32::from((e.position.y - a.mark.y).abs()) > DEAD;
                        a.dragged |= far && e.pressed_button == Some(MouseButton::Middle);
                        on_move.set(Some(a));
                    }
                });
                let on_up = auto.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, window, _| {
                    if !phase.capture() || e.button != MouseButton::Middle {
                        return;
                    }
                    if let Some(a) = on_up.get()
                        && (a.dragged || a.pressed.elapsed().as_millis() > 400)
                        && a.mark != e.position
                    {
                        on_up.set(None);
                        window.refresh();
                    }
                });
                let on_wheel = auto.clone();
                window.on_mouse_event(move |_: &ScrollEvent, phase, _, _| {
                    if phase.capture() {
                        on_wheel.set(None);
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}
