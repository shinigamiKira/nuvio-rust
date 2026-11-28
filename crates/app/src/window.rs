//! The first window: where it opens and how big it is. The previous session's size and place are restored when they
//! still fit on the connected screens; otherwise the window opens centred at a size that fits the main screen.

use eframe::egui::{self, ViewportBuilder};
use nuvio_core::store::WindowState;

use crate::platform::{self, ScreenRect};

pub const DEFAULT_SIZE: (f32, f32) = (1280.0, 800.0);
pub const MIN_SIZE: (f32, f32) = (960.0, 600.0);

/// Whether a saved window is still somewhere the user can reach: enough of its title bar is on one of the screens.
fn fits(state: &WindowState, areas: &[ScreenRect]) -> bool {
    areas.iter().any(|a| state.x + 160.0 < a.right && state.y + 80.0 < a.bottom && state.x + state.width > a.left + 160.0 && state.y >= a.top - 8.0)
}

pub fn initial_viewport(fullscreen: bool, forced_size: Option<(f32, f32)>, saved: Option<WindowState>) -> (ViewportBuilder, bool) {
    let mut viewport = ViewportBuilder::default()
        .with_title("Nuvio")
        .with_min_inner_size([MIN_SIZE.0, MIN_SIZE.1])
        .with_fullscreen(fullscreen);
    let screens = platform::screens();
    let (screen_w, screen_h) = screens.primary;
    // Leave room for the taskbar and the title bar when the screen is small (a laptop at 150 % scaling).
    let fit = |(w, h): (f32, f32)| (w.min(screen_w * 0.94).max(MIN_SIZE.0.min(screen_w)), h.min(screen_h * 0.9).max(MIN_SIZE.1.min(screen_h)));
    if let Some(size) = forced_size {
        return (viewport.with_inner_size([size.0, size.1]), true);
    }
    match saved.filter(|state| fits(state, &screens.areas)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> WindowState {
        WindowState { x, y, width: 1280.0, height: 800.0, maximized: false }
    }

    #[test]
    fn a_window_must_be_reachable_on_one_of_the_screens() {
        // A landscape screen and, to its right and lower, a portrait one.
        let areas = [
            ScreenRect { left: 0.0, top: 25.0, right: 1920.0, bottom: 1080.0 },
            ScreenRect { left: 1920.0, top: 400.0, right: 3000.0, bottom: 2320.0 },
        ];
        assert!(fits(&at(100.0, 100.0), &areas));
        assert!(fits(&at(2000.0, 900.0), &areas));
        // Inside the box around both screens, but on neither of them.
        assert!(!fits(&at(2000.0, 50.0), &areas));
        assert!(!fits(&at(100.0, 1500.0), &areas));
        // Off to the left, and above the top edge.
        assert!(!fits(&at(-1200.0, 100.0), &areas));
        assert!(!fits(&at(100.0, -100.0), &areas));
        assert!(!fits(&at(100.0, 100.0), &[]));
    }
}
