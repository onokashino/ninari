//! The rail on the left: the stamp, the sections, settings at the bottom.

use gpui::{
    Div, FontWeight, Hsla, IntoElement, div, linear_color_stop, linear_gradient, prelude::*, px,
    rgb, rgba,
};

use crate::OnClick;
use crate::assets::icon;
use crate::nav::Route;
use crate::theme;

pub const WIDTH: f32 = 84.;

const TOP: [(Route, &str, &str); 5] = [
    (Route::Home, "home", "Главная"),
    (Route::Search, "search", "Поиск"),
    (Route::Catalog, "compass", "Каталог"),
    (Route::Library, "library", "Библиотека"),
    (Route::Schedule, "calendar", "Расписание"),
];

pub fn label(route: Route) -> &'static str {
    match route {
        Route::Settings => "Настройки",
        Route::History => "История",
        Route::Stats => "Статистика",
        _ => TOP
            .iter()
            .find(|(r, ..)| *r == route)
            .map_or("", |(.., l)| l),
    }
}

/// The ninari stamp: it keeps its own red whatever the accent, it is how the app is recognised.
pub fn brand_mark(size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 60. / 256.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(0xee5462), 0.),
            linear_color_stop(rgb(0xd7384a), 1.),
        ))
        .child(icon("brand", size).text_color(rgb(0xfff4ef)))
}

/// `link(route)` makes the click handler that opens `route`.
pub fn rail(current: Route, accent: Hsla, link: impl Fn(Route) -> OnClick) -> impl IntoElement {
    let item = |route: Route, name: &'static str, text: &'static str| {
        let active = route == current;
        div()
            .id(name)
            .group("rail-item")
            .cursor_pointer()
            .on_click(link(route))
            .w(px(WIDTH - 12.))
            .pt(px(8.))
            .pb(px(6.))
            .px(px(2.))
            .rounded(theme::R_M)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(3.))
            .map(|d| {
                if active {
                    d.text_color(rgb(theme::TEXT)).bg(accent.opacity(0.18))
                } else {
                    d.text_color(rgb(theme::TEXT_3))
                        .hover(|s| s.text_color(rgb(theme::TEXT)).bg(theme::glass()))
                }
            })
            .child(icon(name, 22.).map(|i| {
                if active {
                    i.text_color(accent)
                } else {
                    i.text_color(rgb(theme::TEXT_3))
                        .group_hover("rail-item", |s| s.text_color(rgb(theme::TEXT)))
                }
            }))
            .child(
                div()
                    .max_w_full()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .child(text),
            )
    };

    div()
        .w(px(WIDTH))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(24.))
        .pt(px(42.))
        .pb(px(24.))
        .bg(rgba(0x101117b8))
        .border_r_1()
        .border_color(theme::line())
        .child(
            // The stamp keeps its own red whatever the accent: it is how the app is recognised.
            brand_mark(40.)
                .id("mark")
                .cursor_pointer()
                .on_click(link(Route::Home)),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(TOP.map(|(route, name, text)| item(route, name, text))),
        )
        .child(item(Route::Settings, "settings", label(Route::Settings)))
}
