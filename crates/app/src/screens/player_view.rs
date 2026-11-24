//! The player screen: the video fills the window and the controls float over it. They appear when the pointer moves
//! and fade out (taking the cursor with them) after a few seconds of stillness while a video plays.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CursorIcon, Key, Popup, Rect, Sense, Ui, pos2, vec2};
use nuvio_core::meta::{MetaDetails, Video};
use nuvio_core::segments::Segments;
use nuvio_core::stream::AddonSubtitle;

use super::{Action, PlayRequest, opening};
use crate::icons;
use crate::images::Images;
use crate::mpv_custom::{self, Custom};
use crate::player::{Player, Track};
use crate::theme::{self, size};
use crate::ui_kit::{self, animate, lerp_color, radius_of, with_alpha};

const HIDE_AFTER: Duration = Duration::from_millis(3500);
const HUD_SHOW: Duration = Duration::from_millis(1400);
/// How long the next episode, or a skip, is announced before it happens by itself.
const AUTOPLAY_SECS: f32 = 5.0;
const SKIP_SECS: f32 = 5.0;
/// Where the credits are taken to begin when nobody has marked them: this many seconds before the end.
const FALLBACK_CREDITS_SECS: f64 = 40.0;
const SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

pub enum Hud {
    Volume(f64, bool),
    Seek(String),
    Speed(f64),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Aspect {
    Fit,
    Wide,
    Standard,
    Cinema,
    Fill,
}

impl Aspect {
    const ALL: [(Aspect, &'static str); 5] = [
        (Aspect::Fit, "Fit"),
        (Aspect::Wide, "16:9"),
        (Aspect::Standard, "4:3"),
        (Aspect::Cinema, "21:9"),
        (Aspect::Fill, "Fill the window"),
    ];
}

/// A torrent stream being prepared or played: the numbers to show, and what to remove afterwards.
pub struct Torrent {
    pub request: u64,
    pub status: crate::p2p::Status,
    pub info_hash: String,
}

/// A track the viewer had picked, by what identifies it in another file.
#[derive(Clone, Debug)]
pub struct TrackPick {
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubtitlePick {
    /// Nothing was chosen: the settings decide.
    Unchanged,
    Off,
    On { language: Option<String>, title: Option<String>, addon: Option<String> },
}

/// What the viewer chose while watching an episode, handed to the next one (or the one before) so none of it is chosen
/// again: the volume, the audio track, the subtitles, the speed and the picture size.
#[derive(Clone, Debug)]
pub struct Carry {
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub aspect: Aspect,
    pub audio: Option<TrackPick>,
    pub subtitles: SubtitlePick,
}

impl Carry {
    /// The preferred audio language, subtitle language and whether subtitles are on, as the player takes them: what was
    /// carried over where there is something, the settings where there is not.
    pub fn track_preferences(&self, settings: &nuvio_core::store::Settings) -> (String, String, bool) {
        let audio = self.audio.as_ref().and_then(|a| a.language.clone()).unwrap_or_else(|| settings.preferred_audio_language.clone());
        match &self.subtitles {
            SubtitlePick::Unchanged => (audio, settings.preferred_subtitle_language.clone(), settings.subtitles_on),
            SubtitlePick::Off => (audio, settings.preferred_subtitle_language.clone(), false),
            SubtitlePick::On { language, .. } => (audio, language.clone().unwrap_or_else(|| settings.preferred_subtitle_language.clone()), true),
        }
    }
}

/// The track of `tracks` that `pick` names: the language, and among several in it the title.
fn best_match<'a>(tracks: impl Iterator<Item = &'a Track>, pick: &TrackPick) -> Option<&'a Track> {
    let same_language = |t: &&Track| match (&pick.language, &t.lang) {
        (Some(wanted), Some(have)) => wanted.eq_ignore_ascii_case(have),
        (None, _) => pick.title.is_some() && pick.title == t.title,
        _ => false,
    };
    let mut candidates: Vec<&Track> = tracks.filter(|t| same_language(t)).collect();
    candidates.sort_by_key(|t| std::cmp::Reverse(pick.title.is_some() && pick.title == t.title));
    candidates.into_iter().next()
}

/// A stretch of an episode that is skipped a few seconds after it begins.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Skippable {
    Intro,
    Recap,
    Credits,
}

impl Skippable {
    fn label(self) -> &'static str {
        match self {
            Skippable::Intro => "Skip intro",
            Skippable::Recap => "Skip recap",
            Skippable::Credits => "Skip credits",
        }
    }

    fn done(self) -> &'static str {
        match self {
            Skippable::Intro => "Intro skipped",
            Skippable::Recap => "Recap skipped",
            Skippable::Credits => "Credits skipped",
        }
    }
}

pub struct Session {
    pub player: Player,
    pub torrent: Option<Torrent>,
    /// Subtitle files addons offered for this video, filled in a moment after the player opens.
    pub online_subtitles: Vec<AddonSubtitle>,
    pub subtitles_request: u64,
    /// Whether the preferred subtitle language was already switched on from an addon's list.
    pub subtitle_chosen: bool,
    pub request: PlayRequest,
    pub meta: Option<Arc<MetaDetails>>,
    /// The title's wordmark, shown while the stream opens.
    pub logo: Option<String>,
    /// Where the intro, the recap and the credits are, once IntroDB has answered (`Msg::Segments`).
    pub segments: Segments,
    pub segments_request: u64,
    /// What the episode before this one left behind, when this one was opened from it.
    pub carry: Option<Carry>,
    carry_tracks_applied: bool,
    skip_timer: Option<(Skippable, Instant)>,
    skipped: Vec<Skippable>,
    opened: Instant,
    last_motion: Instant,
    hud: Option<(Hud, Instant)>,
    drag: Option<f64>,
    aspect: Aspect,
    show_stats: bool,
    pub last_saved: Instant,
    ended_at: Option<Instant>,
    autoplay_cancelled: bool,
    last_volume: f64,
    /// When the wheel last turned, and how many notches have followed each other closely since.
    wheel_at: Instant,
    wheel_streak: u32,
    /// The pointer position last sent to mpv.
    mouse_sent: Option<(egui::Pos2, Instant)>,
}

impl Session {
    pub fn new(player: Player, request: PlayRequest, meta: Option<Arc<MetaDetails>>) -> Self {
        Self {
            player,
            torrent: None,
            online_subtitles: Vec::new(),
            subtitles_request: 0,
            subtitle_chosen: false,
            request,
            logo: meta.as_ref().and_then(|m| m.logo.clone()),
            meta,
            segments: Segments::default(),
            segments_request: 0,
            carry: None,
            carry_tracks_applied: false,
            skip_timer: None,
            skipped: Vec::new(),
            opened: Instant::now(),
            last_motion: Instant::now(),
            hud: None,
            drag: None,
            aspect: Aspect::Fit,
            show_stats: false,
            last_saved: Instant::now(),
            ended_at: None,
            autoplay_cancelled: false,
            last_volume: 100.0,
            wheel_at: Instant::now(),
            wheel_streak: 0,
            mouse_sent: None,
        }
    }

