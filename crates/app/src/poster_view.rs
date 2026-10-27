//! A poster over the whole window, in the size it was uploaded: the wheel zooms in where the
//! pointer is, a drag moves it, a double click goes between fitting and close up, a click on the
//! dark around it or Esc closes it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnchoredPositionMode, AnyElement, ClickEvent, Context, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, ScrollEvent, Window, anchored,
    deferred, div, img, point, prelude::*, px, rgba,
};

use crate::assets::icon;
use crate::core::Core;
use crate::theme;

const MAX_SCALE: f32 = 6.;
const CLOSE_UP: f32 = 2.5;
const OPEN: Duration = Duration::from_millis(240);
/// How fast the shown zoom catches up with the wanted one, per second.
const PACE: f32 = 16.;

pub struct PosterView {
    /// The picture: the card's own until the full one is in.
    pub path: PathBuf,
    /// Its size in pixels, to fit it to the window.
    pub size: (f32, f32),
    /// 1 fits the window.
    scale: f32,
    /// How far the picture is moved from the middle.
    pan: Point<Pixels>,
    /// A drag: where the pointer went down, the pan then, and whether it moved.
    drag: Option<(Point<Pixels>, Point<Pixels>, bool)>,
    /// What is drawn, easing towards `scale` and `pan`.
    shown: (f32, Point<Pixels>),
    last: Instant,
    opened: Instant,
    /// The biggest poster is in (or there is none to wait for).
    pub sharp: bool,
}

impl PosterView {
    pub fn new(path: PathBuf) -> Self {
        let size = image::image_dimensions(&path).map_or((2., 3.), |(w, h)| (w as f32, h as f32));
        Self {
            path,
            size,
            scale: 1.,
            pan: Point::default(),
            drag: None,
            shown: (1., Point::default()),
            last: Instant::now(),
            opened: Instant::now(),
            sharp: false,
        }
    }

    /// Eases what is drawn towards what is wanted; whether it is still moving.
    fn step(&mut self) -> bool {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        if self.drag.is_some() {
            self.shown = (self.scale, self.pan);
            return false;
        }
        let k = 1. - (-PACE * dt).exp();
        let (s, p) = self.shown;
        let next = (s + (self.scale - s) * k, p + (self.pan - p) * k);
        let done = (next.0 - self.scale).abs() < 0.002
            && f32::from((next.1.x - self.pan.x).abs() + (next.1.y - self.pan.y).abs()) < 0.5;
        self.shown = if done { (self.scale, self.pan) } else { next };
        !done
    }

    /// The bigger picture arrived: shown in its place at the same zoom.
    pub fn sharpen(&mut self, path: PathBuf) {
        let size = self.size;
        *self = Self {
            scale: self.scale,
            pan: self.pan,
            shown: self.shown,
            opened: self.opened,
            sharp: true,
            ..Self::new(path)
        };
        // Same shape: the zoom and the place stay right; another shape starts over.
        if ((self.size.0 / self.size.1) - (size.0 / size.1)).abs() > 0.02 {
            self.scale = 1.;
            self.pan = Point::default();
            self.shown = (1., Point::default());
        }
    }

    /// Zooms by `factor` keeping the point under `at` (from the middle of the window) in place.
    fn zoom(&mut self, factor: f32, at: Point<Pixels>) {
        let next = (self.scale * factor).clamp(1., MAX_SCALE);
        let k = next / self.scale;
        self.pan = at - (at - self.pan) * k;
        self.scale = next;
        if self.scale == 1. {
            self.pan = Point::default();
        }
    }
}

/// The biggest poster the sources have, downloaded once; `None` when there is none.
pub fn load_full(
    core: &Arc<Core>,
    id: i64,
) -> impl Future<Output = anyhow::Result<Option<PathBuf>>> + use<> {
    core.run(move |c| async move {
        let original = c.catalog.poster_original(id).await.ok().flatten();
        Ok(c.posters.full(id, original).await)
    })
}

