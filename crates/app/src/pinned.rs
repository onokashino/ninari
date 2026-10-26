//! A page's controls (presets, filters, search) that stay along the top of the window once the
//! page scrolls past them. They move rather than copy: while pinned, the page keeps an empty slot
//! of their height and Root draws them over the top of the page, so a text field or a menu among
//! them is still one element.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, BoxShadow, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, canvas, div, point, prelude::*, px, rgba,
};

use crate::theme;

/// Where the controls' slot was on the last frame: its top in window coordinates and its height.
#[derive(Clone, Default)]
pub struct Pin(Rc<Cell<Option<(f32, f32)>>>);

impl Pin {
    /// Whether the slot has gone above the window's top, so the controls ride pinned.
    pub fn pinned(&self) -> bool {
        self.0.get().is_some_and(|(top, _)| top < 0.)
    }

    /// The controls' place in the page: `controls` themselves, or while pinned an empty box of
    /// the height they had.
    pub fn slot(&self, controls: Option<AnyElement>) -> AnyElement {
        let measure = self.0.clone();
        let height = self.0.get().map_or(0., |(_, h)| h);
        div()
            .relative()
            .map(|d| match controls {
                Some(c) => d.child(c),
                None => d.h(px(height)),
            })
            .child(
                canvas(
                    move |bounds, _, _| {
                        let top = f32::from(bounds.top());
                        let height = match measure.get() {
                            // An empty slot keeps the height the controls had.
                            Some((_, h)) if top < 0. => h,
                            _ => f32::from(bounds.size.height),
                        };
                        measure.set(Some((top, height)));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }
}

/// The panel floating over the top of the page that holds pinned controls, in the look of the
/// app's other floating things (menus, the hover details): glass, rounded, a soft shadow.
pub fn band(controls: AnyElement, window: &gpui::Window) -> AnyElement {
    // Open filters can be taller than the window: the panel stops short of the bottom and
    // scrolls inside itself.
    let tallest = f32::from(window.viewport_size().height) - 80.;
    div()
        .id("pinned")
        .occlude()
        .absolute()
        .top(px(10.))
        .left(px(24.))
        .right(px(24.))
        .max_h(px(tallest))
        .overflow_y_scroll()
        .px(px(24.))
        .py(px(12.))
        .rounded(theme::R_L)
        // GPUI cannot blur what lies under an element: dense enough not to need it.
        .bg(rgba((theme::INK << 8) | 0xf6))
        .border_1()
        .border_color(theme::line())
        .shadow(vec![BoxShadow {
            color: rgba(0x00000080).into(),
            offset: point(px(0.), px(12.)),
            blur_radius: px(32.),
            spread_radius: px(-8.),
            inset: false,
        }])
        .child(controls)
        .into_any_element()
}

/// A page's pinned controls as a view of their own, which Root draws cached: the page scrolling
/// under them does not lay them out and paint them again (open filters are a few hundred chips).
/// The page notifies on every frame a card animates, so the band is drawn anew only when what
/// the controls show, `key`, changed; a text field or a menu among them wakes it by itself.
pub struct Band {
    controls: Box<dyn Fn(&mut Window, &mut App) -> Option<AnyElement>>,
    _page: Subscription,
}

impl Band {
    pub fn new<V: 'static>(
        page: &Entity<V>,
        controls: fn(&mut V, &mut Window, &mut Context<V>) -> Option<AnyElement>,
        key: fn(&V) -> String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut last = key(page.read(cx));
        let observed = cx.observe(page, move |_, page, cx| {
            let now = key(page.read(cx));
            if now != last {
                last = now;
                cx.notify();
            }
        });
        let page = page.clone();
        Self {
            controls: Box::new(move |window, cx| {
                page.update(cx, |page, cx| controls(page, window, cx))
            }),
            _page: observed,
        }
    }
}

impl Render for Band {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children((self.controls)(window, cx))
    }
}