    pub fn episode_label(&self) -> Option<String> {
        let video = self.request.video.as_ref()?;
        match &video.title {
            title if !title.is_empty() => Some(format!("{} • {title}", video.code()?)),
            _ => video.code(),
        }
    }

    fn ordered_videos(&self) -> Vec<&Video> {
        let Some(meta) = &self.meta else { return Vec::new() };
        meta.seasons().into_iter().filter(|s| *s > 0).flat_map(|s| meta.episodes_of(s)).collect()
    }

    pub fn next_episode(&self) -> Option<Video> {
        let list = self.ordered_videos();
        let index = list.iter().position(|v| v.id == self.request.video_id)?;
        list.get(index + 1).map(|v| (*v).clone())
    }

    pub fn previous_episode(&self) -> Option<Video> {
        let list = self.ordered_videos();
        let index = list.iter().position(|v| v.id == self.request.video_id)?;
        index.checked_sub(1).and_then(|i| list.get(i)).map(|v| (*v).clone())
    }

    /// What to hand to the episode that follows (or precedes) this one.
    pub fn carry_out(&self) -> Carry {
        let state = &self.player.state;
        let audio = self.player.tracks_of("audio").find(|t| t.selected).map(|t| TrackPick { language: t.lang.clone(), title: t.title.clone() });
        let subtitles = match self.player.tracks_of("sub").find(|t| t.selected) {
            Some(track) => SubtitlePick::On {
                language: track.lang.clone(),
                title: track.title.clone().filter(|_| !track.external),
                // A subtitle from an addon is titled "Language · Addon".
                addon: track.title.as_deref().filter(|_| track.external).and_then(|title| title.split(" · ").nth(1)).map(str::to_owned),
            },
            // Subtitles that were there and switched off are a choice; none to choose from is not.
            None if state.loaded && self.player.tracks_of("sub").next().is_some() => SubtitlePick::Off,
            None => SubtitlePick::Unchanged,
        };
        Carry { volume: state.volume, muted: state.muted, speed: state.speed, aspect: self.aspect, audio, subtitles }
    }

    /// Takes over what the episode before left behind. The player already has its volume, speed and track languages (see
    /// `App::make_player`); the picture size and what is chosen among the tracks of the file come here.
    pub fn take_carry(&mut self, carry: Carry) {
        self.apply_aspect(carry.aspect);
        self.subtitle_chosen = carry.subtitles == SubtitlePick::Off;
        self.carry = Some(carry);
    }

    /// Once the tracks of the file are known: the audio track and the subtitles the viewer had, among those in it. (An
    /// addon's subtitles come later and are chosen when they arrive.)
    fn apply_carry_tracks(&mut self) {
        if self.carry_tracks_applied || !self.player.state.loaded || self.player.state.tracks.is_empty() {
            return;
        }
        let Some(carry) = self.carry.clone() else { return };
        self.carry_tracks_applied = true;
        if let Some(pick) = &carry.audio {
            let target = best_match(self.player.tracks_of("audio"), pick).map(|t| (t.id, t.selected));
            if let Some((id, false)) = target {
                self.player.select_audio(id);
            }
        }
        match &carry.subtitles {
            SubtitlePick::Off => {
                if self.player.tracks_of("sub").any(|t| t.selected) {
                    self.player.select_subtitle(None);
                }
            }
            SubtitlePick::On { language, title, .. } => {
                let pick = TrackPick { language: language.clone(), title: title.clone() };
                let target = best_match(self.player.tracks_of("sub").filter(|t| !t.external), &pick).map(|t| (t.id, t.selected));
                if let Some((id, selected)) = target {
                    if !selected {
                        self.player.select_subtitle(Some(id));
                    }
                    self.subtitle_chosen = true;
                }
            }
            SubtitlePick::Unchanged => {}
        }
    }

    /// The stretch being played that is skipped by itself, and where it ends. The credits are left to the "up next" card when
    /// there is an episode to go to (`credits_card`).
    fn segment_now(&self, credits_card: bool) -> Option<(Skippable, f64)> {
        let state = &self.player.state;
        if !state.started || state.ended {
            return None;
        }
        let position = state.position;
        [(Skippable::Intro, self.segments.intro), (Skippable::Recap, self.segments.recap), (Skippable::Credits, self.segments.outro.filter(|_| !credits_card))]
            .into_iter()
            .find(|(kind, span)| !self.skipped.contains(kind) && span.is_some_and(|s| s.contains(position)))
            .and_then(|(kind, span)| span.map(|s| (kind, s.end)))
    }

    /// Whether the credits are rolling: from where they begin when that is known, else for the last seconds of the file.
    fn credits_now(&self) -> bool {
        let state = &self.player.state;
        if state.ended {
            return true;
        }
        if !state.started || state.duration <= 0.0 {
            return false;
        }
        match self.segments.outro {
            Some(outro) if outro.start > state.duration * 0.6 => state.position >= outro.start,
            _ => state.duration > 300.0 && state.duration - state.position <= FALLBACK_CREDITS_SECS,
        }
    }

    fn show_hud(&mut self, hud: Hud) {
        self.hud = Some((hud, Instant::now()));
    }

    fn apply_aspect(&mut self, aspect: Aspect) {
        self.aspect = aspect;
        let (ratio, pan) = match aspect {
            Aspect::Fit => ("-1", 0.0),
            Aspect::Wide => ("16:9", 0.0),
            Aspect::Standard => ("4:3", 0.0),
            Aspect::Cinema => ("2.35:1", 0.0),
            Aspect::Fill => ("-1", 1.0),
        };
        self.player.set_property_text("video-aspect-override", ratio);
        self.player.set_property_number("panscan", pan);
    }

    fn adjust_volume(&mut self, delta: f64) {
        let volume = (self.player.state.volume + delta).clamp(0.0, 130.0);
        self.player.set_volume(volume);
        if self.player.state.muted && delta > 0.0 {
            self.player.set_muted(false);
        }
        self.show_hud(Hud::Volume(volume, self.player.state.muted));
    }

    fn toggle_mute(&mut self) {
        let muted = !self.player.state.muted;
        self.player.set_muted(muted);
        self.show_hud(Hud::Volume(self.player.state.volume, muted));
    }

    fn seek_by(&mut self, seconds: f64) {
        self.player.seek_by(seconds);
        let sign = if seconds < 0.0 { "−" } else { "+" };
        self.show_hud(Hud::Seek(format!("{sign}{:.0} s", seconds.abs())));
    }
}

