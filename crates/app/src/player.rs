//! The player: one libmpv instance drawing into the window through OpenGL, plus the state the controls show.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, PaintCallback, Rect};
use eframe::egui_glow;
use nuvio_mpv::{EndReason, Event, Mpv, Observe, RenderContext, Value};
use serde::Deserialize;

/// Seconds of media buffered before a stalled stream resumes. A first start needs less, a stall deserves more.
const START_BUFFER_SECS: u32 = 1;
const REBUFFER_SECS: u32 = 5;
/// The Dolby Vision filter runs on 1920 px wide frames: at 4K it fell to 12 frames a second, at 1920 it keeps up.
const DV_FILTER_WIDTH: i64 = 1920;
/// How long to wait for mpv to report what the filter chain produced.
const DV_VERIFY_TIMEOUT: Duration = Duration::from_secs(15);
/// mpv's HTTP stream retries an interrupted transfer a few times and then reports end of file. When that happens far
/// from the end, the stream is opened again where it stopped, this many times before giving up.
const MAX_RECOVERIES: u32 = 5;
/// In the order to try them. NVDEC hands its frames to OpenGL without a copy (35 % of a core on a 30 fps 2160p HEVC
/// file, against 90 % for the copy-back decoders); the copy-back ones are for GPUs that cannot do that through OpenGL.
const HARDWARE_DECODERS: &str = "nvdec,d3d11va-copy,dxva2-copy";

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub external: bool,
    #[serde(default)]
    pub selected: bool,
    #[serde(default, rename = "demux-w")]
    pub width: Option<i64>,
    #[serde(default, rename = "demux-h")]
    pub height: Option<i64>,
    #[serde(default, rename = "demux-channel-count")]
    pub channels: Option<i64>,
    #[serde(default, rename = "dolby-vision-profile")]
    pub dolby_vision_profile: Option<i64>,
}

impl Track {
    pub fn label(&self) -> String {
        let language = self.lang.as_deref().map(language_label).unwrap_or_default();
        let mut parts: Vec<String> = Vec::new();
        if !language.is_empty() {
            parts.push(language);
        }
        if let Some(title) = self.title.as_deref().filter(|t| !t.is_empty()) {
            parts.push(title.to_owned());
        }
        if let Some(codec) = self.codec.as_deref() {
            parts.push(codec.to_uppercase());
        }
        if self.kind == "audio" {
            if let Some(channels) = self.channels {
                parts.push(match channels {
                    1 => "Mono".into(),
                    2 => "Stereo".into(),
                    6 => "5.1".into(),
                    8 => "7.1".into(),
                    n => format!("{n} ch"),
                });
            }
        }
        if parts.is_empty() { format!("Track {}", self.id) } else { parts.join(" Ã‚Â· ") }
    }
}

/// ISO 639 codes mpv reports, in the languages people actually pick between.
pub fn language_label(code: &str) -> String {
    let name = match code.to_ascii_lowercase().as_str() {
        "eng" | "en" => "English",
        "spa" | "es" => "Spanish",
        "fre" | "fra" | "fr" => "French",
        "ger" | "deu" | "de" => "German",
        "ita" | "it" => "Italian",
        "por" | "pt" => "Portuguese",
        "rus" | "ru" => "Russian",
        "jpn" | "ja" => "Japanese",
        "kor" | "ko" => "Korean",
        "chi" | "zho" | "zh" => "Chinese",
        "ara" | "ar" => "Arabic",
        "hin" | "hi" => "Hindi",
        "dut" | "nld" | "nl" => "Dutch",
        "pol" | "pl" => "Polish",
        "tur" | "tr" => "Turkish",
        "swe" | "sv" => "Swedish",
        "nor" | "no" => "Norwegian",
        "dan" | "da" => "Danish",
        "fin" | "fi" => "Finnish",
        "heb" | "he" => "Hebrew",
        "tha" | "th" => "Thai",
        "vie" | "vi" => "Vietnamese",
        "ind" | "id" => "Indonesian",
        "ukr" | "uk" => "Ukrainian",
        "cze" | "ces" | "cs" => "Czech",
        "gre" | "ell" | "el" => "Greek",
        "hun" | "hu" => "Hungarian",
        "rum" | "ron" | "ro" => "Romanian",
        "und" => "",
        other => return other.to_uppercase(),
    };
    name.to_owned()
}

