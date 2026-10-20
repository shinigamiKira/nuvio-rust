//! The player screen: the video fills the window and the controls float over it. They appear when the pointer moves
//! and fade out (taking the cursor with them) after a few seconds of stillness while a video plays.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CursorIcon, Key, Popup, Rect, Sense, Ui, pos2, vec2};
use nuvio_core::meta::{MetaDetails, Video};
use nuvio_core::stream::AddonSubtitle;

use super::{Action, PlayRequest};
use crate::icons;
use crate::player::{Player, Track};
use crate::theme::{self, size};
use crate::ui_kit::{self, animate, lerp_color, radius_of, with_alpha};

const HIDE_AFTER: Duration = Duration::from_millis(3500);
const HUD_SHOW: Duration = Duration::from_millis(1400);
const AUTOPLAY_SECS: f32 = 8.0;
const SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

pub enum Hud {
    Volume(f64, bool),
    Seek(String),
    Speed(f64),
}

#[derive(Clone, Copy, PartialEq)]
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
    last_motion: Instant,
    hud: Option<(Hud, Instant)>,
    drag: Option<f64>,
    aspect: Aspect,
    show_stats: bool,
    pub last_saved: Instant,
    ended_at: Option<Instant>,
    autoplay_cancelled: bool,
    last_volume: f64,
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
            meta,
            last_motion: Instant::now(),
            hud: None,
            drag: None,
            aspect: Aspect::Fit,
            show_stats: false,
            last_saved: Instant::now(),
            ended_at: None,
            autoplay_cancelled: false,
            last_volume: 100.0,
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
pub fn show(session: &mut Session, ui: &mut Ui, actions: &mut Vec<Action>, fullscreen: bool, auto_play_next: bool) {
    let ctx = ui.ctx().clone();
    let t = theme::get();
    let rect = ui.max_rect();
    session.player.pump();

    // The video, behind everything.
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    ui.painter().add(session.player.video_shape(rect));

    // --- input ------------------------------------------------------------------------------------------------------
    let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_pressed() || i.smooth_scroll_delta != egui::Vec2::ZERO);
    if moved {
        session.last_motion = Instant::now();
    }
    let video_area = ui.interact(rect, egui::Id::unique("video-area"), Sense::click());
    if video_area.double_clicked() {
        actions.push(Action::Fullscreen(!fullscreen));
    } else if video_area.clicked() {
        session.player.toggle_pause();
    }
    handle_keys(session, &ctx, actions, fullscreen);
    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
    if scroll != 0.0 && video_area.hovered() {
        session.adjust_volume(if scroll > 0.0 { 5.0 } else { -5.0 });
    }

    let state = session.player.state.clone();
    let menu_open = Popup::is_any_open(&ctx);
    let idle = session.last_motion.elapsed() > HIDE_AFTER;
    let show_controls = !idle || state.paused || menu_open || session.drag.is_some() || state.error.is_some() || !state.started;
    let visible = animate(ui, egui::Id::unique("controls-visible"), show_controls, theme::motion::NORMAL);
    if !show_controls && !state.paused {
        ctx.set_cursor_icon(CursorIcon::None);
    }
    if state.started && !state.paused && !idle {
        ctx.request_repaint_after(HIDE_AFTER.saturating_sub(session.last_motion.elapsed()) + Duration::from_millis(50));
    }

    // --- buffering ------------------------------------------------------------------------------------------------
    if (state.buffering || !state.started) && state.error.is_none() {
        let center = rect.center();
        ui.painter().circle_filled(center, 38.0, Color32::from_black_alpha(130));
        ui_kit::spinner(ui, center, 18.0, Color32::WHITE);
        let text = if let (false, Some(torrent)) = (state.started, &session.torrent) {
            torrent_text(&torrent.status.snapshot())
        } else if !state.started {
            "Opening stream…".to_owned()
        } else if state.buffer_percent > 0 {
            format!("Buffering {}%", state.buffer_percent)
        } else {
            "Buffering…".to_owned()
        };
        ui_kit::draw_text(ui, center + vec2(0.0, 62.0), Align2::CENTER_CENTER, &text, theme::semibold(size::BODY_MD), with_alpha(Color32::WHITE, 0.9));
    }

    // --- errors -----------------------------------------------------------------------------------------------------
    let torrent_error = session.torrent.as_ref().and_then(|t| match t.status.snapshot().phase {
        crate::p2p::Phase::Failed(error) => Some(error),
        _ => None,
    });
    let error = state.error.clone().or(torrent_error).or_else(|| session.player.render_error());
    if let Some(error) = error {
        error_card(ui, rect, &error, actions);
    }

    // --- controls ---------------------------------------------------------------------------------------------------
    if visible > 0.01 {
        controls(session, ui, rect, visible, actions, fullscreen);
    }
    hud(session, ui, rect);
    if session.show_stats {
        stats(session, ui, rect);
    }
    next_episode_card(session, ui, rect, actions, auto_play_next);
    let _ = &t;
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