pub fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// Draws the screen and handles its input. Returns the actions the app should carry out.
pub fn show(session: &mut Session, ui: &mut Ui, images: &Images, actions: &mut Vec<Action>, fullscreen: bool, auto_play_next: bool, auto_skip: bool) {
    let ctx = ui.ctx().clone();
    let t = theme::get();
    let rect = ui.max_rect();
    session.player.pump();
    session.apply_carry_tracks();

    // The video, behind everything.
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    ui.painter().add(session.player.video_shape(rect));

    // --- input ------------------------------------------------------------------------------------------------------
    let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_pressed() || i.smooth_scroll_delta != egui::Vec2::ZERO);
    if moved {
        session.last_motion = Instant::now();
    }
    let video_area = ui.interact(rect, egui::Id::unique("video-area"), Sense::click());
    // A mouse button the person's input.conf binds goes to mpv instead of doing what it does here.
    let custom = session.player.custom().clone();
    if video_area.double_clicked() {
        if custom.binds("MBTN_LEFT_DBL") {
            session.player.send_key("MBTN_LEFT_DBL");
        } else {
            actions.push(Action::Fullscreen(!fullscreen));
        }
    } else if video_area.clicked() && session.player.state.started {
        if custom.binds("MBTN_LEFT") {
            session.player.send_key("MBTN_LEFT");
        } else {
            session.player.toggle_pause();
        }
    }
    if video_area.secondary_clicked() && custom.binds("MBTN_RIGHT") {
        session.player.send_key("MBTN_RIGHT");
    }
    if video_area.middle_clicked() && custom.binds("MBTN_MID") {
        session.player.send_key("MBTN_MID");
    }
    handle_keys(session, &ctx, actions, fullscreen, &custom);
    handle_wheel(session, &ctx, rect, &custom);
    forward_input(session, &ctx, &custom);

    let state = session.player.state.clone();
    let torrent_error = session.torrent.as_ref().and_then(|t| match t.status.snapshot().phase {
        crate::p2p::Phase::Failed(error) => Some(error),
        _ => None,
    });
    let error = state.error.clone().or(torrent_error).or_else(|| session.player.render_error());
    // Until the first picture, the artwork of the title stands in for the video; it fades out when the picture starts.
    let opening_now = !state.started && error.is_none();
    // Keyed by the session: a new one starts with the overlay fully up, whatever the last one ended with.
    let overlay = animate(ui, egui::Id::unique(("opening-visible", session.opened)), opening_now, theme::motion::NORMAL);

    let menu_open = Popup::is_any_open(&ctx);
    let idle = session.last_motion.elapsed() > HIDE_AFTER;
    let show_controls = !idle || state.paused || menu_open || session.drag.is_some() || error.is_some() || !state.started;
    let visible = animate(ui, egui::Id::unique("controls-visible"), show_controls, theme::motion::NORMAL);
    if !show_controls && !state.paused {
        ctx.set_cursor_icon(CursorIcon::None);
    }
    if state.started && !state.paused && !idle {
        ctx.request_repaint_after(HIDE_AFTER.saturating_sub(session.last_motion.elapsed()) + Duration::from_millis(50));
    }

    // --- opening ----------------------------------------------------------------------------------------------------
    if overlay > 0.01 {
        let message = opening_message(session, &state);
        let artwork = session.request.background.as_deref().or(session.request.poster.as_deref());
        let content = opening::Opening { artwork, logo: session.logo.as_deref(), title: &session.request.name, message: Some(&message) };
        if opening::show(ui, rect, images, &content, session.opened.elapsed().as_secs_f32(), overlay) {
            actions.push(Action::Back);
        }
    }

    // --- buffering --------------------------------------------------------------------------------------------------
    // After the picture has started, a stall is a spinner over the frozen frame.
    if state.started && state.buffering && error.is_none() {
        let center = rect.center();
        ui_kit::glass_panel(ui, Rect::from_center_size(center, vec2(76.0, 76.0)), 38.0, 1.0);
        ui_kit::spinner(ui, center, 18.0, Color32::WHITE);
        let text = if state.buffer_percent > 0 { format!("Buffering {}%", state.buffer_percent) } else { "Buffering…".to_owned() };
        let label = ui_kit::galley(ui, &text, theme::semibold(size::BODY_MD), with_alpha(Color32::WHITE, 0.95), 300.0, 1);
        let pill = Rect::from_center_size(center + vec2(0.0, 68.0), label.size() + vec2(36.0, 18.0));
        ui_kit::glass_panel(ui, pill, pill.height() / 2.0, 1.0);
        ui.painter().galley(pill.center() - label.size() / 2.0, label, Color32::WHITE);
    }

    // --- errors -----------------------------------------------------------------------------------------------------
    if let Some(error) = error {
        error_card(ui, rect, &error, actions);
    }

    // --- controls ---------------------------------------------------------------------------------------------------
    if visible > 0.01 && !opening_now {
        controls(session, ui, rect, visible, actions, fullscreen);
    }
    hud(session, ui, rect);
    if session.show_stats {
        stats(session, ui, rect);
    }
    if !opening_now && state.error.is_none() {
        let credits_card = auto_play_next && session.next_episode().is_some();
        skip_prompt(session, ui, rect, auto_skip, credits_card);
    }
    next_episode_card(session, ui, rect, actions, auto_play_next);
    let _ = &t;
}

/// The line under the title while a stream opens, in the words the Kotlin build uses.
fn opening_message(session: &Session, state: &crate::player::State) -> String {
    if let Some(torrent) = &session.torrent {
        return torrent_text(&torrent.status.snapshot());
    }
    if !state.loaded {
        "Preparing stream…".to_owned()
    } else if state.buffering {
        "Buffering…".to_owned()
    } else {
        "Starting stream…".to_owned()
    }
}

fn torrent_text(snapshot: &crate::p2p::Snapshot) -> String {
    use crate::p2p::Phase;
    let megabytes = |bytes: f64| bytes / 1_048_576.0;
    match &snapshot.phase {
        Phase::FindingPeers if snapshot.peers == 0 => "Looking for peers…".to_owned(),
        Phase::FindingPeers => format!("Looking for the file list · {} peers", snapshot.peers),
        Phase::Buffering | Phase::Failed(_) => format!(
            "Downloading the start · {} peers · {:.1} MB/s · {:.0} of {:.0} MB",
            snapshot.peers,
            megabytes(snapshot.download_bytes_per_sec),
            megabytes(snapshot.downloaded as f64),
            megabytes(snapshot.file_size as f64)
        ),
    }
}

/// What a turn of the wheel does to the volume per notch, before it speeds up.
const WHEEL_VOLUME_STEP: f64 = 2.0;
/// Notches closer together than this belong to one flick of the wheel.
const WHEEL_STREAK_GAP: Duration = Duration::from_millis(140);