#[derive(Debug, Clone, Default)]
pub struct State {
    /// The file was opened and its tracks are known.
    pub loaded: bool,
    /// A picture is on screen (the first frame after a load or a seek).
    pub started: bool,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    /// mpv stopped to refill its cache.
    pub buffering: bool,
    /// How full the refill is, 0 to 100.
    pub buffer_percent: i64,
    /// Seconds of media read ahead of the playhead.
    pub cached_secs: f64,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub tracks: Vec<Track>,
    pub ended: bool,
    pub error: Option<String>,
    pub hwdec: String,
    pub video_size: Option<(i64, i64)>,
    pub seeking: bool,
    /// The colour matrix of the picture after the filters (`video-out-params/colormatrix`).
    out_matrix: Option<String>,
}

/// What a player needs to know to open a stream.
#[derive(Clone)]
pub struct Source {
    pub url: String,
    pub start: f64,
    pub headers: Vec<(String, String)>,
    pub subtitles: Vec<String>,
}

/// Timing shared with mpv's threads: when it last announced a video frame, when that last repainted the window, and how
/// long a frame of the current file lasts.
#[derive(Clone)]
struct Clock {
    epoch: Instant,
    last_video_frame: Arc<AtomicU64>,
    last_video_repaint: Arc<AtomicU64>,
    frame_ms: Arc<AtomicU64>,
}

impl Clock {
    fn new() -> Self {
        Self { epoch: Instant::now(), last_video_frame: Arc::default(), last_video_repaint: Arc::default(), frame_ms: Arc::default() }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    fn mark_video_frame(&self) {
        self.last_video_frame.store(self.now_ms(), Ordering::Relaxed);
    }

    /// With the frame rate known, mpv's second announcement of a frame (it calls back about twice per frame) does not
    /// need another pass over a 4K picture.
    fn should_repaint_for_video(&self) -> bool {
        let (now, frame) = (self.now_ms(), self.frame_ms.load(Ordering::Relaxed));
        if frame > 0 && now.saturating_sub(self.last_video_repaint.load(Ordering::Relaxed)) < frame * 6 / 10 {
            return false;
        }
        self.last_video_repaint.store(now, Ordering::Relaxed);
        true
    }

    fn set_fps(&self, fps: f64) {
        let frame = if fps >= 10.0 { (1000.0 / fps) as u64 } else { 0 };
        self.frame_ms.store(frame, Ordering::Relaxed);
    }

    fn since_video_frame(&self) -> Duration {
        let last = self.last_video_frame.load(Ordering::Relaxed);
        if last == 0 {
            return Duration::MAX;
        }
        self.epoch.elapsed().saturating_sub(Duration::from_millis(last - 1))
    }
}

/// The GL half of the player, shared with the paint callback that runs inside egui's render pass.
struct Surface {
    mpv: Arc<Mpv>,
    clock: Clock,
    ctx: egui::Context,
    render: Mutex<Option<GlSide>>,
    failed: Mutex<Option<String>>,
    /// Set once the render context exists. mpv refuses to open a video output before that, so a file is only loaded
    /// after it (the context can only be created inside a paint callback, where the GL context is current).
    ready: AtomicBool,
}

struct GlSide(RenderContext);

// SAFETY: the render context is only ever touched on the UI thread (inside the paint callback and on drop), and the
// Mutex serialises those. It is not `Send` only because it wraps raw pointers.
unsafe impl Send for GlSide {}

impl Surface {
    fn paint(&self, info: &egui::PaintCallbackInfo) {
        let mut slot = self.render.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            if self.failed.lock().map(|f| f.is_some()).unwrap_or(false) {
                return;
            }
            let (ctx, clock) = (self.ctx.clone(), self.clock.clone());
            match RenderContext::new_opengl(&self.mpv, move || {
                clock.mark_video_frame();
                if clock.should_repaint_for_video() {
                    // A new picture: as soon as possible, but through the delayed path (an immediate request draws two frames).
                    ctx.request_repaint_after(Duration::from_micros(500));
                }
            }) {
                Ok(context) => {
                    *slot = Some(GlSide(context));
                    self.ready.store(true, Ordering::Release);
                    crate::services::wake(&self.ctx);
                }
                Err(error) => {
                    tracing::error!(%error, "could not create the mpv render context");
                    *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
                    return;
                }
            }
        }
        if let Some(GlSide(context)) = slot.as_ref() {
            let [width, height] = info.screen_size_px;
            static FIRST: std::sync::Once = std::sync::Once::new();
            FIRST.call_once(|| tracing::info!(width, height, "first video paint"));
            context.take_update();
            if let Err(error) = context.render(0, width as i32, height as i32, true) {
                tracing::warn!(%error, "mpv render failed");
            }
            context.report_swap();
        }
    }

