//! A grid of cards that builds only the rows near the screen; the others are empty boxes of a
//! row's height. Every card of a grid was laid out and painted each frame, so a catalog scrolled
//! a few hundred titles deep took 50 ms a frame; this way a long list costs what a screenful does.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{AnyElement, IntoElement, ParentElement, Pixels, Styled, Window, canvas, div, px};

/// The window's height, which decides how many rows are built.
pub fn screen(window: &Window) -> f32 {
    f32::from(window.viewport_size().height)
}

/// Where the grid's top was on the last frame, in window coordinates; the page keeps it.
#[derive(Clone, Default)]
pub struct GridTop(Rc<Cell<Option<Pixels>>>);

pub const GAP_X: f32 = 24.;
/// Small: a row already leaves room for three lines of title under each card.
pub const GAP_Y: f32 = 12.;

/// The height of a row of poster cards `card_w` wide: the poster, then room for three lines of
/// title and two of notes.
pub fn row_height(card_w: f32) -> f32 {
    (card_w * 1.5).round() + 8. + 51. + 36.
}

/// `count` cards in rows of `columns`, each row `row_h` tall; `card(i)` builds the i-th, only for
/// the rows within about a screen of the window.
pub fn lazy_grid(
    top: &GridTop,
    count: usize,
    columns: usize,
    row_h: f32,
    screen: f32,
    mut card: impl FnMut(usize) -> AnyElement,
) -> AnyElement {
    let columns = columns.max(1);
    let rows = count.div_ceil(columns);
    let shown = visible_rows(top.0.get().map(f32::from), rows, row_h, screen);
    let measure = top.0.clone();
    div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(GAP_Y))
        .child(
            canvas(
                move |bounds, _, _| measure.set(Some(bounds.top())),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .children((0..rows).map(|row| {
            let line = div().h(px(row_h)).flex().gap(px(GAP_X));
            if !shown.contains(&row) {
                return line.into_any_element();
            }
            let end = ((row + 1) * columns).min(count);
            line.children((row * columns..end).map(&mut card))
                .into_any_element()
        }))
        .into_any_element()
}

/// The rows to build: those within half a screen above or below the window, where the grid's top was
/// at `top` (all of them before the first frame measured it).
fn visible_rows(top: Option<f32>, rows: usize, row_h: f32, screen: f32) -> std::ops::Range<usize> {
    let Some(top) = top else {
        return 0..rows;
    };
    let step = row_h + GAP_Y;
    // Half a screen either side: a fast fling shows no empty row before the next frame.
    let first = ((-top - screen / 2.) / step).floor().max(0.) as usize;
    let last = ((-top + 1.5 * screen) / step).ceil().max(0.) as usize;
    first.min(rows)..last.min(rows)
}

#[cfg(test)]
mod tests {
    use super::visible_rows;

    #[test]
    fn only_the_rows_near_the_screen_are_built() {
        // Rows 300 + 12 apart, a screen 1000 tall.
        assert_eq!(visible_rows(None, 50, 300., 1000.), 0..50);
        assert_eq!(visible_rows(Some(200.), 50, 300., 1000.), 0..5);
        // Scrolled 10 000 down: rows 32..35 on screen, plus half a screen each side.
        assert_eq!(visible_rows(Some(-10_000.), 50, 300., 1000.), 30..37);
        assert_eq!(visible_rows(Some(-100_000.), 50, 300., 1000.), 50..50);
    }
}
