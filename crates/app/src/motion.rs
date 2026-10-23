//! CSS-like transitions for what changes on hover or on a toggle: a value per key eases towards
//! 1 or 0 when its target flips, from wherever it is, and the window redraws until it settles.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt, App, ElementId, Global, IntoElement, Window, div, prelude::*, px,
};

use crate::theme::{Curve, EASE};

struct Tween {
    on: bool,
    from: f32,
    since: Instant,
    dur: Duration,
    curve: Curve,
}

impl Tween {
    /// The value at `now` and whether it got there.
    fn at(&self, now: Instant) -> (f32, bool) {
        let t = ((now - self.since).as_secs_f32() / self.dur.as_secs_f32()).min(1.);
        let to = if self.on { 1. } else { 0. };
        (self.from + (to - self.from) * self.curve.at(t), t >= 1.)
    }
}

#[derive(Default)]
struct Tweens(HashMap<ElementId, Tween>);

impl Global for Tweens {}

/// Points `key` at 1 (`on`) or at 0 over `dur`.
pub fn set(key: impl Into<ElementId>, on: bool, dur: Duration, curve: Curve, cx: &mut App) {
    let key = key.into();
    let now = Instant::now();
    let tweens = cx.default_global::<Tweens>();
    let from = match tweens.0.get(&key) {
        Some(t) if t.on == on => return,
        Some(t) => t.at(now).0,
        None if !on => return,
        None => 0.,
    };
    tweens.0.insert(
        key,
        Tween {
            on,
            from,
            since: now,
            dur,
            curve,
        },
    );
}

/// The value of `key` this frame; 0 for a key never set.
pub fn get(key: impl Into<ElementId>, window: &mut Window, cx: &App) -> f32 {
    let Some(tween) = cx.try_global::<Tweens>().and_then(|t| t.0.get(&key.into())) else {
        return 0.;
    };
    let (value, done) = tween.at(Instant::now());
    if !done {
        window.request_animation_frame();
    }
    value
}

/// The entrance of the `index`th card of a list: it rises into place, the next ones a little
/// later. A wrapper moves, so the card keeps its own offsets.
pub fn rise(
    el: impl IntoElement,
    id: impl Into<ElementId>,
    index: usize,
    ms: u64,
    step: u64,
    lift: f32,
) -> impl IntoElement {
    let delay = index.min(14) as u64 * step;
    let total = (delay + ms) as f32;
    div().flex_none().child(el).with_animation(
        id,
        Animation::new(Duration::from_millis(delay + ms))
            .with_easing(move |t| EASE.at(((t * total - delay as f32) / ms as f32).clamp(0., 1.))),
        move |el, t| el.opacity(t).relative().top(px(lift * (1. - t))),
    )
}