/// The wheel turns the volume like a knob: one notch moves it a little, and a run of notches in quick succession moves it
/// more with each one, so a flick covers the range while a slow turn can be stopped on any value. (It used to add five for
/// every frame the smoothed scroll lasted, which sent it to either end at once.)
fn handle_wheel(session: &mut Session, ctx: &egui::Context, rect: Rect, custom: &Custom) {
    if Popup::is_any_open(ctx) || !ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p))) {
        return;
    }
    let notches: f32 = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 10.0,
                    // A touchpad reports points: about forty of them make one notch of a wheel.
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                }),
                _ => None,
            })
            .sum()
    });
    if notches == 0.0 {
        return;
    }
    let key = if notches > 0.0 { "WHEEL_UP" } else { "WHEEL_DOWN" };
    if custom.binds(key) {
        for _ in 0..(notches.abs().round() as usize).clamp(1, 10) {
            session.player.send_key(key);
        }
        return;
    }
    session.wheel_streak = if session.wheel_at.elapsed() <= WHEEL_STREAK_GAP { (session.wheel_streak + 1).min(8) } else { 0 };
    session.wheel_at = Instant::now();
    let step = WHEEL_VOLUME_STEP * (1.0 + 0.35 * session.wheel_streak as f64);
    session.adjust_volume(notches as f64 * step);
}

/// Hands mpv the keys and pointer movements Nuvio has no use for, so the scripts and bindings the person loaded can act on
/// them. The keys the player screen uses stay its own unless `input.conf` binds them (see `mpv_custom::is_nuvios`).
fn forward_input(session: &mut Session, ctx: &egui::Context, custom: &Custom) {
    if !custom.enabled || ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    let names: Vec<String> = ctx.input(|i| {
        let mut out = Vec::new();
        for event in &i.events {
            match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if let Some(name) = mpv_custom::named_key(*key) {
                        out.push(mpv_custom::with_modifiers(name, modifiers));
                    } else if modifiers.ctrl || modifiers.alt {
                        // With Ctrl or Alt held a key types no text, so it is named after the key itself.
                        if let Some(plain) = mpv_custom::plain_key(*key, modifiers.shift) {
                            out.push(mpv_custom::with_modifiers(&plain.to_string(), modifiers));
                        }
                    }
                }
                // What the key typed: the right character for the keyboard layout and for Shift.
                egui::Event::Text(text) if !i.modifiers.ctrl && !i.modifiers.alt => {
                    out.extend(text.chars().filter(|c| !c.is_control() && *c != ' ').map(|c| c.to_string()));
                }
                _ => {}
            }
        }
        out
    });
    for name in names {
        if !mpv_custom::is_nuvios(&name) || custom.binds(&name) {
            session.player.send_key(&name);
        }
    }
    // The pointer, at most thirty times a second.
    if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
        let ppp = ctx.pixels_per_point();
        let moved = session.mouse_sent.is_none_or(|(at, when)| at != pos && when.elapsed() >= Duration::from_millis(33));
        if moved {
            session.player.send_mouse(pos.x * ppp, pos.y * ppp);
            session.mouse_sent = Some((pos, Instant::now()));
        }
    }
}

fn handle_keys(session: &mut Session, ctx: &egui::Context, actions: &mut Vec<Action>, fullscreen: bool, custom: &Custom) {
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    // A key that the person's input.conf binds belongs to mpv, so none of the player's own code below runs for it.
    let mine = |name: &str| !custom.binds(name);
    let (space, k, left, right, up, down, m, f, f11, esc, shift, ctrl, alt, speed_up, speed_down, back_nav) = ctx.input(|i| {
        (
            i.key_pressed(Key::Space) && mine("SPACE"),
            i.key_pressed(Key::K) && mine("k"),
            (i.key_pressed(Key::ArrowLeft) && mine("LEFT")) || (i.key_pressed(Key::J) && mine("j")),
            (i.key_pressed(Key::ArrowRight) && mine("RIGHT")) || (i.key_pressed(Key::L) && mine("l")),
            i.key_pressed(Key::ArrowUp) && mine("UP"),
            i.key_pressed(Key::ArrowDown) && mine("DOWN"),
            i.key_pressed(Key::M) && mine("m"),
            i.key_pressed(Key::F) && mine("f"),
            i.key_pressed(Key::F11) && mine("F11"),
            i.key_pressed(Key::Escape) && mine("ESC"),
            i.modifiers.shift,
            i.modifiers.command,
            i.modifiers.alt,
            i.key_pressed(Key::Period) && i.modifiers.shift,
            i.key_pressed(Key::Comma) && i.modifiers.shift,
            i.pointer.button_pressed(egui::PointerButton::Extra1),
        )
    });
    if space || k {
        session.player.toggle_pause();
        session.last_motion = Instant::now();
    }
    let step = if ctrl { 60.0 } else if shift { 30.0 } else { 10.0 };
    if left && !alt {
        session.seek_by(-step);
    }
    if right && !alt {
        session.seek_by(step);
    }
    if up {
        session.adjust_volume(5.0);
    }
    if down {
        session.adjust_volume(-5.0);
    }
    if m {
        session.toggle_mute();
    }
    if f || f11 {
        actions.push(Action::Fullscreen(!fullscreen));
    }
    if speed_up || speed_down {
        let current = session.player.state.speed;
        let index = SPEEDS.iter().position(|s| (*s - current).abs() < 0.01).unwrap_or(2);
        let next = if speed_up { (index + 1).min(SPEEDS.len() - 1) } else { index.saturating_sub(1) };
        session.player.set_speed(SPEEDS[next]);
        session.show_hud(Hud::Speed(SPEEDS[next]));
    }
    if esc {
        if fullscreen {
            actions.push(Action::Fullscreen(false));
        } else {
            actions.push(Action::Back);
        }
    }
    if back_nav || (alt && left) {
        actions.push(Action::Back);
    }
}

