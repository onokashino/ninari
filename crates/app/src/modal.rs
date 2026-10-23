//! A dialog over the dimmed page: Escape, the close button or a press outside it closes it.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, BoxShadow, ElementId, FontWeight, Window, anchored, deferred, div, point,
    prelude::*, px, rgb, rgba,
};

use crate::assets::icon;
use crate::motion;
use crate::theme;

pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// The dialog while `open` and as it fades out; `body` is drawn under the title.
pub fn modal(
    key: impl Into<ElementId>,
    open: bool,
    title: &'static str,
    body: AnyElement,
    on_close: OnClose,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let key = key.into();
    motion::set(
        key.clone(),
        open,
        Duration::from_millis(220),
        theme::EASE,
        cx,
    );
    let shown = motion::get(key.clone(), window, cx);
    if shown <= 0. {
        return None;
    }
    let view = window.viewport_size();
    let (outside, button, escape) = (on_close.clone(), on_close.clone(), on_close);
    if open {
        crate::popover::close_on_escape(cx, move |window, cx| escape(window, cx));
    }
    let panel = div()
        .occlude()
        .on_mouse_down_out(move |_, window, cx| {
            if open {
                outside(window, cx);
            }
        })
        .flex()
        .flex_col()
        .w(px(640.).min(view.width - px(64.)))
        .max_h(px(760.).min(view.height - px(64.)))
        .rounded(theme::R_L)
        .bg(rgba(0x12121af5))
        .border_1()
        .border_color(rgba(0xffffff14))
        .shadow(vec![BoxShadow {
            color: rgba(0x0000008c).into(),
            offset: point(px(0.), px(40.)),
            blur_radius: px(100.),
            spread_radius: px(0.),
            inset: false,
        }])
        .relative()
        .top(px(10. * (1. - shown)))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .px(px(24.))
                .pt(px(24.))
                .pb(px(12.))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .child(title),
                )
                .child(
                    div()
                        .id(ElementId::NamedChild(Arc::new(key.clone()), "close".into()))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(36.))
                        .rounded_full()
                        .text_color(rgb(theme::TEXT_2))
                        .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
                        .on_click(move |_, window, cx| button(window, cx))
                        .child(icon("close", 20.).text_color(rgb(theme::TEXT_2))),
                ),
        )
        .child(
            div()
                .id(ElementId::NamedChild(Arc::new(key), "body".into()))
                .overflow_y_scroll()
                .min_h(px(0.))
                .px(px(24.))
                .pb(px(24.))
                .child(body),
        );

    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .occlude()
                    .w(view.width)
                    .h(view.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x08080c8c))
                    .opacity(shown.min(1.))
                    .child(panel),
            ),
        )
        .with_priority(1)
        .into_any_element(),
    )
}
