//! Safe wrappers around libmpv: a player handle with an event queue, and an OpenGL render context that draws the
//! current frame into the framebuffer of the host window.

mod gl;
mod sys;

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

pub use gl::proc_address as gl_proc_address;

const GL_RGBA8: c_int = 0x8058;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("libmpv could not be loaded: {0}")]
    Load(String),
    #[error("mpv: {message} ({code})")]
    Mpv { code: i32, message: String },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn api() -> Result<&'static Arc<sys::Api>> {
    static API: OnceLock<std::result::Result<Arc<sys::Api>, String>> = OnceLock::new();
    API.get_or_init(|| sys::load().map(Arc::new).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|message| Error::Load(message.clone()))
}

/// Whether libmpv can be loaded at all. Lets the UI show an explanation instead of failing when a stream is opened.
pub fn available() -> Result<()> {
    api().map(|_| ())
}

fn cstring(text: &str) -> CString {
    // A NUL inside a URL or option would end the C string early; dropping it is the only sane reading.
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

fn check(api: &sys::Api, code: c_int) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: mpv_error_string returns a static string for every code.
    let message = unsafe { CStr::from_ptr((api.mpv_error_string)(code)) }
        .to_string_lossy()
        .into_owned();
    Err(Error::Mpv { code, message })
}

/// A value carried by a property change.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Flag(bool),
    Int(i64),
    Double(f64),
    Text(String),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Flag(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(v) => Some(v),
            _ => None,
        }
    }
}

/// How a property is delivered when it is observed.
#[derive(Debug, Clone, Copy)]
pub enum Observe {
    Flag,
    Int,
    Double,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Other,
}

#[derive(Debug, Clone)]
pub enum Event {
    StartFile,
    FileLoaded,
    EndFile { reason: EndReason, error: Option<String> },
    VideoReconfig,
    Seek,
    PlaybackRestart,
    Property { name: String, value: Value },
    Log { level: String, text: String },
    Shutdown,
}

struct Wakeup(Box<dyn Fn() + Send + Sync>);

unsafe extern "C" fn wakeup_trampoline(context: *mut c_void) {
    // SAFETY: `context` is the Box<Wakeup> kept alive by the owning `Mpv` until the callback is cleared.
    let wakeup = unsafe { &*(context as *const Wakeup) };
    (wakeup.0)();
}

/// One libmpv player instance. Methods take `&self`: the client API is thread safe.
pub struct Mpv {
    api: Arc<sys::Api>,
    handle: sys::Handle,
    _wakeup: Box<Wakeup>,
}

// SAFETY: libmpv documents mpv_handle as usable from any thread.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

impl Mpv {
    /// Creates and initialises a player. `options` are set before initialisation (`vo`, `hwdec`, caches, ...);
    /// `wakeup` runs on an mpv thread whenever events are waiting, so it must only schedule work (a repaint).
    pub fn new(options: &[(&str, &str)], wakeup: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let api = api()?.clone();
        let wakeup = Box::new(Wakeup(Box::new(wakeup)));
        // SAFETY: plain calls into the C API with valid, NUL terminated strings.
        unsafe {
            let handle = (api.mpv_create)();
            if handle.is_null() {
                return Err(Error::Other("mpv_create returned null".into()));
            }
            for (name, value) in options {
                let (n, v) = (cstring(name), cstring(value));
                let code = (api.mpv_set_option_string)(handle, n.as_ptr(), v.as_ptr());
                if code < 0 {
                    tracing::warn!(option = name, value, code, "mpv rejected an option");
                }
            }
            if let Err(error) = check(&api, (api.mpv_initialize)(handle)) {
                (api.mpv_terminate_destroy)(handle);
                return Err(error);
            }
            let context = &*wakeup as *const Wakeup as *mut c_void;
            (api.mpv_set_wakeup_callback)(handle, Some(wakeup_trampoline), context);
            (api.mpv_request_log_messages)(handle, c"warn".as_ptr());
            Ok(Self { api, handle, _wakeup: wakeup })
        }
    }

    /// Which mpv log messages arrive as `Event::Log`: `fatal`, `error`, `warn`, `info`, `v`, `debug`, `trace`.
    pub fn request_log_level(&self, level: &str) {
        let level = cstring(level);
        // SAFETY: valid string; mpv copies it.
        unsafe { (self.api.mpv_request_log_messages)(self.handle, level.as_ptr()) };
    }

