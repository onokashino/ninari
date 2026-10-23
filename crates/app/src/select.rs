//! A dropdown in the app's look: a pill that opens a list under it; a press elsewhere or Escape
//! closes it.

use std::f32::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Div, FontWeight, Hsla, SharedString, Transformation, Window, div, prelude::*, px, radians,
    rgb,
};

use crate::assets::icon;
use crate::motion;
use crate::popover::{Menu, popover};
use crate::theme;

pub struct SelectOption {
    pub value: String,
    pub label: String,
    /// A quiet note beside the label in the list, such as a count.
    pub hint: Option<String>,
}

pub type OnPick = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub fn select(
    id: &'static str,
    menu: &Menu,
    value: &str,
    options: &[SelectOption],
    accent: Hsla,
    on_pick: OnPick,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let open = menu.is_open();
    // The chevron eases from pointing down to up with the list, like the CSS rotate.
    let turn_key = (id, 1u64);
    motion::set(turn_key, open, Duration::from_millis(220), theme::EASE, cx);
    let turn = motion::get(turn_key, window, cx);
    let current = options.iter().find(|o| o.value == value);
    let toggle = menu.clone();
    let trigger = div()
        .id(id)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(16.))
        .pr(px(12.))
        .py(px(8.))
        .rounded_full()
        .border_1()
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .whitespace_nowrap()
        .map(|d| {
            if open {
                d.bg(theme::glass_strong())
                    .border_color(accent.opacity(0.55))
            } else {
                d.bg(theme::glass())
                    .border_color(theme::line())
                    .hover(|s| s.bg(theme::glass_strong()))
            }
        })
        .on_click(move |e, window, _| {
            toggle.toggle(e);
            window.refresh();
        })
        .child(SharedString::from(
            current.map(|o| o.label.clone()).unwrap_or_default(),
        ))
        .child(
            icon("back", 16.)
                .text_color(rgb(theme::TEXT_3))
                .with_transformation(Transformation::rotate(radians(PI * (turn - 0.5)))),
        );

    let list = div()
        .id((id, 2u64))
        .min_w(px(240.))
        .max_h(px(300.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .children(options.iter().enumerate().map(|(i, o)| {
            let picked = o.value == value;
            let (pick, menu, chosen) = (on_pick.clone(), menu.clone(), o.value.clone());
            div()
                .id((id, 100 + i as u64))
                .cursor_pointer()
                .flex()
                .justify_between()
                .items_center()
                .gap(px(16.))
                .px(px(12.))
                .py(px(9.))
                .rounded(px(9.))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .whitespace_nowrap()
                .text_color(if picked {
                    accent
                } else {
                    rgb(theme::TEXT_2).into()
                })
                .hover(|s| s.bg(theme::glass_strong()).text_color(rgb(theme::TEXT)))
                .on_click(move |_, window, cx| {
                    menu.close();
                    pick(chosen.clone(), window, cx);
                    window.refresh();
                })
                .child(o.label.clone())
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.))
                        .children(o.hint.clone().map(|h| {
                            div()
                                .text_color(rgb(theme::TEXT_3))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(h)
                        }))
                        .when(picked, |d| d.child(icon("check", 16.).text_color(accent))),
                )
        }));

    div().relative().child(trigger).children(popover(
        menu,
        (id, 3u64),
        div().child(list),
        window,
        cx,
    ))
}
