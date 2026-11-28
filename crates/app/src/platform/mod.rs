//! Everything the app asks of the operating system. Each system provides the same names:
//!
//! * `NAME`: what the system is called in the app's own text;
//! * `start`: once, at startup;
//! * `playback`: a stream is open or closed;
//! * `keep_display_awake`: a picture is moving, or no longer is;
//! * `FramePacer`: when a frame may start;
//! * `screens`: where a window can go;
//! * `use_runtime_icon`: whether the app sets its own window and Dock icon;
//! * `attach_console`, `show_fatal_error`, `open_url`, `open_folder`, `edit_text_file`.

#[cfg(windows)]
mod compositor;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// A rectangle in the coordinates the window's position is saved in, `y` growing downwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Screens {
    /// The areas a window can be seen in. Empty when the system does not say.
    pub areas: Vec<ScreenRect>,
    /// The size of the main screen.
    pub primary: (f32, f32),
}

/// Whether `url` is a web address. Addresses come from addons, and the system would open a file or start another
/// program for some of the other kinds.
#[cfg(any(target_os = "macos", test))]
fn is_web_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.starts_with("https://") || url.starts_with("http://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_are_handed_to_the_system() {
        assert!(is_web_url("https://www.youtube.com/watch?v=x&t=1"));
        assert!(is_web_url("HTTP://example.com"));
        for bad in ["file:///etc/passwd", "/Applications/Calculator.app", "-a Calculator", "ssh://host", "javascript:alert(1)", ""] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }
}