    pub fn version(&self) -> String {
        // SAFETY: no arguments.
        let v = unsafe { (self.api.mpv_client_api_version)() } as u32;
        format!("{}.{}", v >> 16, v & 0xFFFF)
    }

    /// Runs a command without waiting for it (`loadfile`, `seek`, `cycle`, ...).
    pub fn command(&self, args: &[&str]) -> Result<()> {
        let owned: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        pointers.push(std::ptr::null());
        // SAFETY: `pointers` is NULL terminated and `owned` outlives the call; mpv copies the arguments.
        let code = unsafe { (self.api.mpv_command_async)(self.handle, 0, pointers.as_mut_ptr()) };
        check(&self.api, code)
    }

    /// Sets a property without waiting for the player core to apply it.
    pub fn set_str(&self, name: &str, value: &str) -> Result<()> {
        let (n, v) = (cstring(name), cstring(value));
        let mut data: *const c_char = v.as_ptr();
        // SAFETY: for MPV_FORMAT_STRING the data is a pointer to the string pointer; mpv copies it before returning.
        let code = unsafe {
            (self.api.mpv_set_property_async)(self.handle, 0, n.as_ptr(), sys::FORMAT_STRING, &mut data as *mut *const c_char as *mut c_void)
        };
        check(&self.api, code)
    }

    pub fn set_f64(&self, name: &str, mut value: f64) -> Result<()> {
        let n = cstring(name);
        // SAFETY: `value` outlives the call, and the async setter copies it before returning.
        let code = unsafe {
            (self.api.mpv_set_property_async)(self.handle, 0, n.as_ptr(), sys::FORMAT_DOUBLE, &mut value as *mut f64 as *mut c_void)
        };
        check(&self.api, code)
    }

    pub fn set_bool(&self, name: &str, value: bool) -> Result<()> {
        let n = cstring(name);
        let mut flag: c_int = value.into();
        // SAFETY: as above.
        let code = unsafe {
            (self.api.mpv_set_property_async)(self.handle, 0, n.as_ptr(), sys::FORMAT_FLAG, &mut flag as *mut c_int as *mut c_void)
        };
        check(&self.api, code)
    }

    /// Reads a property as text. Structured properties (`track-list`, `video-params`) come back as JSON.
    ///
    /// The getters wait for the player core to answer, and the core can be waiting for the video output, which only
    /// starts when the thread that renders calls `RenderContext::render`. Never call them on that thread; observe the
    /// property and read the events instead.
    pub fn get_str(&self, name: &str) -> Option<String> {
        let n = cstring(name);
        // SAFETY: the returned string is owned by us and released with mpv_free.
        unsafe {
            let raw = (self.api.mpv_get_property_string)(self.handle, n.as_ptr());
            if raw.is_null() {
                return None;
            }
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            (self.api.mpv_free)(raw as *mut c_void);
            Some(text)
        }
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let n = cstring(name);
        let mut value = 0f64;
        // SAFETY: `value` is a valid f64 for MPV_FORMAT_DOUBLE.
        let code = unsafe {
            (self.api.mpv_get_property)(self.handle, n.as_ptr(), sys::FORMAT_DOUBLE, &mut value as *mut f64 as *mut c_void)
        };
        (code >= 0).then_some(value)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let n = cstring(name);
        let mut value = 0i64;
        // SAFETY: `value` is a valid i64 for MPV_FORMAT_INT64.
        let code = unsafe {
            (self.api.mpv_get_property)(self.handle, n.as_ptr(), sys::FORMAT_INT64, &mut value as *mut i64 as *mut c_void)
        };
        (code >= 0).then_some(value)
    }

    pub fn get_bool(&self, name: &str) -> Option<bool> {
        let n = cstring(name);
        let mut value: c_int = 0;
        // SAFETY: `value` is a valid int for MPV_FORMAT_FLAG.
        let code = unsafe {
            (self.api.mpv_get_property)(self.handle, n.as_ptr(), sys::FORMAT_FLAG, &mut value as *mut c_int as *mut c_void)
        };
        (code >= 0).then_some(value != 0)
    }

    /// Asks for an `Event::Property` whenever `name` changes. The format decides which `Value` variant arrives.
    pub fn observe(&self, name: &str, format: Observe) {
        let n = cstring(name);
        let format = match format {
            Observe::Flag => sys::FORMAT_FLAG,
            Observe::Int => sys::FORMAT_INT64,
            Observe::Double => sys::FORMAT_DOUBLE,
            Observe::Text => sys::FORMAT_STRING,
        };
        // SAFETY: valid name string.
        unsafe { (self.api.mpv_observe_property)(self.handle, 0, n.as_ptr(), format) };
    }