/// The view over the window for the page `V`, which keeps it at `slot`.
pub fn render<V: 'static>(
    slot: fn(&mut V) -> &mut Option<PosterView>,
    view: &mut PosterView,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    let opening = (view.opened.elapsed().as_secs_f32() / OPEN.as_secs_f32()).min(1.);
    let opening = theme::EASE.at(opening);
    if view.step() || opening < 1. {
        window.request_animation_frame();
    }
    let (scale, pan) = view.shown;
    let scale = scale * (0.96 + 0.04 * opening);
    let screen = window.viewport_size();
    let (w, h) = view.size;
    let fit = (f32::from(screen.width) * 0.92 / w).min(f32::from(screen.height) * 0.92 / h);
    let (pw, ph) = (px(w * fit * scale), px(h * fit * scale));
    let middle = point(screen.width / 2., screen.height / 2.);
    let origin = middle + pan - point(pw / 2., ph / 2.);
    let page = cx.entity().downgrade();
    crate::popover::close_on_escape(cx, move |_, cx| {
        page.update(cx, |page, cx| {
            *slot(page) = None;
            cx.notify();
        })
        .ok();
    });
    let close = cx.listener(move |page: &mut V, _: &ClickEvent, _, cx| {
        *slot(page) = None;
        cx.notify();
    });
    let layer = div()
        .id("poster-view")
        .occlude()
        .w(screen.width)
        .h(screen.height)
        .bg(rgba(0x05050af0))
        .opacity(opening)
        .cursor(if view.drag.is_some() {
            gpui::CursorStyle::ClosedHand
        } else if view.scale > 1. {
            gpui::CursorStyle::OpenHand
        } else {
            gpui::CursorStyle::Arrow
        })
        .on_scroll(cx.listener(move |page, e: &ScrollEvent, window, cx| {
            let Some(v) = slot(page) else { return };
            let dy = f32::from(e.delta.pixel_delta(window.line_height()).y);
            let at = e.position
                - point(
                    window.viewport_size().width / 2.,
                    window.viewport_size().height / 2.,
                );
            v.zoom((dy * 0.004).exp(), at);
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |page, e: &MouseDownEvent, window, cx| {
                let Some(v) = slot(page) else { return };
                if e.click_count == 2 {
                    let at = e.position
                        - point(
                            window.viewport_size().width / 2.,
                            window.viewport_size().height / 2.,
                        );
                    let target = if v.scale > 1. { 1. / v.scale } else { CLOSE_UP };
                    v.zoom(target, at);
                } else {
                    v.drag = Some((e.position, v.pan, false));
                }
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(move |page, e: &MouseMoveEvent, _, cx| {
            let Some(v) = slot(page) else { return };
            if let Some((from, pan, moved)) = v.drag.as_mut() {
                let d = e.position - *from;
                *moved |= f32::from(d.x).abs() + f32::from(d.y).abs() > 4.;
                if v.scale > 1. {
                    v.pan = *pan + d;
                }
                cx.notify();
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |page, e: &MouseUpEvent, _, cx| {
                let Some(v) = slot(page) else { return };
                let Some((_, _, moved)) = v.drag.take() else {
                    return;
                };
                // A plain click on the dark around the picture closes it.
                let on_picture =
                    gpui::Bounds::new(origin, gpui::size(pw, ph)).contains(&e.position);
                if !moved && !on_picture && e.click_count < 2 {
                    *slot(page) = None;
                }
                cx.notify();
            }),
        )
        .child(
            div()
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(pw)
                .h(ph)
                .overflow_hidden()
                .rounded(theme::R_M)
                .child(
                    img(view.path.clone())
                        .size_full()
                        .rounded(theme::R_M)
                        .object_fit(ObjectFit::Contain),
                ),
        )
        .when(!view.sharp, |d| {
            d.child(
                div()
                    .absolute()
                    .bottom(px(24.))
                    .left(screen.width / 2. - px(130.))
                    .w(px(260.))
                    .py(px(8.))
                    .rounded_full()
                    .bg(theme::glass_strong())
                    .text_size(px(13.))
                    .text_color(gpui::rgb(theme::TEXT_2))
                    .flex()
                    .justify_center()
                    .child("Загружаю в высоком качестве…"),
            )
        })
        .child(
            div()
                .id("poster-close")
                .absolute()
                .top(px(20.))
                .right(px(20.))
                .size(px(40.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(theme::glass_strong())
                .hover(|s| s.bg(rgba(0xffffff33)))
                .on_click(close)
                .child(icon("close", 20.).text_color(gpui::rgb(theme::TEXT))),
        );
    deferred(
        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .position(point(px(0.), px(0.)))
            .child(layer),
    )
    .with_priority(5)
    .into_any_element()
}
