//! Linux, Wayland: the video in a surface of its own under the window. libmpv draws it through
//! EGL on a thread of its own, and the window above is transparent where the video shows, so the
//! controls drawn by GPUI lie over it. The compositor puts the two together; no frame is copied.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context as _, Result, anyhow, bail};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};

use crate::mpv::{self, GlFbo, GlInit, Mpv, Param, RenderCtx};

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;

const EGL_PLATFORM_WAYLAND_KHR: u32 = 0x31D8;
const EGL_OPENGL_API: u32 = 0x30A2;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_BIT: i32 = 0x0008;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_NONE: i32 = 0x3038;

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetPlatformDisplay(
        platform: u32,
        native: *mut c_void,
        attribs: *const isize,
    ) -> EglDisplay;
    fn eglInitialize(dpy: EglDisplay, major: *mut i32, minor: *mut i32) -> u32;
    fn eglBindAPI(api: u32) -> u32;
    fn eglChooseConfig(
        dpy: EglDisplay,
        attribs: *const i32,
        configs: *mut EglConfig,
        size: i32,
        n: *mut i32,
    ) -> u32;
    fn eglCreateContext(
        dpy: EglDisplay,
        config: EglConfig,
        share: EglContext,
        attribs: *const i32,
    ) -> EglContext;
    fn eglCreatePlatformWindowSurface(
        dpy: EglDisplay,
        config: EglConfig,
        win: *mut c_void,
        attribs: *const isize,
    ) -> EglSurface;
    fn eglMakeCurrent(dpy: EglDisplay, draw: EglSurface, read: EglSurface, ctx: EglContext) -> u32;
    fn eglSwapBuffers(dpy: EglDisplay, surface: EglSurface) -> u32;
    fn eglSwapInterval(dpy: EglDisplay, interval: i32) -> u32;
    fn eglDestroySurface(dpy: EglDisplay, surface: EglSurface) -> u32;
    fn eglDestroyContext(dpy: EglDisplay, ctx: EglContext) -> u32;
    fn eglGetError() -> i32;
    fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
}

#[link(name = "wayland-egl")]
unsafe extern "C" {
    fn wl_egl_window_create(surface: *mut c_void, width: c_int, height: c_int) -> *mut c_void;
    fn wl_egl_window_resize(window: *mut c_void, width: c_int, height: c_int, dx: c_int, dy: c_int);
    fn wl_egl_window_destroy(window: *mut c_void);
}

unsafe extern "C" fn proc_address(_: *mut c_void, name: *const c_char) -> *mut c_void {
    unsafe { eglGetProcAddress(name) }
}

struct Globals;
impl Dispatch<WlRegistry, GlobalListContents> for Globals {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(Globals: ignore WlCompositor);
delegate_noop!(Globals: ignore WlSubcompositor);
delegate_noop!(Globals: ignore WlSurface);
delegate_noop!(Globals: ignore WlSubsurface);
delegate_noop!(Globals: ignore WlRegion);

/// What mpv's update callback wakes: the render thread, to draw a new frame.
struct Wake {
    pending: Mutex<bool>,
    ready: Condvar,
}

unsafe extern "C" fn on_update(wake: *mut c_void) {
    let wake = unsafe { &*(wake as *const Wake) };
    *wake.pending.lock().unwrap() = true;
    wake.ready.notify_one();
}

/// The video surface of a window, alive as long as this is.
pub struct Video {
    _conn: Connection,
    surface: WlSurface,
    subsurface: WlSubsurface,
    size: Arc<Mutex<(i32, i32)>>,
    stop: Arc<AtomicBool>,
    wake: Arc<Wake>,
    thread: Option<JoinHandle<()>>,
}

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}

