//! The part of libmpv's C API Nuvio uses, resolved from the DLL at run time.
//!
//! libmpv is loaded with `LoadLibrary` instead of being linked, so the app starts (and can explain what is missing)
//! when the DLL is not next to it, and no import library is needed to build.

use std::ffi::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use libloading::Library;

use crate::Error;

pub type Handle = *mut c_void;
pub type RenderCtx = *mut c_void;

pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;

pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_LOG_MESSAGE: c_int = 2;
pub const EVENT_START_FILE: c_int = 6;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_VIDEO_RECONFIG: c_int = 17;
pub const EVENT_SEEK: c_int = 20;
pub const EVENT_PLAYBACK_RESTART: c_int = 21;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const RENDER_PARAM_FLIP_Y: c_int = 4;
pub const RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
pub const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;

pub const RENDER_UPDATE_FRAME: u64 = 1;

#[repr(C)]
pub struct Event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct EventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct EventEndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[repr(C)]
pub struct EventLogMessage {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

#[repr(C)]
pub struct RenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct OpenGlInitParams {
    pub get_proc_address: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct OpenGlFbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

macro_rules! api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?),* $(,)?) => {
        pub struct Api {
            $(pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            _library: Library,
        }

        impl Api {
            fn from_library(library: Library) -> Result<Self, Error> {
                // SAFETY: the signatures below are copied from mpv/client.h and mpv/render.h.
                unsafe {
                    Ok(Self {
                        $($name: *library
                            .get(concat!(stringify!($name), "\0").as_bytes())
                            .map_err(|e| Error::Load(format!("{} is missing: {e}", stringify!($name))))?,)*
                        _library: library,
                    })
                }
            }
        }
    };
}

api! {
    mpv_client_api_version: fn() -> std::ffi::c_ulong,
    mpv_error_string: fn(c_int) -> *const c_char,
    mpv_free: fn(*mut c_void),
    mpv_create: fn() -> Handle,
    mpv_initialize: fn(Handle) -> c_int,
    mpv_terminate_destroy: fn(Handle),
    mpv_set_option_string: fn(Handle, *const c_char, *const c_char) -> c_int,
    mpv_get_property_string: fn(Handle, *const c_char) -> *mut c_char,
    mpv_get_property: fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    mpv_set_property_async: fn(Handle, u64, *const c_char, c_int, *mut c_void) -> c_int,
    mpv_command_async: fn(Handle, u64, *mut *const c_char) -> c_int,
    mpv_observe_property: fn(Handle, u64, *const c_char, c_int) -> c_int,
    mpv_wait_event: fn(Handle, f64) -> *mut Event,
    mpv_set_wakeup_callback: fn(Handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    mpv_request_log_messages: fn(Handle, *const c_char) -> c_int,
    mpv_render_context_create: fn(*mut RenderCtx, Handle, *mut RenderParam) -> c_int,
    mpv_render_context_set_update_callback: fn(RenderCtx, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    mpv_render_context_update: fn(RenderCtx) -> u64,
    mpv_render_context_render: fn(RenderCtx, *mut RenderParam) -> c_int,
    mpv_render_context_report_swap: fn(RenderCtx),
    mpv_render_context_free: fn(RenderCtx),
}

/// File names libmpv builds are published under.
const DLL_NAMES: [&str; 2] = ["libmpv-2.dll", "mpv-2.dll"];

/// Where to look, in order: `NUVIO_LIBMPV`, the folder of the running exe, `libs/windows-x64` above it (a source
/// checkout), then whatever the Windows loader finds on its own.
fn candidates() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(path) = std::env::var_os("NUVIO_LIBMPV") {
        found.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent();
        for depth in 0..5 {
            let Some(current) = dir else { break };
            for name in DLL_NAMES {
                found.push(current.join(name));
                if depth > 0 {
                    found.push(current.join("libs").join("windows-x64").join(name));
                }
            }
            dir = current.parent();
        }
    }
    for name in DLL_NAMES {
        found.push(PathBuf::from(name));
    }
    found
}

pub(crate) fn load() -> Result<Api, Error> {
    let mut last = String::from("libmpv-2.dll was not found");
    for path in candidates() {
        if path.components().count() > 1 && !Path::new(&path).exists() {
            continue;
        }
        // SAFETY: loading a DLL runs its initialisers; libmpv's are the normal ones.
        match unsafe { Library::new(&path) } {
            Ok(library) => {
                tracing::info!(path = %path.display(), "libmpv loaded");
                return Api::from_library(library);
            }
            Err(error) => last = format!("{}: {error}", path.display()),
        }
    }
    Err(Error::Load(last))
}
