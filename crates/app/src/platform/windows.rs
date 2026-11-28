//! What the process asks of Windows so that a video does not stutter when its window is not the one in front.
//!
//! Windows 10 and 11 give a process that is not in the foreground less of the machine: its threads may be moved to the slow
//! cores at a low clock ("power throttling", EcoQoS), timers that were asked to tick every millisecond are rounded to 15
//! milliseconds, and the foreground window's threads are favoured over everyone else's when the processor is busy (a
//! browser, a download, a virus scan). A video has to deliver a picture every 17 to 42 milliseconds whichever window is in
//! front, so the process opts out of the first two and, while a stream is open, takes a little more of the third.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
use windows_sys::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcess, NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, MB_ICONERROR, MessageBoxW, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

pub use super::compositor::Compositor as FramePacer;
use super::{ScreenRect, Screens};

pub const NAME: &str = "Windows";

static PLAYING: AtomicBool = AtomicBool::new(false);

/// Called once at startup: the process is never throttled for being in the background.
pub fn start() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        // A control bit with its state bit clear means "never throttle this".
        StateMask: 0,
    };
    // SAFETY: the pointer and size describe `state`, which outlives the call; the pseudo handle needs no closing. Older
    // Windows versions do not know the request and refuse it, which only means there is nothing to opt out of.
    let done = unsafe { SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, (&raw const state).cast(), size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32) };
    tracing::debug!(opted_out = done != 0, "power throttling");
}

/// A stream is open (`true`) or closed (`false`): while it is open the timers tick every millisecond and the process runs
/// above the normal priority. Both are given back when playback ends, so Home costs nothing.
pub fn playback(on: bool) {
    if PLAYING.swap(on, Ordering::AcqRel) == on {
        return;
    }
    // SAFETY: plain calls with constants; every `timeBeginPeriod` is matched by a `timeEndPeriod` with the same value.
    unsafe {
        if on {
            timeBeginPeriod(1);
            SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
        } else {
            SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS);
            timeEndPeriod(1);
        }
    }
}

static AWAKE: AtomicBool = AtomicBool::new(false);

/// A picture is moving (`true`): neither the display nor the machine goes to sleep for lack of input. The request
/// belongs to the calling thread, so this is only called from the one that draws.
pub fn keep_display_awake(on: bool) {
    if AWAKE.swap(on, Ordering::AcqRel) == on {
        return;
    }
    let state = if on { ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
    // SAFETY: a plain call with constants.
    unsafe { SetThreadExecutionState(state) };
}

/// The desktop in physical pixels over all screens, and the size of the main one. Coordinates saved in points are never
/// larger than the pixel coordinates they came from, so checking them against pixel bounds can only be too lenient,
/// never wrong in a way that hides the window.
pub fn screens() -> Screens {
    // SAFETY: GetSystemMetrics takes no pointers.
    unsafe {
        let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN) as f32, GetSystemMetrics(SM_YVIRTUALSCREEN) as f32);
        let all = ScreenRect { left: x, top: y, right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32, bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32 };
        Screens { areas: vec![all], primary: (GetSystemMetrics(SM_CXSCREEN) as f32, GetSystemMetrics(SM_CYSCREEN) as f32) }
    }
}

pub fn use_runtime_icon() -> bool {
    true
}

/// The exe is a GUI program, so Windows gives it no console. When it was started from a terminal, borrow that one.
pub fn attach_console() -> bool {
    // SAFETY: no pointers involved; failure just means there is no parent console.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

pub fn show_fatal_error(title: &str, text: &str) {
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let (title, text) = (wide(title), wide(text));
    // SAFETY: both strings are NUL terminated and outlive the call.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR);
    }
}

pub fn open_url(url: &str) -> bool {
    // `rundll32 url.dll` hands the address to the default browser without a shell, so `&` in a URL stays literal.
    std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn().is_ok()
}

pub fn open_folder(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

pub fn edit_text_file(path: &Path) {
    let _ = std::process::Command::new("notepad").arg(path).spawn();
}