fn controls(session: &mut Session, ui: &mut Ui, rect: Rect, visible: f32, actions: &mut Vec<Action>, fullscreen: bool) {
    let t = theme::get();
    let a = visible;
    let state = session.player.state.clone();

    // Top: the way back and the title, each on a piece of glass over the picture.
    let back = Rect::from_min_size(rect.min + vec2(24.0, 22.0), vec2(48.0, 48.0));
    ui_kit::glass_panel(ui, back, 24.0, a);
    let response = ui.interact(back, egui::Id::unique("player-back"), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().circle_filled(back.center(), 24.0, with_alpha(Color32::WHITE, 0.14 * hover * a));
    ui_kit::draw_icon(ui, back.center(), icons::ARROW_BACK, 26.0, with_alpha(Color32::WHITE, a));
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if response.on_hover_text("Back").clicked() {
        actions.push(Action::Back);
    }
    let title = ui_kit::galley(ui, &session.request.name, theme::bold(size::TITLE_SM), with_alpha(Color32::WHITE, a), (rect.width() * 0.5).max(200.0), 1);
    let label = session.episode_label().map(|l| ui_kit::galley(ui, &l, theme::regular(size::BODY_SM), with_alpha(Color32::WHITE, 0.78 * a), 600.0, 1));
    let text_w = title.size().x.max(label.as_ref().map_or(0.0, |g| g.size().x));
    let text_h = title.size().y + label.as_ref().map_or(0.0, |g| g.size().y + 2.0);
    let pill = Rect::from_center_size(pos2(back.right() + 12.0 + (text_w + 44.0) / 2.0, back.center().y), vec2(text_w + 44.0, (text_h + 20.0).max(48.0)));
    ui_kit::glass_panel(ui, pill, (pill.height() / 2.0).min(26.0), a);
    ui.painter().galley(pos2(pill.left() + 22.0, pill.center().y - text_h / 2.0), title.clone(), Color32::WHITE);
    if let Some(label) = label {
        ui.painter().galley(pos2(pill.left() + 22.0, pill.center().y - text_h / 2.0 + title.size().y + 2.0), label, Color32::WHITE);
    }

    // Centre: play / pause with the 10 second jumps, shown while paused.
    let paused_alpha = animate(ui, egui::Id::unique("center-visible"), state.paused && state.started, theme::motion::NORMAL);
    if paused_alpha > 0.01 {
        let center = rect.center();
        for (dx, icon, secs, tip) in [(-150.0, icons::REPLAY_10, -10.0, "Back 10 seconds"), (150.0, icons::FORWARD_10, 10.0, "Forward 10 seconds")] {
            let r = Rect::from_center_size(center + vec2(dx, 0.0), vec2(68.0, 68.0));
            ui_kit::glass_panel(ui, r, 34.0, paused_alpha);
            let resp = ui.interact(r, egui::Id::unique(("center-jump", dx as i32)), Sense::click());
            let h = animate(ui, resp.id.with("h"), resp.hovered(), 0.1);
            ui.painter().circle_filled(r.center(), 34.0, with_alpha(Color32::WHITE, 0.14 * h * paused_alpha));
            ui_kit::draw_icon(ui, r.center(), icon, 40.0, with_alpha(Color32::WHITE, paused_alpha));
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.on_hover_text(tip).clicked() {
                session.seek_by(secs);
            }
        }
        let r = Rect::from_center_size(center, vec2(92.0, 92.0));
        ui_kit::glass_panel(ui, r, 46.0, paused_alpha);
        let resp = ui.interact(r, egui::Id::unique("center-play"), Sense::click());
        let h = animate(ui, resp.id.with("h"), resp.hovered(), 0.12);
        ui.painter().circle_filled(r.center(), 46.0, with_alpha(Color32::WHITE, 0.14 * h * paused_alpha));
        ui_kit::draw_icon(ui, r.center(), icons::PLAY, 58.0, with_alpha(Color32::WHITE, paused_alpha));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.on_hover_text("Play").clicked() {
            session.player.set_paused(false);
        }
    }

    // Bottom: one floating panel of glass with the timeline above the buttons.
    let panel_w = (rect.width() - 40.0).min(1500.0);
    let panel = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 22.0 - 55.0), vec2(panel_w, 110.0));
    ui_kit::glass_panel(ui, panel, 32.0, a);
    let bar_left = panel.left() + 30.0;
    let bar_right = panel.right() - 30.0;
    timeline(session, ui, bar_left, bar_right, panel.top() + 30.0, a);

    // Bottom: buttons.
    let y = panel.bottom() - 36.0;
    let mut x = bar_left - 12.0;
    let button = |ui: &mut Ui, x: &mut f32, icon: char, tip: &str, px: f32| -> bool {
        let r = Rect::from_center_size(pos2(*x + 24.0, y), vec2(48.0, 48.0));
        let resp = ui.interact(r, egui::Id::unique(("player-btn", tip)), Sense::click());
        let h = animate(ui, resp.id.with("h"), resp.hovered(), theme::motion::FAST);
        ui.painter().circle_filled(r.center(), 22.0, with_alpha(Color32::WHITE, 0.14 * h * a));
        ui_kit::draw_icon(ui, r.center(), icon, px, with_alpha(lerp_color(Color32::from_gray(225), Color32::WHITE, h), a));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        *x += 48.0;
        resp.on_hover_text(tip).clicked()
    };

    let play_icon = if state.paused { icons::PLAY } else { icons::PAUSE };
    if button(ui, &mut x, play_icon, if state.paused { "Play  (Space)" } else { "Pause  (Space)" }, 32.0) {
        session.player.toggle_pause();
    }
    let has_prev = session.previous_episode();
    let has_next = session.next_episode();
    if has_prev.is_some() && button(ui, &mut x, icons::SKIP_PREVIOUS, "Previous episode", 28.0) {
        if let Some(video) = has_prev {
            actions.push(Action::PlayEpisode(video));
        }
    }
    if has_next.is_some() && button(ui, &mut x, icons::SKIP_NEXT, "Next episode", 28.0) {
        if let Some(video) = has_next.clone() {
            actions.push(Action::PlayEpisode(video));
        }
    }

    // Volume: the icon mutes, the slider sets.
    let volume_icon = if state.muted || state.volume < 0.5 {
        icons::VOLUME_OFF
    } else if state.volume < 40.0 {
        icons::VOLUME_DOWN
    } else {
        icons::VOLUME_UP
    };
    if button(ui, &mut x, volume_icon, "Mute  (M)", 26.0) {
        session.toggle_mute();
    }
    let slider = Rect::from_min_size(pos2(x + 4.0, y - 12.0), vec2(110.0, 24.0));
    volume_slider(session, ui, slider, a);
    x = slider.right() + 18.0;
    let time = format!("{} / {}", format_time(state.position), format_time(state.duration));
    ui_kit::draw_text(ui, pos2(x, y), Align2::LEFT_CENTER, &time, theme::semibold(size::BODY_MD), with_alpha(Color32::WHITE, 0.92 * a));

    // Right side, from the edge inwards.
    let mut rx = bar_right + 12.0;
    let right_button = |ui: &mut Ui, rx: &mut f32, icon: char, tip: &str| -> egui::Response {
        let r = Rect::from_center_size(pos2(*rx - 24.0, y), vec2(48.0, 48.0));
        let resp = ui.interact(r, egui::Id::unique(("player-btn", tip)), Sense::click());
        let h = animate(ui, resp.id.with("h"), resp.hovered(), theme::motion::FAST);
        ui.painter().circle_filled(r.center(), 22.0, with_alpha(Color32::WHITE, 0.14 * h * a));
        ui_kit::draw_icon(ui, r.center(), icon, 26.0, with_alpha(lerp_color(Color32::from_gray(225), Color32::WHITE, h), a));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        *rx -= 48.0;
        resp.on_hover_text(tip)
    };

    let fullscreen_icon = if fullscreen { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN };
    if right_button(ui, &mut rx, fullscreen_icon, if fullscreen { "Leave full screen  (F)" } else { "Full screen  (F)" }).clicked() {
        actions.push(Action::Fullscreen(!fullscreen));
    }
    if right_button(ui, &mut rx, icons::INFO, "Playback info").clicked() {
        session.show_stats = !session.show_stats;
    }
    let aspect = right_button(ui, &mut rx, icons::ASPECT_RATIO, "Picture size");
    ui_kit::glass_popup(Popup::menu(&aspect).gap(14.0).align(egui::RectAlign::TOP), |ui| {
        ui.set_min_width(190.0);
        for (value, label) in Aspect::ALL {
            if menu_row(ui, label, session.aspect == value).clicked() {
                session.apply_aspect(value);
            }
        }
    });
    let speed = right_button(ui, &mut rx, icons::SPEED, "Speed  (Shift + , and Shift + .)");
    ui_kit::glass_popup(Popup::menu(&speed).gap(14.0).align(egui::RectAlign::TOP), |ui| {
        ui.set_min_width(150.0);
        for value in SPEEDS {
            let label = if value == 1.0 { "Normal".to_owned() } else { format!("{value}×") };
            if menu_row(ui, &label, (state.speed - value).abs() < 0.01).clicked() {
                session.player.set_speed(value);
            }
        }
    });
    let sources = right_button(ui, &mut rx, icons::LAYERS, "Choose another stream");
    if sources.clicked() {
        actions.push(Action::Back);
    }
    // The menus open above the buttons: leave room for the glass panel and the gap.
    let menu_height = (rect.height() - 230.0).clamp(160.0, 560.0);
    let audio = right_button(ui, &mut rx, icons::AUDIO_TRACK, "Audio");
    let audio_tracks: Vec<Track> = session.player.tracks_of("audio").cloned().collect();
    ui_kit::glass_popup(Popup::menu(&audio).gap(14.0).align(egui::RectAlign::TOP), |ui| {
        ui.set_min_width(260.0);
        scrolling(ui, menu_height, |ui| {
            if audio_tracks.is_empty() {
                menu_note(ui, "This stream has one audio track.");
            }
            for track in &audio_tracks {
                if menu_row(ui, &track.label(), track.selected).clicked() {
                    session.player.select_audio(track.id);
                }
            }
        });
    });
    let subtitles = right_button(ui, &mut rx, icons::SUBTITLES, "Subtitles");
    let sub_tracks: Vec<Track> = session.player.tracks_of("sub").cloned().collect();
    // Embedded and already loaded tracks are listed above; offer each online file once per language and addon.
    let online: Vec<AddonSubtitle> = session.online_subtitles.iter().filter(|s| !sub_tracks.iter().any(|t| t.external && t.title.as_deref().is_some_and(|title| title.contains(&s.addon)) && t.lang.as_deref() == Some(s.language.as_str()))).take(200).cloned().collect();
    ui_kit::glass_popup(Popup::menu(&subtitles).gap(14.0).align(egui::RectAlign::TOP), |ui| {
        ui.set_min_width(280.0);
        scrolling(ui, menu_height, |ui| {
            if menu_row(ui, "Off", sub_tracks.iter().all(|t| !t.selected)).clicked() {
                session.player.select_subtitle(None);
            }
            if sub_tracks.is_empty() {
                menu_note(ui, "No subtitles in this stream.");
            }
            for track in &sub_tracks {
                if menu_row(ui, &track.label(), track.selected).clicked() {
                    session.player.select_subtitle(Some(track.id));
                }
            }
            if !online.is_empty() {
                ui.add_space(6.0);
                menu_note(ui, "From your addons");
                for subtitle in &online {
                    let label = format!("{} · {}", crate::player::language_label(&subtitle.language), subtitle.addon);
                    if menu_row(ui, &label, false).clicked() {
                        actions.push(Action::AddSubtitle { url: subtitle.url.clone(), title: label, language: subtitle.language.clone() });
                    }
                }
            }
        });
    });
    let _ = t;
}

