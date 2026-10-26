//! Windows: the video in a child window under the app's own drawing. mpv draws into it (`--wid`),
//! and GPUI composes the window above its children (DirectComposition), so the controls lie over
//! the video wherever the window is transparent. The child is disabled: clicks go to the window.

use std::ffi::c_void;

use anyhow::{Result, bail};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

type Hwnd = isize;

const WS_CHILD: u32 = 0x4000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_DISABLED: u32 = 0x0800_0000;
/// A black rectangle until mpv draws.
const SS_BLACKRECT: u32 = 0x4;
const HWND_BOTTOM: Hwnd = 1;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SW_HIDE: i32 = 0;

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: isize,
        instance: isize,
        param: *mut c_void,
    ) -> Hwnd;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
}

pub struct VideoWindow {
    hwnd: Hwnd,
    /// The size it was last shown at, in device pixels.
    shown: Option<(i32, i32)>,
}

impl VideoWindow {
    /// A hidden child of `window`, made on the window's thread so that thread runs its messages.
    pub fn new(window: &impl HasWindowHandle) -> Result<Self> {
        let RawWindowHandle::Win32(parent) = window.window_handle()?.as_raw() else {
            bail!("not a Win32 window");
        };
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_CLIPSIBLINGS | WS_DISABLED | SS_BLACKRECT,
                0,
                0,
                1,
                1,
                parent.hwnd.get(),
                0,
                0,
                std::ptr::null_mut(),
            )
        };
        if hwnd == 0 {
            bail!("CreateWindowExW: {}", std::io::Error::last_os_error());
        }
        Ok(Self { hwnd, shown: None })
    }

    /// The handle mpv draws into.
    pub fn handle(&self) -> isize {
        self.hwnd
    }

    /// Shown over the whole window, under everything GPUI draws.
    pub fn show(&mut self, width: i32, height: i32) {
        if self.shown == Some((width, height)) {
            return;
        }
        self.shown = Some((width, height));
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_BOTTOM,
                0,
                0,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
    }

    pub fn hide(&mut self) {
        if self.shown.take().is_some() {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
    }
}

impl Drop for VideoWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}