impl Video {
    /// Puts a video surface under `window`, `width` x `height` device pixels, and starts drawing
    /// `mpv`'s frames into it.
    pub fn attach(
        window: &impl HasWindowHandle,
        display: &impl HasDisplayHandle,
        mpv: Arc<Mpv>,
        width: i32,
        height: i32,
    ) -> Result<Self> {
        let RawDisplayHandle::Wayland(d) = display
            .display_handle()
            .map_err(|e| anyhow!("{e}"))?
            .as_raw()
        else {
            bail!("the video surface needs Wayland");
        };
        let RawWindowHandle::Wayland(w) =
            window.window_handle().map_err(|e| anyhow!("{e}"))?.as_raw()
        else {
            bail!("the video surface needs Wayland");
        };
        let display_ptr = d.display.as_ptr();
        let conn =
            Connection::from_backend(unsafe { Backend::from_foreign_display(display_ptr.cast()) });
        let (globals, mut queue) =
            registry_queue_init::<Globals>(&conn).context("wayland registry")?;
        let qh = queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 4..=6, ()).context("wl_compositor")?;
        let subcompositor: WlSubcompositor =
            globals.bind(&qh, 1..=1, ()).context("wl_subcompositor")?;
        let parent_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), w.surface.as_ptr().cast()) }
                .map_err(|_| anyhow!("the window's surface is not a wl_surface"))?;
        let parent = WlSurface::from_id(&conn, parent_id)
            .map_err(|_| anyhow!("the window's surface is gone"))?;

        let surface = compositor.create_surface(&qh, ());
        let subsurface = subcompositor.get_subsurface(&surface, &parent, &qh, ());
        subsurface.place_below(&parent);
        subsurface.set_desync();
        subsurface.set_position(0, 0);
        // Clicks go through to the window above, where the controls are.
        let region = compositor.create_region(&qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        surface.commit();
        queue.roundtrip(&mut Globals).context("wayland roundtrip")?;

        let size = Arc::new(Mutex::new((width.max(1), height.max(1))));
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Wake {
            pending: Mutex::new(true),
            ready: Condvar::new(),
        });
        let (dpy, surf) = (SendPtr(display_ptr), SendPtr(surface.id().as_ptr().cast()));
        let thread = {
            let (size, stop, wake) = (size.clone(), stop.clone(), wake.clone());
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let thread = std::thread::Builder::new()
                .name("video".into())
                .spawn(move || {
                    let (dpy, surf) = (dpy, surf);
                    if let Err(e) =
                        render_loop(dpy.0, surf.0, &mpv, &size, &stop, &wake, &started_tx)
                    {
                        tracing::error!("video: {e:#}");
                        let _ = started_tx.send(Err(e));
                    }
                })?;
            started_rx
                .recv()
                .unwrap_or_else(|_| Err(anyhow!("the video thread ended")))?;
            thread
        };
        Ok(Video {
            _conn: conn,
            surface,
            subsurface,
            size,
            stop,
            wake,
            thread: Some(thread),
        })
    }

    pub fn resize(&self, width: i32, height: i32) {
        let mut size = self.size.lock().unwrap();
        if *size != (width, height) {
            *size = (width.max(1), height.max(1));
            *self.wake.pending.lock().unwrap() = true;
            self.wake.ready.notify_one();
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        *self.wake.pending.lock().unwrap() = true;
        self.wake.ready.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.subsurface.destroy();
        self.surface.destroy();
    }
}

