//! The app's button: primary in the accent, glass, or quiet. The caller adds what a click does.

use gpui::{
    BoxShadow, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, div, point, prelude::*,
    px, rgb, white,
};

use crate::assets::icon;
use crate::theme;

#[derive(Clone, Copy, PartialEq)]
pub enum Variant {
    Primary,
    Glass,
    Quiet,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Size {
    M,
    L,
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<&str>,
    variant: Variant,
    size: Size,
    accent: Hsla,
) -> Stateful<Div> {
    let (fg, hover_fg): (Hsla, Hsla) = match variant {
        Variant::Primary => (
            rgb(theme::ACCENT_CONTRAST).into(),
            rgb(theme::ACCENT_CONTRAST).into(),
        ),
        Variant::Glass => (rgb(theme::TEXT).into(), rgb(theme::TEXT).into()),
        Variant::Quiet => (rgb(theme::TEXT_2).into(), rgb(theme::TEXT).into()),
    };
    // A hovered button rises a pixel and a pressed one sinks one: GPUI cannot scale a box.
    let base = div()
        .id(id)
        .relative()
        .active(|s| s.top(px(1.)))
        .group("button")
        .cursor_pointer()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .rounded_full()
        .font_weight(FontWeight::BOLD)
        .whitespace_nowrap()
        .text_color(fg);
    let base = match size {
        Size::M => base.px(px(16.)).py(px(9.)).text_size(px(13.)),
        Size::L => base.pl(px(20.)).pr(px(24.)).py(px(13.)).text_size(px(15.)),
    };
    let base = match variant {
        Variant::Primary => base
            .bg(accent)
            .shadow(vec![BoxShadow {
                color: accent.opacity(0.7),
                offset: point(px(0.), px(10.)),
                blur_radius: px(30.),
                spread_radius: px(-10.),
                inset: false,
            }])
            .hover(move |s| s.bg(theme::mix(accent, white(), 0.12)).top(px(-1.))),
        Variant::Glass => base
            .bg(theme::glass())
            .border_1()
            .border_color(theme::line())
            .hover(|s| s.bg(theme::glass_strong()).top(px(-1.))),
        Variant::Quiet => base
            .px(px(8.))
            .hover(move |s| s.text_color(hover_fg).top(px(-1.))),
    };
    let size = if size == Size::L { 20. } else { 18. };
    base.children(glyph.map(|g| {
        icon(g, size)
            .text_color(fg)
            .group_hover("button", move |s| s.text_color(hover_fg))
    }))
    .child(label.into())
}