    /// Drains the event queue without blocking.
    pub fn poll_events(&self, out: &mut Vec<Event>) {
        loop {
            // SAFETY: mpv_wait_event returns a pointer valid until the next call on this handle.
            let event = unsafe { &*(self.api.mpv_wait_event)(self.handle, 0.0) };
            let converted = match event.event_id {
                sys::EVENT_NONE => return,
                sys::EVENT_SHUTDOWN => Some(Event::Shutdown),
                sys::EVENT_START_FILE => Some(Event::StartFile),
                sys::EVENT_FILE_LOADED => Some(Event::FileLoaded),
                sys::EVENT_VIDEO_RECONFIG => Some(Event::VideoReconfig),
                sys::EVENT_SEEK => Some(Event::Seek),
                sys::EVENT_PLAYBACK_RESTART => Some(Event::PlaybackRestart),
                sys::EVENT_END_FILE => {
                    // SAFETY: END_FILE events carry an mpv_event_end_file.
                    let end = unsafe { &*(event.data as *const sys::EventEndFile) };
                    let reason = match end.reason {
                        0 => EndReason::Eof,
                        2 => EndReason::Stop,
                        3 => EndReason::Quit,
                        4 => EndReason::Error,
                        5 => EndReason::Redirect,
                        _ => EndReason::Other,
                    };
                    let error = (end.error < 0).then(|| match check(&self.api, end.error) {
                        Err(Error::Mpv { message, .. }) => message,
                        _ => "playback failed".into(),
                    });
                    Some(Event::EndFile { reason, error })
                }
                sys::EVENT_LOG_MESSAGE => {
                    // SAFETY: LOG_MESSAGE events carry an mpv_event_log_message with valid strings.
                    let log = unsafe { &*(event.data as *const sys::EventLogMessage) };
                    let (level, text) = unsafe {
                        (
                            CStr::from_ptr(log.level).to_string_lossy().into_owned(),
                            CStr::from_ptr(log.text).to_string_lossy().trim_end().to_owned(),
                        )
                    };
                    Some(Event::Log { level, text })
                }
                sys::EVENT_PROPERTY_CHANGE => {
                    // SAFETY: PROPERTY_CHANGE events carry an mpv_event_property; `data` matches `format`.
                    let property = unsafe { &*(event.data as *const sys::EventProperty) };
                    let name = unsafe { CStr::from_ptr(property.name) }.to_string_lossy().into_owned();
                    let value = if property.data.is_null() {
                        Value::None
                    } else {
                        unsafe {
                            match property.format {
                                sys::FORMAT_FLAG => Value::Flag(*(property.data as *const c_int) != 0),
                                sys::FORMAT_INT64 => Value::Int(*(property.data as *const i64)),
                                sys::FORMAT_DOUBLE => Value::Double(*(property.data as *const f64)),
                                sys::FORMAT_STRING => {
                                    let text = *(property.data as *const *const c_char);
                                    if text.is_null() {
                                        Value::None
                                    } else {
                                        Value::Text(CStr::from_ptr(text).to_string_lossy().into_owned())
                                    }
                                }
                                _ => Value::None,
                            }
                        }
                    };
                    Some(Event::Property { name, value })
                }
                _ => None,
            };
            if let Some(converted) = converted {
                out.push(converted);
            }
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let api = self.api.clone();
        let handle = self.handle as usize;
        // SAFETY: stop callbacks into `_wakeup` before it is freed. mpv_terminate_destroy waits for playback to stop,
        // which on a stalled network stream can take seconds, so it runs off the UI thread.
        unsafe { (api.mpv_set_wakeup_callback)(self.handle, None, std::ptr::null_mut()) };
        std::thread::Builder::new()
            .name("mpv-destroy".into())
            .spawn(move || unsafe { (api.mpv_terminate_destroy)(handle as sys::Handle) })
            .ok();
    }
}

struct UpdateCallback {
    changed: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

unsafe extern "C" fn render_update_trampoline(context: *mut c_void) {
    // SAFETY: `context` points into the Arc<UpdateCallback> held by the RenderContext until it is freed.
    let callback = unsafe { &*(context as *const UpdateCallback) };
    callback.changed.store(true, Ordering::Release);
    (callback.wake)();
}

/// Draws mpv's current frame into an OpenGL framebuffer. Must be created, used and dropped with the window's GL
/// context current.
pub struct RenderContext {
    api: Arc<sys::Api>,
    ctx: sys::RenderCtx,
    callback: Arc<UpdateCallback>,
    /// mpv must outlive its render context: freeing the context after `mpv_terminate_destroy` has shut the core down
    /// crashes inside libmpv. Declared last, so it is released after `Drop::drop` has freed the context.
    _mpv: Arc<Mpv>,
}

impl RenderContext {
    /// `wake` runs on an mpv thread when a new frame is ready; it should request a repaint.
    pub fn new_opengl(mpv: &Arc<Mpv>, wake: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let api = mpv.api.clone();
        let callback = Arc::new(UpdateCallback { changed: AtomicBool::new(true), wake: Box::new(wake) });
        let mut init = sys::OpenGlInitParams { get_proc_address: Some(gl::loader), get_proc_address_ctx: std::ptr::null_mut() };
        let mut advanced: c_int = 1;
        let mut params = [
            sys::RenderParam { kind: sys::RENDER_PARAM_API_TYPE, data: c"opengl".as_ptr() as *mut c_void },
            sys::RenderParam { kind: sys::RENDER_PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
            sys::RenderParam { kind: sys::RENDER_PARAM_ADVANCED_CONTROL, data: &mut advanced as *mut c_int as *mut c_void },
            sys::RenderParam { kind: 0, data: std::ptr::null_mut() },
        ];
        let mut ctx: sys::RenderCtx = std::ptr::null_mut();
        // SAFETY: `params` is terminated with an invalid entry and points at live locals.
        let code = unsafe { (api.mpv_render_context_create)(&mut ctx, mpv.handle, params.as_mut_ptr()) };
        check(&api, code)?;
        let raw = Arc::as_ptr(&callback) as *mut c_void;
        // SAFETY: the Arc in `callback` keeps the pointee alive until `Drop` clears the callback.
        unsafe { (api.mpv_render_context_set_update_callback)(ctx, Some(render_update_trampoline), raw) };
        Ok(Self { api, ctx, callback, _mpv: mpv.clone() })
    }

    /// True when mpv has a frame that was not drawn yet. Must be called once per paint, before `render`, because
    /// the advanced control mode asks the host to poll.
    pub fn take_update(&self) -> bool {
        // SAFETY: valid context.
        let flags = unsafe { (self.api.mpv_render_context_update)(self.ctx) };
        let signalled = self.callback.changed.swap(false, Ordering::AcqRel);
        flags & sys::RENDER_UPDATE_FRAME != 0 || signalled
    }

    /// Draws into `fbo` (0 is the window's back buffer) of the given pixel size. mpv sets its own viewport.
    pub fn render(&self, fbo: i32, width: i32, height: i32, flip_y: bool) -> Result<()> {
        // gpu-next (libplacebo) has to know the format behind the default framebuffer; the window's is RGBA8.
        let mut target = sys::OpenGlFbo { fbo, w: width, h: height, internal_format: GL_RGBA8 };
        let mut flip: c_int = flip_y.into();
        let mut block: c_int = 0;
        let mut params = [
            sys::RenderParam { kind: sys::RENDER_PARAM_OPENGL_FBO, data: &mut target as *mut _ as *mut c_void },
            sys::RenderParam { kind: sys::RENDER_PARAM_FLIP_Y, data: &mut flip as *mut c_int as *mut c_void },
            sys::RenderParam { kind: sys::RENDER_PARAM_BLOCK_FOR_TARGET_TIME, data: &mut block as *mut c_int as *mut c_void },
            sys::RenderParam { kind: 0, data: std::ptr::null_mut() },
        ];
        // SAFETY: valid context and parameter list.
        check(&self.api, unsafe { (self.api.mpv_render_context_render)(self.ctx, params.as_mut_ptr()) })
    }

    pub fn report_swap(&self) {
        // SAFETY: valid context.
        unsafe { (self.api.mpv_render_context_report_swap)(self.ctx) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: the update callback must not fire after `callback` is released; freeing the context needs the GL
        // context current, which the owner guarantees.
        unsafe {
            (self.api.mpv_render_context_set_update_callback)(self.ctx, None, std::ptr::null_mut());
            (self.api.mpv_render_context_free)(self.ctx);
        }
    }
}
