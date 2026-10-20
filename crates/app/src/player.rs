//! The player: one libmpv instance drawing into the window through OpenGL, plus the state the controls show.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{self, PaintCallback, Rect};
use eframe::egui_glow;
use nuvio_mpv::{EndReason, Event, Mpv, Observe, RenderContext, Value};
use serde::Deserialize;

/// Seconds of media buffered before a stalled stream resumes. A first start needs less, a stall deserves more.
const START_BUFFER_SECS: u32 = 1;
const REBUFFER_SECS: u32 = 5;

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
}

impl Track {
    pub fn label(&self) -> String {
        let language = self.lang.as_deref().map(language_name).unwrap_or_default();
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
        if parts.is_empty() { format!("Track {}", self.id) } else { parts.join(" Â· ") }
    }
}

/// ISO 639 codes mpv reports, in the languages people actually pick between.
fn language_name(code: &str) -> String {
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
}

/// What a player needs to know to open a stream.
pub struct Source {
    pub url: String,
    pub start: f64,
    pub headers: Vec<(String, String)>,
    pub subtitles: Vec<String>,
}

/// The GL half of the player, shared with the paint callback that runs inside egui's render pass.
struct Surface {
    mpv: Arc<Mpv>,
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
            let ctx = self.ctx.clone();
            match RenderContext::new_opengl(&self.mpv, move || ctx.request_repaint()) {
                Ok(context) => {
                    *slot = Some(GlSide(context));
                    self.ready.store(true, Ordering::Release);
                    self.ctx.request_repaint();
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

pub struct Player {
    mpv: Arc<Mpv>,
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
        let hwdec = std::env::var("NUVIO_MPV_HWDEC").unwrap_or_else(|_| if hardware_decoding { "auto-safe".into() } else { "no".into() });
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
        let mpv = Arc::new(Mpv::new(&borrowed, move || repaint.request_repaint())?);
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
            ("hwdec-current", Observe::Text),
        ] {
            mpv.observe(name, kind);
        }
        let surface = Arc::new(Surface {
            mpv: mpv.clone(),
            ctx: ctx.clone(),
            render: Mutex::new(None),
            failed: Mutex::new(None),
            ready: AtomicBool::new(false),
        });
        Ok(Self {
            mpv,
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
        self.surface.ctx.request_repaint();
    }

    fn start_pending(&mut self) {
        let Some(source) = self.pending.take() else { return };
        self.state = State { volume: self.state.volume, muted: self.state.muted, speed: 1.0, ..State::default() };
        self.rebuffer_raised = false;
        self.opened = Instant::now();
        let _ = self.mpv.set_str("cache-pause-wait", &START_BUFFER_SECS.to_string());
        let headers = source
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>();
        let _ = self.mpv.set_str("http-header-fields", &headers.join(","));
        let mut options = Vec::new();
        if source.start > 1.0 {
            options.push(format!("start={:.3}", source.start));
        }
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

    pub fn stop(&self) {
        let _ = self.mpv.command(&["stop"]);
    }

    pub fn property(&self, name: &str) -> Option<String> {
        self.mpv.get_str(name)
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
                    self.refresh_tracks();
                    if let Some(w) = self.mpv.get_i64("video-params/w") {
                        let h = self.mpv.get_i64("video-params/h").unwrap_or(0);
                        self.state.video_size = Some((w, h));
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
            ("hwdec-current", Value::Text(v)) => self.state.hwdec = v.clone(),
            _ => {}
        }
    }

    fn refresh_tracks(&mut self) {
        if let Some(json) = self.mpv.get_str("track-list") {
            match serde_json::from_str::<Vec<Track>>(&json) {
                Ok(tracks) => self.state.tracks = tracks,
                Err(error) => tracing::warn!(%error, "could not read the track list"),
            }
        }
    }

    pub fn tracks_of(&self, kind: &str) -> impl Iterator<Item = &Track> {
        self.state.tracks.iter().filter(move |t| t.kind == kind)
    }

    /// Re-reads the track list (after an external subtitle was added, for example).
    pub fn reload_tracks(&mut self) {
        self.refresh_tracks();
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

    /// Frees the GL resources while the context is still current (window closing, leaving the player).
    pub fn release_gl(&self) {
        self.surface.release();
    }
}

impl Drop for Player {
    /// Players are only dropped from the UI thread, where the GL context is current.
    fn drop(&mut self) {
        self.surface.release();
    }
}