/// A menu body that scrolls when it is taller than the window: an addon can offer a subtitle in forty languages.
fn scrolling<R>(ui: &mut Ui, max_height: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::ScrollArea::vertical().max_height(max_height).auto_shrink([true, true]).show(ui, add_contents).inner
}

fn menu_row(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let t = theme::get();
    let width = ui.available_width().max(180.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
    let hover = animate(ui, response.id.with("h"), response.hovered(), theme::motion::FAST);
    ui.painter().rect_filled(rect, radius_of(9.0), with_alpha(Color32::WHITE, 0.09 * hover));
    let colour = if selected { t.accent } else { t.text };
    let text = ui_kit::galley(ui, label, theme::semibold(size::BODY_SM), colour, width - 52.0, 1);
    ui.painter().galley(pos2(rect.left() + 14.0, rect.center().y - text.size().y / 2.0), text, colour);
    if selected {
        ui_kit::draw_icon(ui, pos2(rect.right() - 20.0, rect.center().y), icons::CHECK, 20.0, t.accent);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

fn menu_note(ui: &mut Ui, text: &str) {
    let t = theme::get();
    ui.label(egui::RichText::new(text).color(t.text_muted).size(size::BODY_SM));
}

fn timeline(session: &mut Session, ui: &mut Ui, left: f32, right: f32, y: f32, alpha: f32) {
    let t = theme::get();
    let state = session.player.state.clone();
    let hit = Rect::from_min_max(pos2(left, y - 14.0), pos2(right, y + 14.0));
    let response = ui.interact(hit, egui::Id::unique("timeline"), Sense::click_and_drag());
    let hover = animate(ui, response.id.with("hover"), response.hovered() || response.dragged(), theme::motion::FAST);
    let thickness = 4.0 + 4.0 * hover;
    let track = Rect::from_min_max(pos2(left, y - thickness / 2.0), pos2(right, y + thickness / 2.0));
    ui.painter().rect_filled(track, radius_of(thickness / 2.0), with_alpha(Color32::WHITE, 0.28 * alpha));
    let duration = state.duration.max(0.001);
    // Buffered range.
    let buffered = ((state.position + state.cached_secs) / duration).clamp(0.0, 1.0) as f32;
    ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(left + (right - left) * buffered, track.bottom())), radius_of(thickness / 2.0), with_alpha(Color32::WHITE, 0.30 * alpha));
    let shown = session.drag.unwrap_or(state.position);
    let played = (shown / duration).clamp(0.0, 1.0) as f32;
    let played_x = left + (right - left) * played;
    ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(played_x, track.bottom())), radius_of(thickness / 2.0), with_alpha(t.accent, alpha));
    if hover > 0.01 {
        ui.painter().circle_filled(pos2(played_x, y), 7.0 * hover, with_alpha(Color32::WHITE, alpha));
    }
    let to_seconds = |x: f32| (((x - left) / (right - left)).clamp(0.0, 1.0) as f64) * duration;
    if response.dragged() || response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            session.drag = Some(to_seconds(pos.x));
        }
    }
    if response.drag_stopped() {
        if let Some(target) = session.drag.take() {
            session.player.seek_to(target);
        }
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            session.player.seek_to(to_seconds(pos.x));
        }
    }
    // Time under the pointer.
    if let Some(pos) = response.hover_pos().filter(|_| hover > 0.1) {
        let label = format_time(to_seconds(pos.x));
        let g = ui_kit::galley(ui, &label, theme::semibold(size::LABEL_SM), Color32::WHITE, 100.0, 1);
        let bubble = Rect::from_center_size(pos2(pos.x.clamp(left + 30.0, right - 30.0), y - 44.0), g.size() + vec2(24.0, 14.0));
        ui_kit::glass_panel(ui, bubble, bubble.height() / 2.0, hover.min(1.0));
        ui.painter().galley(bubble.center() - g.size() / 2.0, g, Color32::WHITE);
    }
}

