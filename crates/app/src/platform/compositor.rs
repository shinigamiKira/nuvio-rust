//! Pacing against the desktop compositor.
//!
//! On a laptop where the NVIDIA GPU renders and the Intel one drives the display, a window drawn through OpenGL showed single
//! frames with a band of the picture missing (the bottom 200 pixels black, or a stale top half), at the start of animations,
//! on tab switches and as the player's controls faded. They are in a capture of what the desktop shows (a screen grab of the
//! window at 60 samples a second) and in nothing the app can read back, so they come from how a finished frame reaches the
//! screen. Starting each frame right after the compositor has finished a pass made them go away: none in four runs of 14 hero
//! crossfades, none in a run of tab switches and none in a run of video with pausing, against 10 to 27 glitching frames per
//! crossfade run and 368 samples in the video run before. It also spaces frames evenly, one per refresh.
//!
//! `DwmFlush` is what waits for that pass, but it blocks for as long as the compositor has nothing to do. A helper thread
//! therefore runs it while frames are being drawn and shows whether the compositor is awake: the drawing thread calls
//! `DwmFlush` itself only when it is (doing the wait on the drawing thread is what removed the glitches; waiting for the
//! helper's tick instead still let one through in three runs), and otherwise waits for the helper with a time limit.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Graphics::Dwm::DwmFlush;

/// The helper keeps ticking this long after the last frame, so the next one of a burst finds it running.
const ACTIVE_FOR: Duration = Duration::from_millis(250);
/// A frame never waits longer than this for the compositor: two refreshes at 60 Hz.
const WAIT_AT_MOST: Duration = Duration::from_millis(34);
/// A pass seen this recently means the compositor is awake.
const TICKING: Duration = Duration::from_millis(30);

struct State {
    /// Compositor passes seen so far.
    ticks: u64,
    last_tick: Option<Instant>,
    wanted_until: Instant,
}

struct Shared {
    state: Mutex<State>,
    tick: Condvar,
    wake: Condvar,
}

pub struct Compositor {
    shared: Arc<Shared>,
}

impl Compositor {
    pub fn start() -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State { ticks: 0, last_tick: None, wanted_until: Instant::now() }), tick: Condvar::new(), wake: Condvar::new() });
        let worker = shared.clone();
        if let Err(error) = std::thread::Builder::new().name("nuvio-compositor".into()).spawn(move || worker.run()) {
            tracing::warn!(%error, "frames will not be paced against the compositor");
        }
        Self { shared }
    }

    /// Returns after the compositor's next pass, or after a short time when there is none to wait for.
    pub fn wait_for_pass(&self) {
        if std::env::var_os("NUVIO_PACE_OFF").is_some() {
            return;
        }
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let seen = state.ticks;
        let ticking = state.last_tick.is_some_and(|at| at.elapsed() < TICKING);
        state.wanted_until = Instant::now() + ACTIVE_FOR;
        self.shared.wake.notify_one();
        if ticking {
            // The helper has just seen a pass, so the compositor is awake and this returns within a refresh. Waiting here, on
            // the thread that draws, is what removed the glitches; waiting for the helper's tick did not always.
            drop(state);
            // SAFETY: no arguments; it only waits.
            unsafe { DwmFlush() };
        } else {
            let _ = self.shared.tick.wait_timeout_while(state, WAIT_AT_MOST, |s| s.ticks == seen);
        }
    }
}

impl Shared {
    fn run(&self) {
        loop {
            {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                while Instant::now() >= state.wanted_until {
                    state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
                }
            }
            let started = Instant::now();
            // SAFETY: no arguments; it only waits.
            unsafe { DwmFlush() };
            {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.ticks += 1;
                state.last_tick = Some(Instant::now());
                self.tick.notify_all();
            }
            // With composition off DwmFlush returns at once; do not spin.
            if started.elapsed() < Duration::from_millis(1) {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
    }
}