fn handle_keys(session: &mut Session, ctx: &egui::Context, actions: &mut Vec<Action>, fullscreen: bool) {
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    let (space, k, left, right, up, down, m, f, f11, esc, shift, ctrl, alt, speed_up, speed_down, back_nav) = ctx.input(|i| {
        (
            i.key_pressed(Key::Space),
            i.key_pressed(Key::K),
            i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::J),
            i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::L),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::M),
            i.key_pressed(Key::F),
            i.key_pressed(Key::F11),
            i.key_pressed(Key::Escape),
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

    // Scrims so white controls stay legible on bright video.
    ui_kit::vertical_gradient(ui, Rect::from_min_size(rect.min, vec2(rect.width(), 140.0)), Color32::from_black_alpha((170.0 * a) as u8), Color32::from_black_alpha(0));
    ui_kit::vertical_gradient(ui, Rect::from_min_max(pos2(rect.left(), rect.bottom() - 230.0), rect.max), Color32::from_black_alpha(0), Color32::from_black_alpha((210.0 * a) as u8));

    // Top: back and title.
    let back = Rect::from_min_size(rect.min + vec2(24.0, 22.0), vec2(46.0, 46.0));
    let response = ui.interact(back, egui::Id::unique("player-back"), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().circle_filled(back.center(), 23.0, Color32::from_black_alpha(((90.0 + 70.0 * hover) * a) as u8));
    ui_kit::draw_icon(ui, back.center(), icons::ARROW_BACK, 26.0, with_alpha(Color32::WHITE, a));
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if response.on_hover_text("Back").clicked() {
        actions.push(Action::Back);
    }
    let title_x = back.right() + 18.0;
    ui_kit::draw_text(ui, pos2(title_x, back.center().y - 9.0), Align2::LEFT_CENTER, &session.request.name, theme::bold(size::TITLE_SM), with_alpha(Color32::WHITE, a));
    if let Some(label) = session.episode_label() {
        ui_kit::draw_text(ui, pos2(title_x, back.center().y + 13.0), Align2::LEFT_CENTER, &label, theme::regular(size::BODY_SM), with_alpha(Color32::WHITE, 0.75 * a));
    }

    // Centre: play / pause with the 10 second jumps, shown while paused.
    let paused_alpha = animate(ui, egui::Id::unique("center-visible"), state.paused && state.started, theme::motion::NORMAL);
    if paused_alpha > 0.01 {
        let center = rect.center();
        for (dx, icon, secs, tip) in [(-150.0, icons::REPLAY_10, -10.0, "Back 10 seconds"), (150.0, icons::FORWARD_10, 10.0, "Forward 10 seconds")] {
            let r = Rect::from_center_size(center + vec2(dx, 0.0), vec2(64.0, 64.0));
            let resp = ui.interact(r, egui::Id::unique(("center-jump", dx as i32)), Sense::click());
            ui_kit::draw_icon(ui, r.center(), icon, 44.0, with_alpha(Color32::WHITE, paused_alpha * (0.8 + 0.2 * animate(ui, resp.id.with("h"), resp.hovered(), 0.1))));
            if resp.on_hover_text(tip).clicked() {
                session.seek_by(secs);
            }
        }
        let r = Rect::from_center_size(center, vec2(86.0, 86.0));
        let resp = ui.interact(r, egui::Id::unique("center-play"), Sense::click());
        let h = animate(ui, resp.id.with("h"), resp.hovered(), 0.12);
        ui.painter().circle_filled(r.center(), 40.0 + 2.0 * h, with_alpha(Color32::BLACK, 0.45 * paused_alpha));
        ui_kit::draw_icon(ui, r.center(), icons::PLAY, 60.0, with_alpha(Color32::WHITE, paused_alpha));
        if resp.on_hover_text("Play").clicked() {
            session.player.set_paused(false);
        }
    }

    // Bottom: timeline.
    let bar_left = rect.left() + 28.0;
    let bar_right = rect.right() - 28.0;
    let bar_y = rect.bottom() - 92.0;
    timeline(session, ui, bar_left, bar_right, bar_y, a);

    // Bottom: buttons.
    let y = rect.bottom() - 48.0;
    let mut x = bar_left;
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
            actions.push(Action::OpenStreams { video: Some(video) });
        }
    }
    if has_next.is_some() && button(ui, &mut x, icons::SKIP_NEXT, "Next episode", 28.0) {
        if let Some(video) = has_next.clone() {
            actions.push(Action::OpenStreams { video: Some(video) });
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
    let mut rx = bar_right;
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
    Popup::menu(&aspect).gap(10.0).align(egui::RectAlign::TOP).show(|ui| {
        ui.set_min_width(190.0);
        for (value, label) in Aspect::ALL {
            if menu_row(ui, label, session.aspect == value).clicked() {
                session.apply_aspect(value);
            }
        }
    });
    let speed = right_button(ui, &mut rx, icons::SPEED, "Speed  (Shift + , and Shift + .)");
    Popup::menu(&speed).gap(10.0).align(egui::RectAlign::TOP).show(|ui| {
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
    let audio = right_button(ui, &mut rx, icons::AUDIO_TRACK, "Audio");
    let audio_tracks: Vec<Track> = session.player.tracks_of("audio").cloned().collect();
    Popup::menu(&audio).gap(10.0).align(egui::RectAlign::TOP).show(|ui| {
        ui.set_min_width(260.0);
        if audio_tracks.is_empty() {
            menu_note(ui, "This stream has one audio track.");
        }
        for track in &audio_tracks {
            if menu_row(ui, &track.label(), track.selected).clicked() {
                session.player.select_audio(track.id);
            }
        }
    });
    let subtitles = right_button(ui, &mut rx, icons::SUBTITLES, "Subtitles");
    let sub_tracks: Vec<Track> = session.player.tracks_of("sub").cloned().collect();
    // Embedded and already loaded tracks are listed above; offer each online file once per language and addon.
    let online: Vec<AddonSubtitle> = session.online_subtitles.iter().filter(|s| !sub_tracks.iter().any(|t| t.external && t.title.as_deref().is_some_and(|title| title.contains(&s.addon)) && t.lang.as_deref() == Some(s.language.as_str()))).take(40).cloned().collect();
    Popup::menu(&subtitles).gap(10.0).align(egui::RectAlign::TOP).show(|ui| {
        ui.set_min_width(280.0);
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
                    session.player.add_subtitle(&subtitle.url, &label, &subtitle.language);
                }
            }
        }
    });
    let _ = t;
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
        let bubble = Rect::from_center_size(pos2(pos.x.clamp(left + 30.0, right - 30.0), y - 30.0), g.size() + vec2(16.0, 10.0));
        ui.painter().rect_filled(bubble, radius_of(8.0), Color32::from_black_alpha(200));
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
    ui.ctx().request_repaint();
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
    let width = if level.is_some() { 260.0 } else { 150.0 };
    let pill = Rect::from_center_size(center, vec2(width, 52.0));
    ui.painter().rect_filled(pill, radius_of(26.0), Color32::from_black_alpha((190.0 * alpha) as u8));
    ui.painter().rect_stroke(pill, radius_of(26.0), egui::Stroke::new(1.0, Color32::from_white_alpha((36.0 * alpha) as u8)), egui::StrokeKind::Inside);
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
    let p = &session.player;
    let get = |name: &str| p.property(name).unwrap_or_else(|| "–".into());
    let lines = [
        ("Video", format!("{} {}×{} @ {} fps", get("video-codec"), get("video-params/w"), get("video-params/h"), get("container-fps"))),
        ("Decoder", get("hwdec-current")),
        ("Colour", format!("{} / {}", get("video-params/colormatrix"), get("video-params/gamma"))),
        ("Audio", format!("{} {}", get("audio-codec-name"), get("audio-params/hr-channels"))),
        ("Buffer", format!("{:.1} s ahead, {:.1} MB/s", p.state.cached_secs, p.property("cache-speed").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) / 1_048_576.0)),
        ("Frames", format!("{} dropped, {} delayed", get("frame-drop-count"), get("vo-delayed-frame-count"))),
    ];
    let torrent_line = session.torrent.as_ref().map(|t| {
        let s = t.status.snapshot();
        ("Torrent", format!("{} peers, {:.1} MB/s", s.peers, s.download_bytes_per_sec / 1_048_576.0))
    });
    let lines: Vec<(&str, String)> = lines.into_iter().chain(torrent_line).collect();
    let t = theme::get();
    let panel = Rect::from_min_size(rect.right_top() + vec2(-390.0, 92.0), vec2(366.0, 28.0 + lines.len() as f32 * 24.0));
    ui.painter().rect_filled(panel, radius_of(14.0), Color32::from_black_alpha(205));
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

/// When an episode ends: a card offering the next one, which starts by itself after a few seconds.
fn next_episode_card(session: &mut Session, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>, auto_play_next: bool) {
    if !session.player.state.ended {
        session.ended_at = None;
        return;
    }
    let Some(next) = session.next_episode() else { return };
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
        actions.push(Action::OpenStreams { video: Some(next) });
    }
}