fn volume_slider(session: &mut Session, ui: &mut Ui, rect: Rect, alpha: f32) {
    let t = theme::get();
    let (muted, volume_now) = (session.player.state.muted, session.player.state.volume);
    let response = ui.interact(rect, egui::Id::unique("volume-slider"), Sense::click_and_drag());
    let hover = animate(ui, response.id.with("hover"), response.hovered() || response.dragged(), theme::motion::FAST);
    let value = if muted { 0.0 } else { (volume_now / 100.0).clamp(0.0, 1.3) as f32 };
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0 + 2.0 * hover));
    ui.painter().rect_filled(track, radius_of(3.0), with_alpha(Color32::WHITE, 0.28 * alpha));
    let filled_x = track.left() + track.width() * (value / 1.3).min(1.0);
    ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(filled_x, track.bottom())), radius_of(3.0), with_alpha(Color32::WHITE, alpha));
    ui.painter().circle_filled(pos2(filled_x, rect.center().y), 5.5 + 1.5 * hover, with_alpha(t.accent, alpha));
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let fraction = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
            let volume = (fraction * 130.0).round();
            session.player.set_volume(volume);
            if volume > 0.0 && session.player.state.muted {
                session.player.set_muted(false);
            }
            session.last_volume = volume;
        }
    }
    response.on_hover_text(format!("Volume {volume_now:.0}%"));
}

fn hud(session: &mut Session, ui: &mut Ui, rect: Rect) {
    let Some((hud, at)) = &session.hud else { return };
    let age = at.elapsed();
    if age > HUD_SHOW {
        session.hud = None;
        return;
    }
    crate::services::animate_on(ui.ctx());
    let fade = 1.0 - ((age.as_secs_f32() - (HUD_SHOW.as_secs_f32() - 0.3)) / 0.3).clamp(0.0, 1.0);
    let enter = (age.as_secs_f32() / 0.12).min(1.0);
    let alpha = fade * enter;
    let center = pos2(rect.center().x, rect.top() + 96.0 - 8.0 * (1.0 - enter));
    let (icon, text, level) = match hud {
        Hud::Volume(volume, muted) => {
            let icon = if *muted || *volume < 0.5 { icons::VOLUME_OFF } else if *volume < 40.0 { icons::VOLUME_DOWN } else { icons::VOLUME_UP };
            (icon, if *muted { "Muted".to_owned() } else { format!("{volume:.0}%") }, Some((*volume / 130.0).clamp(0.0, 1.0) as f32))
        }
        Hud::Seek(text) => (icons::FORWARD_10, text.clone(), None),
        Hud::Speed(speed) => (icons::SPEED, format!("{speed}×"), None),
    };
    let width = if level.is_some() { 260.0 } else if text.chars().count() > 8 { 240.0 } else { 150.0 };
    let pill = Rect::from_center_size(center, vec2(width, 52.0));
    ui_kit::glass_panel(ui, pill, 26.0, alpha);
    ui_kit::draw_icon(ui, pos2(pill.left() + 30.0, pill.center().y), icon, 26.0, with_alpha(Color32::WHITE, alpha));
    if let Some(level) = level {
        let bar = Rect::from_min_size(pos2(pill.left() + 60.0, pill.center().y - 3.0), vec2(width - 140.0, 6.0));
        ui.painter().rect_filled(bar, radius_of(3.0), Color32::from_white_alpha((60.0 * alpha) as u8));
        ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * level, 6.0)), radius_of(3.0), with_alpha(theme::get().accent, alpha));
        ui_kit::draw_text(ui, pos2(pill.right() - 20.0, pill.center().y), Align2::RIGHT_CENTER, &text, theme::semibold(size::BODY_MD), with_alpha(Color32::WHITE, alpha));
    } else {
        ui_kit::draw_text(ui, pos2(pill.left() + 58.0, pill.center().y), Align2::LEFT_CENTER, &text, theme::semibold(size::BODY_LG), with_alpha(Color32::WHITE, alpha));
    }
}

fn stats(session: &mut Session, ui: &mut Ui, rect: Rect) {
    let mut lines = session.player.stats();
    lines.push(("Buffer", format!("{:.1} s ahead", session.player.state.cached_secs)));
    if let Some(torrent) = &session.torrent {
        let s = torrent.status.snapshot();
        lines.push(("Torrent", format!("{} peers, {:.1} MB/s", s.peers, s.download_bytes_per_sec / 1_048_576.0)));
    }
    let t = theme::get();
    let panel = Rect::from_min_size(rect.right_top() + vec2(-390.0, 92.0), vec2(366.0, 28.0 + lines.len() as f32 * 24.0));
    ui_kit::glass_panel(ui, panel, 22.0, 1.0);
    for (i, (label, value)) in lines.iter().enumerate() {
        let y = panel.top() + 24.0 + i as f32 * 24.0;
        ui_kit::draw_text(ui, pos2(panel.left() + 16.0, y), Align2::LEFT_CENTER, label, theme::semibold(size::LABEL_SM), t.text_muted);
        ui_kit::draw_text(ui, pos2(panel.left() + 96.0, y), Align2::LEFT_CENTER, value, theme::regular(size::LABEL_SM + 0.5), t.text);
    }
    ui.ctx().request_repaint_after(Duration::from_millis(500));
}

