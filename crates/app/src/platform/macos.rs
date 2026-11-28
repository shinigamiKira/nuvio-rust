//! macOS.
//!
//! A process that is not in front, or whose window is covered, is napped: its timers are held back and its threads get
//! less of the machine. While a stream is open the app declares an activity, which opts out of that, and while a picture
//! is moving a second one keeps the display from sleeping. Frames need no pacing of their own: the OpenGL swap waits for
//! the display.

use std::cell::RefCell;
use std::path::Path;
use std::process::Command;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

use super::{ScreenRect, Screens, is_web_url};

pub const NAME: &str = "macOS";

/// An open file limit that lets the torrent engine talk to a swarm. Programs started from the Finder get 256.
const WANTED_OPEN_FILES: libc::rlim_t = 8192;

pub fn start() {
    // SAFETY: both calls only read or write the `rlimit` they are given.
    unsafe {
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
            return;
        }
        let wanted = WANTED_OPEN_FILES.min(limit.rlim_max);
        if limit.rlim_cur >= wanted {
            return;
        }
        limit.rlim_cur = wanted;
        let done = libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0;
        tracing::debug!(done, wanted, "open file limit");
    }
}

/// An activity that is ended when it is dropped.
struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

impl Activity {
    fn begin(options: NSActivityOptions, reason: &str) -> Self {
        Self(NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &NSString::from_str(reason)))
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // SAFETY: the object came from `beginActivityWithOptions:reason:` and is ended once.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}

thread_local! {
    // Both are only touched by the thread that draws.
    static PLAYBACK: RefCell<Option<Activity>> = const { RefCell::new(None) };
    static DISPLAY: RefCell<Option<Activity>> = const { RefCell::new(None) };
}

fn hold(slot: &'static std::thread::LocalKey<RefCell<Option<Activity>>>, on: bool, options: NSActivityOptions, reason: &str) {
    slot.with_borrow_mut(|held| {
        if on != held.is_some() {
            *held = on.then(|| Activity::begin(options, reason));
            tracing::debug!(on, reason, "activity");
        }
    });
}

/// A stream is open (`true`) or closed (`false`).
pub fn playback(on: bool) {
    let options = NSActivityOptions(NSActivityOptions::UserInitiatedAllowingIdleSystemSleep.0 | NSActivityOptions::LatencyCritical.0);
    hold(&PLAYBACK, on, options, "A stream is open");
}

/// A picture is moving (`true`): neither the display nor the machine goes to sleep for lack of input.
pub fn keep_display_awake(on: bool) {
    let options = NSActivityOptions(NSActivityOptions::IdleDisplaySleepDisabled.0 | NSActivityOptions::IdleSystemSleepDisabled.0);
    hold(&DISPLAY, on, options, "Playing video");
}

/// The swap of the window's buffers already waits for the display.
pub struct FramePacer;

impl FramePacer {
    pub fn start() -> Self {
        Self
    }

    pub fn wait_for_pass(&self) {}
}

/// The screens in points, with the origin at the top left of the main screen, which is how window positions are saved.
pub fn screens() -> Screens {
    let mut screens = Screens { areas: Vec::new(), primary: (1920.0, 1080.0) };
    let Some(mtm) = MainThreadMarker::new() else { return screens };
    let list = NSScreen::screens(mtm);
    let mut main_height = None;
    for (index, screen) in list.iter().enumerate() {
        let (frame, usable) = (screen.frame(), screen.visibleFrame());
        // AppKit counts from the bottom of the first screen upwards.
        let height = *main_height.get_or_insert(frame.size.height);
        if index == 0 {
            screens.primary = (usable.size.width as f32, usable.size.height as f32);
        }
        screens.areas.push(from_appkit(usable.origin.x, usable.origin.y, usable.size.width, usable.size.height, height));
    }
    screens
}

fn from_appkit(x: f64, y: f64, width: f64, height: f64, main_height: f64) -> ScreenRect {
    let top = main_height - (y + height);
    ScreenRect { left: x as f32, top: top as f32, right: (x + width) as f32, bottom: (top + height) as f32 }
}

/// In an application bundle the Dock takes the icon from the bundle; setting one at run time would replace it.
pub fn use_runtime_icon() -> bool {
    !std::env::current_exe().is_ok_and(|exe| exe.parent().is_some_and(|dir| dir.ends_with("Contents/MacOS")))
}

/// A program started from a terminal writes to it already; one started from the Finder has nowhere to write.
pub fn attach_console() -> bool {
    true
}

/// A dialog in a process of its own: a panic can come from any thread, and AppKit only shows windows from the main one.
pub fn show_fatal_error(title: &str, text: &str) {
    // The texts are arguments of the script, never part of it.
    let _ = Command::new("/usr/bin/osascript")
        .args(["-e", "on run argv", "-e", "display alert (item 1 of argv) message (item 2 of argv) as critical", "-e", "end run"])
        .args([title, text])
        .stdout(std::process::Stdio::null())
        .status();
}

fn open(args: &[&std::ffi::OsStr]) {
    match Command::new("/usr/bin/open").args(args).spawn() {
        // Collected, so no finished process is left waiting.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(error) => tracing::warn!(%error, "could not run open"),
    }
}

/// Opens a web address in the browser. Returns whether it was one.
pub fn open_url(url: &str) -> bool {
    if !is_web_url(url) {
        tracing::warn!("not opening an address that is not a web address");
        return false;
    }
    open(&[url.as_ref()]);
    true
}

pub fn open_folder(path: &Path) {
    open(&[path.as_os_str()]);
}

pub fn edit_text_file(path: &Path) {
    // `-t`: the default text editor, whatever the file's extension is associated with.
    open(&["-t".as_ref(), path.as_os_str()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screens_are_turned_to_count_from_the_top() {
        // The main screen, 1080 high, with the menu bar taken off its top.
        assert_eq!(from_appkit(0.0, 0.0, 1920.0, 1055.0, 1080.0), ScreenRect { left: 0.0, top: 25.0, right: 1920.0, bottom: 1080.0 });
        // A portrait screen to the left that reaches above and below it.
        assert_eq!(from_appkit(-1080.0, -400.0, 1080.0, 1920.0, 1080.0), ScreenRect { left: -1080.0, top: -440.0, right: 0.0, bottom: 1480.0 });
    }
}
