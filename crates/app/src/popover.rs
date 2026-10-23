//! A panel under its trigger that eases in and out (the status menu, the score, the dub picker): a
//! press outside it or Escape closes it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Anchor, AnyElement, App, BoxShadow, ClickEvent, Div, ElementId, Global, Pixels, Point, Window,
    anchored, deferred, div, point, prelude::*, px, relative, rgb, rgba,
};

use crate::motion;
use crate::theme;

type Close = Rc<dyn Fn(&mut Window, &mut App)>;

/// The overlays open on screen, the topmost last. Each one adds itself while it draws, and
/// the window forgets them before each frame: a key listener on the overlay's own element
/// would hear keys only while focus is inside it.
#[derive(Default)]
struct Open(Vec<Close>);

impl Global for Open {}

/// Escape closes this overlay while nothing above it is open.
pub fn close_on_escape(cx: &mut App, close: impl Fn(&mut Window, &mut App) + 'static) {
    cx.default_global::<Open>().0.push(Rc::new(close));
}

/// Called by the window before it draws.
pub fn begin_frame(cx: &mut App) {
    cx.default_global::<Open>().0.clear();
}

/// Closes the topmost overlay; false when none is open.
pub fn escape(window: &mut Window, cx: &mut App) -> bool {
    match cx.default_global::<Open>().0.pop() {
        Some(close) => {
            close(window, cx);
            window.refresh();
            true
        }
        None => false,
    }
}

#[derive(Clone, Copy, Default)]
struct State {
    open: bool,
    /// Where the press that closed the panel from outside went down: that same press must not
    /// reopen it when it lands on the trigger.
    dismissed: Option<Point<Pixels>>,
}

/// Whether a panel is open; listeners change it without going through a view.
#[derive(Clone, Default)]
pub struct Menu(Rc<Cell<State>>);

impl Menu {
    pub fn is_open(&self) -> bool {
        self.0.get().open
    }

    pub fn close(&self) {
        self.0.set(State::default());
    }

    /// The trigger's click: opens or closes the panel, unless this press just closed it.
    pub fn toggle(&self, e: &ClickEvent) {
        let state = self.0.get();
        let same_press =
            matches!(e, ClickEvent::Mouse(m) if Some(m.down.position) == state.dismissed);
        self.0.set(State {
            open: !state.open && !same_press,
            dismissed: None,
        });
    }
}

/// `panel` under the element this is placed in (give that one `.relative()`), while `menu` is
/// open and as it eases out.
pub fn popover(
    menu: &Menu,
    key: impl Into<ElementId>,
    panel: Div,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    place(menu, key, panel, false, window, cx)
}

/// The same panel above its trigger, for controls along the bottom of the screen.
pub fn popover_up(
    menu: &Menu,
    key: impl Into<ElementId>,
    panel: Div,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    place(menu, key, panel, true, window, cx)
}

fn place(
    menu: &Menu,
    key: impl Into<ElementId>,
    panel: Div,
    above: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let key = key.into();
    motion::set(
        key.clone(),
        menu.is_open(),
        Duration::from_millis(220),
        theme::EASE,
        cx,
    );
    let shown = motion::get(key, window, cx);
    if shown <= 0. {
        return None;
    }
    let (outside, escape) = (menu.clone(), menu.clone());
    if menu.is_open() {
        close_on_escape(cx, move |_, _| escape.close());
    }
    let panel = panel
        .occlude()
        .on_mouse_down_out(move |e, window, _| {
            if outside.is_open() {
                outside.0.set(State {
                    open: false,
                    dismissed: Some(e.position),
                });
                window.refresh();
            }
        })
        .p(px(6.))
        .rounded(theme::R_M)
        .bg(rgb(theme::SURFACE_2))
        .border_1()
        .border_color(theme::line())
        .shadow(vec![BoxShadow {
            color: rgba(0x00000080).into(),
            offset: point(px(0.), px(20.)),
            blur_radius: px(60.),
            spread_radius: px(0.),
            inset: false,
        }])
        .opacity(shown.min(1.))
        .relative()
        .top(px(if above { 6. } else { -6. } * (1. - shown)));
    let anchor = anchored().snap_to_window_with_margin(px(16.));
    Some(if above {
        div()
            .absolute()
            .left_0()
            .top_0()
            .child(deferred(
                anchor
                    .anchor(Anchor::BottomLeft)
                    .child(div().pb(px(8.)).child(panel)),
            ))
            .into_any_element()
    } else {
        div()
            .absolute()
            .left_0()
            .top(relative(1.))
            .child(deferred(anchor.child(div().pt(px(8.)).child(panel))))
            .into_any_element()
    })
}
