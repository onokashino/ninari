//! Text the app shows can't be selected (GPUI has no selectable text): names and descriptions
//! are copied with a click instead, and "Скопировано" shows for a moment at the bottom.

use std::time::{Duration, Instant};

use gpui::{AnyElement, App, ClipboardItem, Global, IntoElement, Styled, div, prelude::*, px, rgb};

use crate::theme;

const SHOWN: Duration = Duration::from_millis(1300);

#[derive(Default)]
struct Copied(Option<Instant>);

impl Global for Copied {}

pub fn copy(text: impl Into<String>, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
    cx.default_global::<Copied>().0 = Some(Instant::now());
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SHOWN).await;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

/// The note over the bottom of the window while a copy is fresh; Root draws it.
pub fn note(cx: &App) -> Option<AnyElement> {
    let at = cx.try_global::<Copied>()?.0?;
    (at.elapsed() < SHOWN).then(|| {
        div()
            .absolute()
            .bottom(px(28.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .px(px(14.))
                    .py(px(7.))
                    .rounded_full()
                    .bg(theme::glass_strong())
                    .border_1()
                    .border_color(theme::line())
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("Скопировано"),
            )
            .into_any_element()
    })
}