    /// Drops the render context. Needs the GL context current, which it is during a paint callback or `on_exit`.
    fn release(&self) {
        self.render.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

/// Dolby Vision profile 5 has no HDR10 layer to fall back on: its colours only come out right after libplacebo reshapes
/// them, which mpv's own renderer does not do. Such a file is opened a second time with a libplacebo filter that
/// converts it to SDR. mpv silently drops a filter it cannot start, so the result is checked.
#[derive(Clone, Copy, PartialEq)]
enum DolbyVision {
    Unchecked,
    NotNeeded,
    /// Reopened with the filter, waiting for the second `FileLoaded`.
    Opening,
    /// Playing with the filter; waiting to see what colour matrix comes out of it.
    Verifying(Instant),
    Converting,
    /// The filter did not start; the file was opened again without it.
    Dropped,
}

pub struct Player {
    mpv: Arc<Mpv>,
    hardware_decoding: bool,
    stats_shared: Arc<Mutex<Vec<(&'static str, String)>>>,
    stats_busy: Arc<AtomicBool>,
    stats_at: Instant,
    source: Option<Source>,
    /// The per-file options of the current load, kept so a recovery reopens the file the same way.
    last_extra: Vec<String>,
    recoveries: u32,
    dolby_vision: DolbyVision,
    surface: Arc<Surface>,
    pub state: State,
    scratch: Vec<Event>,
    pending: Option<Source>,
    rebuffer_raised: bool,
    opened: Instant,
    pub log_lines: Vec<String>,
}

impl Player {
    pub fn new(ctx: &egui::Context, hardware_decoding: bool) -> nuvio_mpv::Result<Self> {
        let repaint = ctx.clone();
        let hwdec = std::env::var("NUVIO_MPV_HWDEC").unwrap_or_else(|_| if hardware_decoding { HARDWARE_DECODERS.into() } else { "no".into() });
        let vo = std::env::var("NUVIO_MPV_VO").unwrap_or_else(|_| "libmpv".into());
        let start_buffer = START_BUFFER_SECS.to_string();
        let mut options: Vec<(String, String)> = vec![
            ("vo".into(), vo),
            ("hwdec".into(), hwdec),
            ("keep-open".into(), "yes".into()),
            ("idle".into(), "yes".into()),
            ("terminal".into(), "no".into()),
            ("osc".into(), "no".into()),
            ("osd-level".into(), "0".into()),
            ("input-default-bindings".into(), "no".into()),
            ("input-vo-keyboard".into(), "no".into()),
            ("input-media-keys".into(), "no".into()),
            ("msg-level".into(), format!("all={}", std::env::var("NUVIO_MPV_LOG").unwrap_or_else(|_| "warn".into()))),
            ("audio-fallback-to-null".into(), "yes".into()),
            ("audio-channels".into(), "auto".into()),
            ("sub-auto".into(), "no".into()),
            ("subs-fallback".into(), "yes".into()),
            ("video-rotate".into(), "no".into()),
            // A network stream is read ahead into the demuxer cache. Playback starts once a second is buffered.
            ("cache".into(), "yes".into()),
            ("demuxer-max-bytes".into(), (160u64 * 1024 * 1024).to_string()),
            ("demuxer-max-back-bytes".into(), (32u64 * 1024 * 1024).to_string()),
            ("cache-pause-wait".into(), start_buffer),
            ("network-timeout".into(), "15".into()),
            ("tls-verify".into(), "yes".into()),
            ("hr-seek".into(), "yes".into()),
            ("save-position-on-quit".into(), "no".into()),
            ("config".into(), "no".into()),
        ];
        if let Ok(extra) = std::env::var("NUVIO_MPV_OPTS") {
            for pair in extra.split(';').filter(|p| p.contains('=')) {
                let (name, value) = pair.split_once('=').unwrap_or_default();
                options.push((name.to_owned(), value.to_owned()));
            }
        }
        let borrowed: Vec<(&str, &str)> = options.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        // mpv wakes this up for every property change, dozens of times a second. Each wake is a whole frame of the window
        // (and another full-size video pass). While video frames are arriving they repaint the window themselves, so a wake
        // that comes right after one is dropped; otherwise (paused, buffering) wakes are coalesced to 30 a second.
        let clock = Clock::new();
        let wake_clock = clock.clone();
        let mpv = Arc::new(Mpv::new(&borrowed, move || {
            if wake_clock.since_video_frame() > Duration::from_millis(50) {
                repaint.request_repaint_after(Duration::from_millis(33));
            }
        })?);
        mpv.request_log_level(&std::env::var("NUVIO_MPV_LOG").unwrap_or_else(|_| "warn".into()));
        for (name, kind) in [
            ("time-pos", Observe::Double),
            ("duration", Observe::Double),
            ("pause", Observe::Flag),
            ("paused-for-cache", Observe::Flag),
            ("cache-buffering-state", Observe::Int),
            ("demuxer-cache-time", Observe::Double),
            ("volume", Observe::Double),
            ("mute", Observe::Flag),
            ("speed", Observe::Double),
            ("seeking", Observe::Flag),
            ("container-fps", Observe::Double),
            // With keep-open mpv never sends END_FILE at the end; it parks on the last frame and raises this flag.
            ("eof-reached", Observe::Flag),
            ("hwdec-current", Observe::Text),
            ("track-list", Observe::Text),
            ("video-params/w", Observe::Int),
            ("video-params/h", Observe::Int),
            ("video-out-params/colormatrix", Observe::Text),
        ] {
            mpv.observe(name, kind);
        }
        let surface = Arc::new(Surface {
            mpv: mpv.clone(),
            clock,
            ctx: ctx.clone(),
            render: Mutex::new(None),
            failed: Mutex::new(None),
            ready: AtomicBool::new(false),
        });
        Ok(Self {
            mpv,
            hardware_decoding,
            stats_shared: Arc::new(Mutex::new(Vec::new())),
            stats_busy: Arc::new(AtomicBool::new(false)),
            stats_at: Instant::now() - Duration::from_secs(5),
            source: None,
            last_extra: Vec::new(),
            recoveries: 0,
            dolby_vision: DolbyVision::Unchecked,
            surface,
            state: State { volume: 100.0, speed: 1.0, ..State::default() },
            scratch: Vec::new(),
            pending: None,
            rebuffer_raised: false,
            opened: Instant::now(),
            log_lines: Vec::new(),
        })
    }

    /// Opens a stream. The load starts on the first frame after the video surface exists.
    pub fn open(&mut self, source: Source) {
        self.state = State { volume: self.state.volume, muted: self.state.muted, speed: 1.0, ..State::default() };
        self.pending = Some(source);
        self.ctx_repaint();
    }

    fn ctx_repaint(&self) {
        crate::services::wake(&self.surface.ctx);
    }

    fn start_pending(&mut self) {
        let Some(source) = self.pending.take() else { return };
        self.dolby_vision = DolbyVision::Unchecked;
        self.recoveries = 0;
        let start = source.start;
        self.load(&source, &[], start);
        self.source = Some(source);
    }

    /// Sends the file to mpv. `extra` are per-file options (a filter, a decoder) that only apply to this load.
    fn load(&mut self, source: &Source, extra: &[String], start: f64) {
        self.opened = Instant::now();
        self.state = State { volume: self.state.volume, muted: self.state.muted, speed: 1.0, ..State::default() };
        self.rebuffer_raised = false;
        self.last_extra = extra.to_vec();
        let _ = self.mpv.set_str("cache-pause-wait", &START_BUFFER_SECS.to_string());
        let headers: Vec<String> = source.headers.iter().map(|(name, value)| format!("{name}: {value}")).collect();
        let _ = self.mpv.set_str("http-header-fields", &headers.join(","));
        let mut options: Vec<String> = Vec::new();
        if start > 1.0 {
            options.push(format!("start={start:.3}"));
        }
        options.extend(extra.iter().cloned());
        let options = options.join(",");
        let result = if options.is_empty() {
            self.mpv.command(&["loadfile", &source.url, "replace"])
        } else {
            self.mpv.command(&["loadfile", &source.url, "replace", "-1", &options])
        };
        if let Err(error) = result {
            self.state.error = Some(error.to_string());
        }
        for subtitle in &source.subtitles {
            let _ = self.mpv.command(&["sub-add", subtitle, "auto"]);
        }
    }

    /// Once the tracks of a freshly loaded file are known: is it Dolby Vision profile 5?
    fn detect_dolby_vision(&mut self) {
        match self.dolby_vision {
            DolbyVision::Unchecked if !self.state.tracks.is_empty() => {
                let found = self.state.tracks.iter().find(|t| t.kind == "video" && t.dolby_vision_profile == Some(5)).cloned();
                let Some(track) = found else {
                    self.dolby_vision = DolbyVision::NotNeeded;
                    return;
                };
                let Some(source) = self.source.clone() else { return };
                let (width, height) = (track.width.unwrap_or(0), track.height.unwrap_or(0));
                let scale = if width > DV_FILTER_WIDTH && height > 0 {
                    format!("w={DV_FILTER_WIDTH}:h={}:", (height * DV_FILTER_WIDTH / width) & !1)
                } else {
                    String::new()
                };
                let mut extra = vec![format!(
                    "vf=libplacebo={scale}tonemapping=auto:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:format=yuv420p"
                )];
                // A filter only takes frames in memory. Given the zero-copy frames of a GPU decoder mpv drops it and plays
                // the picture green and purple, so the decode is copied back: NVDEC keeps up at 4K, other GPUs fall back
                // to software decoding.
                if self.hardware_decoding {
                    extra.push("hwdec=nvdec-copy".into());
                }
                tracing::info!("Dolby Vision profile 5: opening again with libplacebo");
                self.dolby_vision = DolbyVision::Opening;
                let at = self.state.position.max(source.start);
                self.load(&source, &extra, at);
            }
            _ => {}
        }
    }

    fn verify_dolby_vision(&mut self) {
        let DolbyVision::Verifying(since) = self.dolby_vision else { return };
        match self.state.out_matrix.clone() {
            Some(matrix) if matrix == "dolbyvision" => {
                tracing::warn!("the Dolby Vision filter did not start, playing without it");
                self.dolby_vision = DolbyVision::Dropped;
                if let Some(source) = self.source.clone() {
                    let at = self.state.position;
                    self.load(&source, &[], at);
                }
            }
            Some(matrix) if !matrix.is_empty() => {
                tracing::info!(matrix, "Dolby Vision filter is converting");
                self.dolby_vision = DolbyVision::Converting;
            }
            _ if since.elapsed() > DV_VERIFY_TIMEOUT => self.dolby_vision = DolbyVision::Converting,
            _ => self.surface.ctx.request_repaint_after(Duration::from_millis(200)),
        }
    }

    pub fn toggle_pause(&self) {
        let _ = self.mpv.set_bool("pause", !self.state.paused);
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.mpv.set_bool("pause", paused);
    }

    pub fn seek_to(&self, seconds: f64) {
        let _ = self.mpv.command(&["seek", &format!("{seconds:.3}"), "absolute+exact"]);
    }

    pub fn seek_by(&self, seconds: f64) {
        let _ = self.mpv.command(&["seek", &format!("{seconds:.3}"), "relative+exact"]);
    }

    pub fn set_volume(&mut self, volume: f64) {
        let volume = volume.clamp(0.0, 130.0);
        self.state.volume = volume;
        let _ = self.mpv.set_f64("volume", volume);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.state.muted = muted;
        let _ = self.mpv.set_bool("mute", muted);
    }

    pub fn set_speed(&self, speed: f64) {
        let _ = self.mpv.set_f64("speed", speed);
    }

    pub fn select_audio(&self, id: i64) {
        let _ = self.mpv.set_str("aid", &id.to_string());
    }

    pub fn select_subtitle(&self, id: Option<i64>) {
        let _ = self.mpv.set_str("sid", &id.map(|i| i.to_string()).unwrap_or_else(|| "no".into()));
    }

    pub fn set_property_text(&self, name: &str, value: &str) {
        let _ = self.mpv.set_str(name, value);
    }

    pub fn set_property_number(&self, name: &str, value: f64) {
        let _ = self.mpv.set_f64(name, value);
    }

    /// Track preferences, applied to the next file that opens. An empty language means "whatever the file defaults to".
    pub fn set_track_preferences(&self, audio_language: &str, subtitle_language: &str, subtitles_on: bool) {
        let _ = self.mpv.set_str("alang", audio_language);
        let _ = self.mpv.set_str("slang", subtitle_language);
        let _ = self.mpv.set_str("sid", if subtitles_on { "auto" } else { "no" });
    }

    /// Shows an error card instead of the picture (a torrent that could not be opened).
    pub fn fail(&mut self, message: String) {
        self.state.error = Some(message);
    }

    /// Loads a subtitle file from an addon and switches to it.
    pub fn add_subtitle(&self, url: &str, title: &str, language: &str) {
        let _ = self.mpv.command(&["sub-add", url, "select", title, language]);
    }

    pub fn stop(&self) {
        let _ = self.mpv.command(&["stop"]);
    }

    /// Playback details for the info panel. They are read on a short-lived thread, a few times a second: the getters of
    /// libmpv wait for the player core and must stay off the thread that renders.
    pub fn stats(&mut self) -> Vec<(&'static str, String)> {
        if self.stats_at.elapsed() > Duration::from_millis(500) && !self.stats_busy.swap(true, Ordering::AcqRel) {
            self.stats_at = Instant::now();
            let (mpv, shared, busy, ctx) = (self.mpv.clone(), self.stats_shared.clone(), self.stats_busy.clone(), self.surface.ctx.clone());
            std::thread::spawn(move || {
                let get = |name: &str| mpv.get_str(name).unwrap_or_else(|| "–".into());
                let speed = mpv.get_f64("cache-speed").unwrap_or(0.0) / 1_048_576.0;
                let lines = vec![
                    ("Video", format!("{} {}×{} @ {} fps", get("video-codec"), get("video-params/w"), get("video-params/h"), get("container-fps"))),
                    ("Decoder", get("hwdec-current")),
                    ("Colour", format!("{} / {}", get("video-params/colormatrix"), get("video-params/gamma"))),
                    ("Audio", format!("{} {}", get("audio-codec-name"), get("audio-params/hr-channels"))),
                    ("Network", format!("{speed:.1} MB/s")),
                    ("Frames", format!("{} dropped, {} delayed", get("frame-drop-count"), get("vo-delayed-frame-count"))),
                ];
                if let Ok(mut slot) = shared.lock() {
                    *slot = lines;
                }
                busy.store(false, Ordering::Release);
                crate::services::wake(&ctx);
            });
        }
        self.stats_shared.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Reads what changed in mpv since the last call. Call once per frame.
    pub fn pump(&mut self) {
        if self.pending.is_some() && self.surface.ready.load(Ordering::Acquire) {
            self.start_pending();
        }
        self.scratch.clear();
        self.mpv.poll_events(&mut self.scratch);
        let events = std::mem::take(&mut self.scratch);
        for event in &events {
            match event {
                Event::FileLoaded => {
                    self.state.loaded = true;
                    if self.dolby_vision == DolbyVision::Opening {
                        self.dolby_vision = DolbyVision::Verifying(Instant::now());
                    } else {
                        self.detect_dolby_vision();
                    }
                }
                Event::PlaybackRestart => {
                    if !self.state.started {
                        self.state.started = true;
                        tracing::info!(ms = self.opened.elapsed().as_millis() as u64, "first frame");
                    }
                    if !self.rebuffer_raised {
                        self.rebuffer_raised = true;
                        let _ = self.mpv.set_str("cache-pause-wait", &REBUFFER_SECS.to_string());
                    }
                }
                Event::EndFile { reason, error } => match reason {
                    EndReason::Eof | EndReason::Error if self.should_recover() => self.recover(),
                    EndReason::Eof => self.state.ended = true,
                    EndReason::Error => self.state.error = Some(error.clone().unwrap_or_else(|| "Playback failed".into())),
                    _ => {}
                },
                Event::Property { name, value } => self.apply_property(name, value),
                Event::Log { level, text } => {
                    // mpv names the URL it failed on, and a stream URL carries the addon's token.
                    let text = nuvio_core::redact::urls(text);
                    tracing::debug!(level, text, "mpv");
                    if self.log_lines.len() < 200 {
                        self.log_lines.push(format!("{level}: {text}"));
                    }
                }
                Event::Seek => self.state.seeking = true,
                Event::StartFile | Event::VideoReconfig | Event::Shutdown => {}
            }
        }
        self.scratch = events;
        self.verify_dolby_vision();
    }

    /// A network stream that ends long before its duration was cut off, not finished.
    fn should_recover(&self) -> bool {
        let remote = self.source.as_ref().is_some_and(|s| s.url.starts_with("http"));
        remote && self.state.started && self.recoveries < MAX_RECOVERIES && self.state.duration > 30.0 && self.state.position < self.state.duration - 20.0
    }

    fn recover(&mut self) {
        let Some(source) = self.source.clone() else { return };
        self.recoveries += 1;
        let at = self.state.position;
        tracing::warn!(attempt = self.recoveries, position = at as u64, "the stream ended early, opening it again");
        let extra = self.last_extra.clone();
        self.load(&source, &extra, at);
    }

    fn apply_property(&mut self, name: &str, value: &Value) {
        match (name, value) {
            ("time-pos", v) => self.state.position = v.as_f64().unwrap_or(self.state.position),
            ("duration", v) => self.state.duration = v.as_f64().unwrap_or(0.0),
            ("pause", Value::Flag(v)) => self.state.paused = *v,
            ("paused-for-cache", Value::Flag(v)) => self.state.buffering = *v,
            ("cache-buffering-state", Value::Int(v)) => self.state.buffer_percent = *v,
            ("demuxer-cache-time", v) => {
                self.state.cached_secs = (v.as_f64().unwrap_or(0.0) - self.state.position).max(0.0);
            }
            ("volume", v) => self.state.volume = v.as_f64().unwrap_or(self.state.volume),
            ("mute", Value::Flag(v)) => self.state.muted = *v,
            ("speed", v) => self.state.speed = v.as_f64().unwrap_or(1.0),
            ("seeking", Value::Flag(v)) => self.state.seeking = *v,
            ("container-fps", Value::Double(fps)) => self.surface.clock.set_fps(*fps),
            ("eof-reached", Value::Flag(true)) if self.should_recover() => self.recover(),
            ("eof-reached", Value::Flag(reached)) => self.state.ended = *reached,
            ("hwdec-current", Value::Text(v)) => self.state.hwdec = v.clone(),
            ("track-list", Value::Text(json)) => match serde_json::from_str::<Vec<Track>>(json) {
                Ok(tracks) => {
                    self.state.tracks = tracks;
                    if self.state.loaded {
                        self.detect_dolby_vision();
                    }
                }
                Err(error) => tracing::warn!(%error, "could not read the track list"),
            },
            ("video-params/w", Value::Int(w)) => self.state.video_size = Some((*w, self.state.video_size.map(|s| s.1).unwrap_or(0))),
            ("video-params/h", Value::Int(h)) => self.state.video_size = Some((self.state.video_size.map(|s| s.0).unwrap_or(0), *h)),
            ("video-out-params/colormatrix", Value::Text(matrix)) => self.state.out_matrix = Some(matrix.clone()),
            _ => {}
        }
    }

    pub fn tracks_of(&self, kind: &str) -> impl Iterator<Item = &Track> {
        self.state.tracks.iter().filter(move |t| t.kind == kind)
    }

    pub fn render_error(&self) -> Option<String> {
        self.surface.failed.lock().ok().and_then(|f| f.clone())
    }

    /// The shape that draws the video behind everything else in `rect`. mpv fills the whole window, so the video
    /// area is the window; the controls are painted on top of it.
    pub fn video_shape(&self, rect: Rect) -> egui::Shape {
        let surface = self.surface.clone();
        PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, _painter| surface.paint(&info))),
        }
        .into()
    }

}

impl Drop for Player {
    /// Players are only dropped from the UI thread, where the GL context is current.
    fn drop(&mut self) {
        self.surface.release();
    }
}