fn error_card(ui: &mut Ui, rect: Rect, error: &str, actions: &mut Vec<Action>) {
    let t = theme::get();
    let card = Rect::from_center_size(rect.center(), vec2(480.0, 190.0));
    ui_kit::glass_panel(ui, card, 24.0, 1.0);
    ui_kit::draw_text(ui, card.center_top() + vec2(0.0, 40.0), Align2::CENTER_CENTER, "This stream could not be played", theme::bold(size::TITLE_SM), t.text);
    let g = ui_kit::galley(ui, error, theme::regular(size::BODY_SM), t.text_muted, 420.0, 3);
    ui.painter().galley(pos2(card.center().x - g.size().x / 2.0, card.top() + 66.0), g, t.text_muted);
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(card.left() + 110.0, card.bottom() - 62.0), vec2(280.0, 44.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if ui_kit::pill_button(&mut row, Some(icons::LAYERS), "Choose another stream", ui_kit::PillStyle::Primary).clicked() {
        actions.push(Action::Back);
    }
}

/// While the credits roll (or once the episode has ended): a card offering the next episode, which starts by itself after a
/// few seconds.
fn next_episode_card(session: &mut Session, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>, auto_play_next: bool) {
    let Some(next) = session.next_episode().filter(|_| session.credits_now()) else {
        session.ended_at = None;
        return;
    };
    let started = *session.ended_at.get_or_insert_with(Instant::now);
    let t = theme::get();
    let card = Rect::from_min_size(pos2(rect.right() - 440.0, rect.bottom() - 260.0), vec2(412.0, 120.0));
    ui_kit::glass_panel(ui, card, 22.0, 1.0);
    ui_kit::draw_text(ui, card.left_top() + vec2(22.0, 26.0), Align2::LEFT_CENTER, "Up next", theme::semibold(size::LABEL_SM), t.text_muted);
    let title = format!("{} · {}", next.code().unwrap_or_default(), next.title);
    let g = ui_kit::galley(ui, &title, theme::bold(size::BODY_LG), t.text, 370.0, 1);
    ui.painter().galley(card.left_top() + vec2(22.0, 42.0), g, t.text);
    let elapsed = started.elapsed().as_secs_f32();
    let counting = auto_play_next && !session.autoplay_cancelled;
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(card.left_top() + vec2(18.0, 68.0), vec2(380.0, 40.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let label = if counting { format!("Play now  ({:.0})", (AUTOPLAY_SECS - elapsed).ceil().max(0.0)) } else { "Play now".to_owned() };
    let play = ui_kit::pill_button(&mut row, Some(icons::PLAY), &label, ui_kit::PillStyle::Primary).clicked();
    if counting && ui_kit::pill_button(&mut row, None, "Cancel", ui_kit::PillStyle::Soft).clicked() {
        session.autoplay_cancelled = true;
    }
    if counting {
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
    if play || (counting && elapsed >= AUTOPLAY_SECS) {
        session.ended_at = None;
        actions.push(Action::PlayEpisode(next));
    }
}

/// While an intro, a recap or (with no episode to go on to) the credits are playing: a button that skips them, with the
/// seconds left before it presses itself. The cross leaves that stretch alone.
fn skip_prompt(session: &mut Session, ui: &mut Ui, rect: Rect, auto_skip: bool, credits_card: bool) {
    let Some((kind, end)) = session.segment_now(credits_card) else {
        session.skip_timer = None;
        return;
    };
    let since = match session.skip_timer {
        Some((current, at)) if current == kind => at,
        _ => {
            let now = Instant::now();
            session.skip_timer = Some((kind, now));
            now
        }
    };
    let remaining = (SKIP_SECS - since.elapsed().as_secs_f32()).max(0.0);
    let t = theme::get();
    let text = ui_kit::galley(ui, kind.label(), theme::semibold(size::BODY_MD), Color32::WHITE, 260.0, 1);
    let ring = if auto_skip { 34.0 } else { 0.0 };
    let width = 20.0 + 26.0 + 8.0 + text.size().x + if auto_skip { 12.0 + ring } else { 0.0 } + 10.0 + 32.0 + 10.0;
    let pill = Rect::from_min_size(pos2(rect.right() - 28.0 - width, rect.bottom() - 156.0 - 52.0), vec2(width, 52.0));
    ui_kit::glass_panel(ui, pill, 26.0, 1.0);
    let response = ui.interact(pill, egui::Id::unique("skip-segment"), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().rect_filled(pill, radius_of(26.0), with_alpha(Color32::WHITE, 0.10 * hover));
    let y = pill.center().y;
    let mut x = pill.left() + 20.0;
    ui_kit::draw_icon(ui, pos2(x + 13.0, y), icons::SKIP_NEXT, 26.0, Color32::WHITE);
    x += 26.0 + 8.0;
    let label_width = text.size().x;
    ui.painter().galley(pos2(x, y - text.size().y / 2.0), text, Color32::WHITE);
    x += label_width + 12.0;
    if auto_skip {
        // The seconds left, as a ring that empties.
        let centre = pos2(x + ring / 2.0, y);
        ui.painter().circle_stroke(centre, 13.0, egui::Stroke::new(2.5, with_alpha(Color32::WHITE, 0.25)));
        let sweep = std::f32::consts::TAU * (remaining / SKIP_SECS);
        let points: Vec<egui::Pos2> = (0..=24)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + sweep * i as f32 / 24.0;
                centre + vec2(angle.cos(), angle.sin()) * 13.0
            })
            .collect();
        ui.painter().add(egui::Shape::line(points, egui::Stroke::new(2.5, t.accent)));
        ui_kit::draw_text(ui, centre, Align2::CENTER_CENTER, &format!("{:.0}", remaining.ceil()), theme::bold(size::LABEL_SM), Color32::WHITE);
    }
    let cross = Rect::from_center_size(pos2(pill.right() - 10.0 - 16.0, y), vec2(32.0, 32.0));
    let cross_response = ui.interact(cross, egui::Id::unique("skip-segment-cancel"), Sense::click());
    let cross_hover = animate(ui, cross_response.id.with("hover"), cross_response.hovered(), theme::motion::FAST);
    ui.painter().circle_filled(cross.center(), 16.0, with_alpha(Color32::WHITE, 0.14 * cross_hover));
    ui_kit::draw_icon(ui, cross.center(), icons::CLOSE, 20.0, with_alpha(Color32::WHITE, 0.75 + 0.25 * cross_hover));
    if response.hovered() || cross_response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if cross_response.clicked() {
        session.skipped.push(kind);
        session.skip_timer = None;
    } else if response.clicked() || (auto_skip && remaining <= 0.0) {
        let duration = session.player.state.duration;
        session.player.seek_to(if duration > 0.0 { end.min(duration) } else { end });
        session.skipped.push(kind);
        session.skip_timer = None;
        session.show_hud(Hud::Seek(kind.done().to_owned()));
    } else if auto_skip {
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
}
