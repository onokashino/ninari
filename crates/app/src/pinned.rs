//! A page's controls (presets, filters, search) that stay along the top of the window once the
//! page scrolls past them. They move rather than copy: while pinned, the page keeps an empty slot
//! of their height and Root draws them over the top of the page, so a text field or a menu among
//! them is still one element.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, BoxShadow, IntoElement, ParentElement, Styled, canvas, div, point, prelude::*, px,
    rgba,
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
pub fn band(controls: AnyElement) -> AnyElement {
    div()
        .id("pinned")
        .occlude()
        .absolute()
        .top(px(10.))
        .left(px(24.))
        .right(px(24.))
        .max_h(gpui::relative(0.8))
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
