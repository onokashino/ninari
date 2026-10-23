//! libmpv inside the app: its client API to control playback and its OpenGL render API to draw
//! frames where we say.

use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};

use anyhow::{Result, bail};

#[repr(C)]
pub struct Handle(u8);
#[repr(C)]
pub struct RenderCtx(u8);
#[repr(C)]
pub struct Param {
    pub kind: c_int,
    pub data: *mut c_void,
}
#[repr(C)]
pub struct GlInit {
    pub get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    pub ctx: *mut c_void,
}
#[repr(C)]
pub struct GlFbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

pub const PARAM_END: c_int = 0;
pub const PARAM_API_TYPE: c_int = 1;
pub const PARAM_OPENGL_INIT: c_int = 2;
pub const PARAM_OPENGL_FBO: c_int = 3;
pub const PARAM_FLIP_Y: c_int = 4;
pub const PARAM_WL_DISPLAY: c_int = 9;
const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;
const LC_NUMERIC: c_int = 1;

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut Handle;
    fn mpv_set_option_string(h: *mut Handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_initialize(h: *mut Handle) -> c_int;
    fn mpv_terminate_destroy(h: *mut Handle);
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_command(h: *mut Handle, args: *mut *const c_char) -> c_int;
    fn mpv_set_property_string(h: *mut Handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_get_property(
        h: *mut Handle,
        name: *const c_char,
        format: c_int,
        data: *mut c_void,
    ) -> c_int;
    pub fn mpv_render_context_create(
        res: *mut *mut RenderCtx,
        h: *mut Handle,
        params: *mut Param,
    ) -> c_int;
    pub fn mpv_render_context_set_update_callback(
        ctx: *mut RenderCtx,
        cb: Option<unsafe extern "C" fn(*mut c_void)>,
        data: *mut c_void,
    );
    pub fn mpv_render_context_render(ctx: *mut RenderCtx, params: *mut Param) -> c_int;
    pub fn mpv_render_context_free(ctx: *mut RenderCtx);
}

unsafe extern "C" {
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

pub fn error(code: c_int) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)) }
        .to_string_lossy()
        .into_owned()
}

/// An mpv core. Its client API is thread-safe, so it is shared across threads.
pub struct Mpv(*mut Handle);

unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

fn c(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

impl Mpv {
    /// A core drawing through the render API (`vo=libmpv`) with these options.
    pub fn new(options: &[(&str, &str)]) -> Result<Self> {
        // libmpv formats numbers as C does and refuses to start otherwise.
        unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
        let h = unsafe { mpv_create() };
        if h.is_null() {
            bail!("libmpv did not start");
        }
        let mpv = Mpv(h);
        for (k, v) in options.iter().copied().chain([("vo", "libmpv")]) {
            let r = unsafe { mpv_set_option_string(h, c(k).as_ptr(), c(v).as_ptr()) };
            if r < 0 {
                tracing::warn!("mpv option {k}={v}: {}", error(r));
            }
        }
        let r = unsafe { mpv_initialize(h) };
        if r < 0 {
            bail!("libmpv did not start: {}", error(r));
        }
        Ok(mpv)
    }

    pub fn raw(&self) -> *mut Handle {
        self.0
    }

    pub fn command(&self, args: &[&str]) -> Result<()> {
        let owned: Vec<CString> = args.iter().map(|a| c(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let r = unsafe { mpv_command(self.0, ptrs.as_mut_ptr()) };
        if r < 0 {
            bail!("mpv {}: {}", args.first().unwrap_or(&""), error(r));
        }
        Ok(())
    }

    pub fn set(&self, name: &str, value: &str) {
        let r = unsafe { mpv_set_property_string(self.0, c(name).as_ptr(), c(value).as_ptr()) };
        if r < 0 {
            tracing::warn!("mpv set {name}={value}: {}", error(r));
        }
    }

    pub fn number(&self, name: &str) -> Option<f64> {
        let mut v: c_double = 0.0;
        let r = unsafe {
            mpv_get_property(
                self.0,
                c(name).as_ptr(),
                FORMAT_DOUBLE,
                &mut v as *mut c_double as *mut c_void,
            )
        };
        (r >= 0).then_some(v)
    }

    pub fn flag(&self, name: &str) -> Option<bool> {
        let mut v: c_int = 0;
        let r = unsafe {
            mpv_get_property(
                self.0,
                c(name).as_ptr(),
                FORMAT_FLAG,
                &mut v as *mut c_int as *mut c_void,
            )
        };
        (r >= 0).then_some(v != 0)
    }
}

struct SendHandle(*mut Handle);
unsafe impl Send for SendHandle {}

impl Drop for Mpv {
    /// Shutting the core down waits for its threads, a network stream's included: not on the
    /// thread that draws the window.
    fn drop(&mut self) {
        let h = SendHandle(self.0);
        std::thread::spawn(move || {
            let h = h; // the whole Send wrapper, not its raw field
            unsafe { mpv_terminate_destroy(h.0) }
        });
    }
}
