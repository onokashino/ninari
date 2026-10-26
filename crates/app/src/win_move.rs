//! Windows: moving the window costs nothing. GPUI answers every WM_MOVE with a full redraw of the
//! window (`Window::bounds_changed` refreshes it), and dragging a window sends one for every
//! pixel the pointer moves: the whole interface was laid out and painted again and again for a
//! picture that does not change. While the window is dragged its moves are held back; the last
//! one reaches GPUI when the drag ends, so it still knows where the window is.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::Relaxed};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

type Hwnd = isize;
type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

const GWLP_WNDPROC: i32 = -4;
const WM_MOVE: u32 = 0x0003;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;

#[link(name = "user32")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn CallWindowProcW(
        previous: isize,
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;
}

/// GPUI's own window procedure, which every message still reaches.
static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// The last move held back during a drag, as its lparam (the client area's position).
static HELD: AtomicIsize = AtomicIsize::new(0);
static HELD_ANY: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn procedure(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    let previous = PREVIOUS.load(Relaxed);
    match msg {
        WM_ENTERSIZEMOVE => DRAGGING.store(true, Relaxed),
        WM_MOVE if DRAGGING.load(Relaxed) => {
            HELD.store(lparam, Relaxed);
            HELD_ANY.store(true, Relaxed);
            return 0;
        }
        WM_EXITSIZEMOVE => {
            DRAGGING.store(false, Relaxed);
            if HELD_ANY.swap(false, Relaxed) {
                unsafe { CallWindowProcW(previous, hwnd, WM_MOVE, 0, HELD.load(Relaxed)) };
            }
        }
        _ => {}
    }
    unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) }
}

/// Puts the procedure above in front of GPUI's for `window`.
pub fn hold_moves(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get();
    let ours = procedure as WndProc as isize;
    let previous = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
    // Never chained to itself; known before any message can reach ours.
    if previous == 0 || previous == ours {
        return;
    }
    PREVIOUS.store(previous, Relaxed);
    unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, ours) };
}
