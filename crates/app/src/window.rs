//! The first window: where it opens and how big it is. The previous session's size and place are restored when they
//! still fit on the connected screens; otherwise the window opens centred at a size that fits the main screen.

use eframe::egui::{self, ViewportBuilder};
use nuvio_core::store::WindowState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub const DEFAULT_SIZE: (f32, f32) = (1280.0, 800.0);
pub const MIN_SIZE: (f32, f32) = (960.0, 600.0);

/// The desktop in physical pixels: `(left, top, right, bottom)` over all screens.
fn virtual_screen() -> (f32, f32, f32, f32) {
    // SAFETY: GetSystemMetrics takes no pointers.
    unsafe {
        let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN) as f32, GetSystemMetrics(SM_YVIRTUALSCREEN) as f32);
        (x, y, x + GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32, y + GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32)
    }
}

fn primary_size() -> (f32, f32) {
    // SAFETY: as above.
    unsafe { (GetSystemMetrics(SM_CXSCREEN) as f32, GetSystemMetrics(SM_CYSCREEN) as f32) }
}

/// Whether a saved window is still somewhere the user can reach. Coordinates saved in points are never larger than the
/// pixel coordinates they came from, so checking them against pixel bounds can only be too lenient, never wrong in a
/// way that hides the window.
fn fits(state: &WindowState) -> bool {
    let (left, top, right, bottom) = virtual_screen();
    state.x + 160.0 < right && state.y + 80.0 < bottom && state.x + state.width > left + 160.0 && state.y >= top - 8.0
}

pub fn initial_viewport(fullscreen: bool, forced_size: Option<(f32, f32)>, saved: Option<WindowState>) -> (ViewportBuilder, bool) {
    let mut viewport = ViewportBuilder::default()
        .with_title("Nuvio")
        .with_min_inner_size([MIN_SIZE.0, MIN_SIZE.1])
        .with_fullscreen(fullscreen);
    let (screen_w, screen_h) = primary_size();
    // Leave room for the taskbar and the title bar when the screen is small (a laptop at 150 % scaling).
    let fit = |(w, h): (f32, f32)| (w.min(screen_w * 0.94).max(MIN_SIZE.0.min(screen_w)), h.min(screen_h * 0.9).max(MIN_SIZE.1.min(screen_h)));
    if let Some(size) = forced_size {
        return (viewport.with_inner_size([size.0, size.1]), true);
    }
    match saved.filter(fits) {
        Some(state) => {
            let (w, h) = (state.width.max(MIN_SIZE.0), state.height.max(MIN_SIZE.1));
            viewport = viewport.with_inner_size([w, h]).with_position(egui::pos2(state.x, state.y)).with_maximized(state.maximized);
            (viewport, false)
        }
        None => {
            let (w, h) = fit(DEFAULT_SIZE);
            (viewport.with_inner_size([w, h]), true)
        }
    }
}