fn render_loop(
    wl_display: *mut c_void,
    wl_surface: *mut c_void,
    mpv: &Mpv,
    size: &Mutex<(i32, i32)>,
    stop: &AtomicBool,
    wake: &Arc<Wake>,
    started: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    unsafe {
        let dpy = eglGetPlatformDisplay(EGL_PLATFORM_WAYLAND_KHR, wl_display, ptr::null());
        if dpy.is_null() || eglInitialize(dpy, ptr::null_mut(), ptr::null_mut()) == 0 {
            bail!("EGL did not start: {:#x}", eglGetError());
        }
        eglBindAPI(EGL_OPENGL_API);
        let attribs = [
            EGL_SURFACE_TYPE,
            EGL_WINDOW_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_NONE,
        ];
        let (mut config, mut n) = (ptr::null_mut(), 0);
        if eglChooseConfig(dpy, attribs.as_ptr(), &mut config, 1, &mut n) == 0 || n == 0 {
            bail!("no EGL config: {:#x}", eglGetError());
        }
        let ctx = eglCreateContext(dpy, config, ptr::null_mut(), [EGL_NONE].as_ptr());
        if ctx.is_null() {
            bail!("no OpenGL context: {:#x}", eglGetError());
        }
        let (mut w, mut h) = *size.lock().unwrap();
        let egl_window = wl_egl_window_create(wl_surface, w, h);
        let egl_surface = eglCreatePlatformWindowSurface(dpy, config, egl_window, ptr::null());
        if egl_surface.is_null() || eglMakeCurrent(dpy, egl_surface, egl_surface, ctx) == 0 {
            bail!("no EGL window surface: {:#x}", eglGetError());
        }
        // mpv paces the frames; waiting for the compositor here would stall a hidden window.
        eglSwapInterval(dpy, 0);

        let api = c"opengl";
        let mut init = GlInit {
            get_proc_address: proc_address,
            ctx: ptr::null_mut(),
        };
        let mut params = [
            Param {
                kind: mpv::PARAM_API_TYPE,
                data: api.as_ptr() as *mut c_void,
            },
            Param {
                kind: mpv::PARAM_OPENGL_INIT,
                data: &mut init as *mut GlInit as *mut c_void,
            },
            Param {
                kind: mpv::PARAM_WL_DISPLAY,
                data: wl_display,
            },
            Param {
                kind: mpv::PARAM_END,
                data: ptr::null_mut(),
            },
        ];
        let mut render: *mut RenderCtx = ptr::null_mut();
        let r = mpv::mpv_render_context_create(&mut render, mpv.raw(), params.as_mut_ptr());
        if r < 0 {
            bail!("mpv cannot draw here: {}", mpv::error(r));
        }
        mpv::mpv_render_context_set_update_callback(
            render,
            Some(on_update),
            Arc::as_ptr(wake) as *mut c_void,
        );
        let _ = started.send(Ok(()));

        let mut frames: u64 = 0;
        while !stop.load(Ordering::SeqCst) {
            {
                let mut pending = wake.pending.lock().unwrap();
                while !*pending {
                    pending = wake.ready.wait(pending).unwrap();
                }
                *pending = false;
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let (nw, nh) = *size.lock().unwrap();
            if (nw, nh) != (w, h) {
                (w, h) = (nw, nh);
                wl_egl_window_resize(egl_window, w, h, 0, 0);
            }
            let mut fbo = GlFbo {
                fbo: 0,
                w,
                h,
                internal_format: 0,
            };
            let mut flip: c_int = 1;
            let mut params = [
                Param {
                    kind: mpv::PARAM_OPENGL_FBO,
                    data: &mut fbo as *mut GlFbo as *mut c_void,
                },
                Param {
                    kind: mpv::PARAM_FLIP_Y,
                    data: &mut flip as *mut c_int as *mut c_void,
                },
                Param {
                    kind: mpv::PARAM_END,
                    data: ptr::null_mut(),
                },
            ];
            mpv::mpv_render_context_render(render, params.as_mut_ptr());
            eglSwapBuffers(dpy, egl_surface);
            frames += 1;
            if frames == 1 || frames % 600 == 0 {
                tracing::info!("video: {frames} frames drawn at {w}x{h}");
            }
        }
        mpv::mpv_render_context_free(render);
        eglMakeCurrent(dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
        eglDestroySurface(dpy, egl_surface);
        eglDestroyContext(dpy, ctx);
        wl_egl_window_destroy(egl_window);
    }
    Ok(())
}
