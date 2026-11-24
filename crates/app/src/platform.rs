//! What the process asks of Windows so that a video does not stutter when its window is not the one in front.
//!
//! Windows 10 and 11 give a process that is not in the foreground less of the machine: its threads may be moved to the slow
//! cores at a low clock ("power throttling", EcoQoS), timers that were asked to tick every millisecond are rounded to 15
//! milliseconds, and the foreground window's threads are favoured over everyone else's when the processor is busy (a
//! browser, a download, a virus scan). A video has to deliver a picture every 17 to 42 milliseconds whichever window is in
//! front, so the process opts out of the first two and, while a stream is open, takes a little more of the third.

use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcess, NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
};

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
