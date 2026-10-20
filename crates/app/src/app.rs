//! The application: owns the state, draws the current screen, and carries out what the screens asked for.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Pos2, vec2};
use nuvio_core::cache::DiskCache;
use nuvio_core::http::Http;
use nuvio_core::client::AddonClient;
use nuvio_core::meta::{MetaDetails, MetaPreview};
use nuvio_core::paths;
use nuvio_core::store::{Progress, WindowState, unix_seconds};
use nuvio_core::stream::{Playback, Stream};
use tokio::runtime::Handle;

use crate::args::Args;
use crate::images::Images;
use crate::player::{Player, Source};
use crate::screens::player_view::{self, Session};
use crate::screens::{self, Action, Cx, PlayRequest, Route, Tab, ViewState, nav};
use crate::services::{Msg, Services};
use crate::state::{State, StreamsState};
use crate::theme;
use crate::ui_kit::{self, glass_panel};

const PROGRESS_SAVE_EVERY: Duration = Duration::from_secs(10);
const TOAST_FOR: Duration = Duration::from_millis(2800);

pub struct App {
    args: Args,
    state: State,
    svc: Services,
    rx: Receiver<Msg>,
    routes: Vec<Route>,
    view: ViewState,
    actions: Vec<Action>,
    session: Option<Session>,
    toasts: Vec<(String, Instant)>,
    started: Instant,
    screenshot_requested: bool,
    window: Option<WindowState>,
    auto_pick: Option<AutoPick>,
    opened_from_args: bool,
}

/// After "next episode": play the stream that matches the one just watched as soon as the addon lists it.
struct AutoPick {
    request: u64,
    binge_group: Option<String>,
    addon_id: String,
    resolution: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args, runtime: Handle) -> Self {
        let state = State::load();
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &theme::Theme::new(theme::palette_by_id(&state.settings.theme), state.settings.amoled));
        if let Some(scale) = args.scale {
            cc.egui_ctx.set_pixels_per_point(scale);
        }

        let http = Http::new().expect("the HTTP client could not be created");
        let client = AddonClient::new(http.clone(), DiskCache::new(paths::cache_dir().join("json")));
        let image_cache = DiskCache::new(paths::cache_dir().join("images"));
        image_cache.trim(400 * 1024 * 1024);
        let images = Images::new(cc.egui_ctx.clone(), runtime.clone(), http, image_cache);
        let (tx, rx) = channel();
        let svc = Services::new(runtime, client, images, cc.egui_ctx.clone(), tx);

        let mut app = Self {
            args,
            state,
            svc,
            rx,
            routes: vec![Route::Tab(Tab::Home)],
            view: ViewState::default(),
            actions: Vec::new(),
            session: None,
            toasts: Vec::new(),
            started: Instant::now(),
            screenshot_requested: false,
            window: None,
            auto_pick: None,
            opened_from_args: false,
        };
        app.state.start_addons(&app.svc);
        if let Some(url) = app.args.play.clone() {
            app.play_test_url(&cc.egui_ctx, url);
        }
        app.svc.post(Msg::KotlinAddons(nuvio_core::import::kotlin_addon_urls()));
        app
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            let meta_for_streams = match &message {
                Msg::Meta { result: Ok(meta), .. } => Some(meta.clone()),
                _ => None,
            };
            self.state.apply(message, &self.svc);
            if let Some(meta) = meta_for_streams {
                self.fill_streams_from_meta(&meta);
            }
        }
    }

    /// A stream list opened from a deep link or Continue Watching has no title yet; the meta fills it in.
    fn fill_streams_from_meta(&mut self, meta: &MetaDetails) {
        let Some(streams) = self.state.streams.as_mut() else { return };
        if streams.title_id != meta.id {
            return;
        }
        let meta = Arc::new(meta.clone());
        let video = meta.video(&streams.video_id).cloned();
        streams.name = meta.name.clone();
        streams.poster = meta.poster.clone().or(streams.poster.take());
        streams.background = meta.background.clone().or(streams.background.take());
        streams.episode_label = video.as_ref().and_then(|v| v.code());
        streams.episode_title = video.map(|v| v.title);
        streams.meta = Some(meta);
    }

    // -- navigation --------------------------------------------------------------------------------------------------

    fn current(&self) -> &Route {
        self.routes.last().unwrap_or(&Route::Tab(Tab::Home))
    }

    fn push_toast(&mut self, text: impl Into<String>) {
        self.toasts.push((text.into(), Instant::now()));
    }

    fn apply_actions(&mut self, ctx: &egui::Context) {
        let actions = std::mem::take(&mut self.actions);
        for action in actions {
            match action {
                Action::Tab(tab) => {
                    self.stop_session();
                    self.routes = vec![Route::Tab(tab)];
                    if tab == Tab::Search {
                        self.view.search_focused = false;
                    }
                }
                Action::Back => self.go_back(ctx),
                Action::OpenDetails { kind, id, preview } => {
                    self.state.open_details(&self.svc, &kind, &id, preview);
                    self.routes.push(Route::Details);
                }
                Action::OpenStreams { video } => self.open_streams(video),
                Action::Resume(video_id) => self.resume(&video_id),
                Action::Play(request) => self.play(ctx, *request),
                Action::ToggleLibrary(item) => {
                    let name = item.name.clone();
                    let added = self.state.library.toggle(item);
                    self.state.save_library();
                    self.push_toast(if added { format!("{name} added to your library") } else { format!("{name} removed from your library") });
                }
                Action::Toast(text) => self.push_toast(text),
                Action::AddAddon(url) => {
                    if self.state.add_addon(&self.svc, url) {
                        self.push_toast("Installing the addon…");
                    } else {
                        self.push_toast("That addon is already installed");
                    }
                }
                Action::ImportKotlinAddons => {
                    let urls = self.state.kotlin_addons.clone();
                    let added = urls.into_iter().filter(|url| self.state.add_addon(&self.svc, url.clone())).count();
                    self.push_toast(format!("Imported {added} addon{}", if added == 1 { "" } else { "s" }));
                }
                Action::Fullscreen(on) => {
                    self.view.fullscreen = on;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
                }
                Action::OpenExternal(url) => open_in_browser(&url),
            }
        }
    }

    fn go_back(&mut self, ctx: &egui::Context) {
        if matches!(self.current(), Route::Player) {
            self.stop_session();
            if self.view.fullscreen {
                self.view.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        }
        if self.routes.len() > 1 {
            self.routes.pop();
        }
        self.auto_pick = None;
    }

    fn open_streams(&mut self, video: Option<nuvio_core::meta::Video>) {
        // Opening the next episode from inside the player replaces the player.
        let remembered = self.session.as_ref().map(|s| (s.request.stream.clone(), s.meta.clone()));
        if matches!(self.current(), Route::Player) {
            self.stop_session();
            self.routes.pop();
        }
        let Some(details) = self.state.details.as_ref() else { return };
        let meta = details.meta.ready().cloned().or_else(|| remembered.as_ref().and_then(|(_, m)| m.clone()));
        let preview = details.preview.clone();
        let (name, poster, background) = match (&meta, &preview) {
            (Some(m), _) => (m.name.clone(), m.poster.clone(), m.background.clone()),
            (None, Some(p)) => (p.name.clone(), p.poster.clone(), p.background.clone()),
            _ => (details.id.clone(), None, None),
        };
        let state = StreamsState {
            kind: details.kind.clone(),
            title_id: details.id.clone(),
            video_id: video.as_ref().map(|v| v.id.clone()).unwrap_or_else(|| details.id.clone()),
            name,
            episode_label: video.as_ref().and_then(|v| v.code()),
            episode_title: video.as_ref().map(|v| v.title.clone()),
            poster,
            background,
            meta,
            request: 0,
            groups: Vec::new(),
            selection: Default::default(),
            started: Instant::now(),
        };
        self.state.open_streams(&self.svc, state);
        let request = self.state.streams.as_ref().map(|s| s.request).unwrap_or(0);
        self.auto_pick = remembered.filter(|_| self.state.settings.auto_play_next).map(|(stream, _)| AutoPick {
            request,
            binge_group: stream.binge_group.clone(),
            addon_id: stream.addon_id.clone(),
            resolution: stream.facet_values().get(&nuvio_core::stream::Facet::Resolution).and_then(|v| v.first().cloned()),
        });
        if !matches!(self.current(), Route::Streams) {
            self.routes.push(Route::Streams);
        }
    }

    /// Continue Watching: open the stream list of the saved episode.
    fn resume(&mut self, video_id: &str) {
        let Some(progress) = self.state.progress.get(video_id).cloned() else { return };
        let preview = MetaPreview {
            id: progress.id.clone(),
            kind: progress.kind.clone(),
            name: progress.name.clone(),
            poster: progress.poster.clone(),
            background: progress.background.clone(),
            ..MetaPreview::default()
        };
        self.state.open_details(&self.svc, &progress.kind, &progress.id, Some(preview));
        let state = StreamsState {
            kind: progress.kind.clone(),
            title_id: progress.id.clone(),
            video_id: progress.video_id.clone(),
            name: progress.name.clone(),
            episode_label: progress.season.zip(progress.episode).map(|(s, e)| format!("S{s} E{e}")),
            episode_title: progress.episode_title.clone(),
            poster: progress.poster.clone(),
            background: progress.background.clone(),
            meta: None,
            request: 0,
            groups: Vec::new(),
            selection: Default::default(),
            started: Instant::now(),
        };
        self.state.open_streams(&self.svc, state);
        self.routes.push(Route::Streams);
    }

    fn play(&mut self, ctx: &egui::Context, request: PlayRequest) {
        let stream = &request.stream;
        match stream.playback() {
            Some(Playback::Url(url)) => {
                self.start_player(ctx, request.clone(), url);
            }
            Some(Playback::Torrent { .. }) => {
                self.push_toast("This is a torrent stream. The built-in P2P engine is not part of this build yet.");
            }
            Some(Playback::External(url)) => {
                open_in_browser(&url);
                self.push_toast("Opened in your browser");
            }
            None => self.push_toast("This stream has nothing to play"),
        }
    }

    fn start_player(&mut self, ctx: &egui::Context, request: PlayRequest, url: String) {
        let settings = &self.state.settings;
        let mut player = match Player::new(ctx, settings.hardware_decoding) {
            Ok(player) => player,
            Err(error) => {
                self.push_toast(format!("The player could not start: {error}"));
                return;
            }
        };
        player.set_volume(settings.volume);
        player.set_track_preferences(&settings.preferred_audio_language, &settings.preferred_subtitle_language, settings.subtitles_on);
        let start = self.state.progress.get(&request.video_id).filter(|p| p.is_resumable()).map(|p| p.position).unwrap_or(0.0);
        player.open(Source {
            url,
            start: if self.args.start > 0.0 { self.args.start } else { start },
            headers: request.stream.request_headers.clone(),
            subtitles: request.stream.subtitles.iter().map(|s| s.url.clone()).collect(),
        });
        let meta = self.state.details.as_ref().and_then(|d| d.meta.ready().cloned()).filter(|m| m.id == request.title_id);
        self.session = Some(Session::new(player, request, meta));
        self.auto_pick = None;
        if !matches!(self.current(), Route::Player) {
            self.routes.push(Route::Player);
        }
    }

    /// Records how far the viewer got and releases the player.
    fn stop_session(&mut self) {
        self.save_progress(true);
        if let Some(session) = self.session.take() {
            self.state.settings.volume = session.player.state.volume;
            self.state.save_settings();
            session.player.stop();
        }
    }

    fn save_progress(&mut self, force: bool) {
        let Some(session) = &mut self.session else { return };
        if !force && session.last_saved.elapsed() < PROGRESS_SAVE_EVERY {
            return;
        }
        let state = &session.player.state;
        if !state.started || state.duration <= 0.0 {
            return;
        }
        session.last_saved = Instant::now();
        let request = &session.request;
        let position = if state.ended { state.duration } else { state.position };
        self.state.progress.record(Progress {
            kind: request.kind.clone(),
            id: request.title_id.clone(),
            video_id: request.video_id.clone(),
            season: request.video.as_ref().and_then(|v| v.season),
            episode: request.video.as_ref().and_then(|v| v.episode),
            episode_title: request.video.as_ref().map(|v| v.title.clone()),
            name: request.name.clone(),
            poster: request.poster.clone(),
            background: request.background.clone(),
            position,
            duration: state.duration,
            updated_at: unix_seconds(),
        });
        self.state.save_progress();
    }

    /// Plays the stream that matches the episode before it, as soon as its addon has answered.
    fn run_auto_pick(&mut self, ctx: &egui::Context) {
        let Some(pick) = &self.auto_pick else { return };
        let Some(streams) = self.state.streams.as_ref().filter(|s| s.request == pick.request) else { return };
        let candidates: Vec<&Stream> = streams.all_streams().into_iter().filter(|s| s.is_playable()).collect();
        let by_group = pick.binge_group.as_ref().and_then(|group| candidates.iter().find(|s| s.binge_group.as_ref() == Some(group)));
        let same_addon_and_quality = candidates.iter().find(|s| {
            s.addon_id == pick.addon_id
                && pick.resolution.as_ref().is_some_and(|r| s.facet_values().get(&nuvio_core::stream::Facet::Resolution).is_some_and(|v| v.contains(r)))
        });
        let chosen = by_group.or(same_addon_and_quality).map(|s| (*s).clone());
        let finished = streams.pending() == 0;
        match chosen {
            Some(stream) => {
                let video = streams.meta.as_ref().and_then(|m| m.video(&streams.video_id)).cloned();
                let request = PlayRequest {
                    kind: streams.kind.clone(),
                    title_id: streams.title_id.clone(),
                    video_id: streams.video_id.clone(),
                    name: streams.name.clone(),
                    poster: streams.poster.clone(),
                    background: streams.background.clone(),
                    video,
                    stream,
                };
                self.auto_pick = None;
                self.play(ctx, request);
            }
            None if finished => self.auto_pick = None,
            None => {}
        }
    }

    /// `--play <url>`: open the player on a bare URL with no title behind it.
    fn play_test_url(&mut self, ctx: &egui::Context, url: String) {
        let name = url.rsplit(['/', '\\']).next().unwrap_or("Video").split('?').next().unwrap_or("Video").to_owned();
        let request = PlayRequest {
            kind: "movie".into(),
            title_id: "test".into(),
            video_id: "test".into(),
            name,
            poster: None,
            background: None,
            video: None,
            stream: Stream { url: Some(url.clone()), ..Stream::default() },
        };
        self.start_player(ctx, request, url);
    }

    // -- development hooks (see args.rs) -----------------------------------------------------------------------------

    fn open_from_args(&mut self) {
        let Some(target) = self.args.open.clone() else { return };
        // Wait until every addon has answered, so detail and stream pages know who to ask.
        if self.state.addons.iter().any(|a| matches!(a.status, crate::state::Load::Loading)) {
            return;
        }
        self.opened_from_args = true;
        let mut parts = target.splitn(3, ':');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("home"), ..) => {}
            (Some("search"), ..) => self.routes = vec![Route::Tab(Tab::Search)],
            (Some("library"), ..) => self.routes = vec![Route::Tab(Tab::Library)],
            (Some("settings"), page, _) => {
                self.routes = vec![Route::Tab(Tab::Settings)];
                self.view.settings_page = match page {
                    Some("playback") => screens::settings::Page::Playback,
                    Some("appearance") => screens::settings::Page::Appearance,
                    Some("storage") => screens::settings::Page::Storage,
                    Some("about") => screens::settings::Page::About,
                    _ => screens::settings::Page::Addons,
                };
            }
            (Some("detail"), Some(kind), Some(id)) => {
                self.state.open_details(&self.svc, kind, id, None);
                self.routes.push(Route::Details);
            }
            (Some("streams"), Some(kind), Some(video_id)) => {
                let title_id = video_id.split(':').next().unwrap_or(video_id).to_owned();
                self.state.open_details(&self.svc, kind, &title_id, None);
                let state = StreamsState {
                    kind: kind.to_owned(),
                    title_id: title_id.clone(),
                    video_id: video_id.to_owned(),
                    name: title_id,
                    episode_label: None,
                    episode_title: None,
                    poster: None,
                    background: None,
                    meta: None,
                    request: 0,
                    groups: Vec::new(),
                    selection: Default::default(),
                    started: Instant::now(),
                };
                self.state.open_streams(&self.svc, state);
                self.routes.push(Route::Streams);
            }
            _ => tracing::warn!("unknown --open target"),
        }
    }

    fn drive_tests(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.args.select {
            let ready = self.state.streams.as_ref().is_some_and(|s| s.pending() == 0 || s.all_streams().len() > index);
            if ready && matches!(self.current(), Route::Streams) {
                let picked = self.state.streams.as_ref().and_then(|s| {
                    let list: Vec<&Stream> = s.all_streams().into_iter().filter(|st| st.is_playable() && st.matches(&s.selection, None)).collect();
                    list.get(index).map(|stream| (s, (*stream).clone()))
                });
                if let Some((s, stream)) = picked {
                    let request = PlayRequest {
                        kind: s.kind.clone(),
                        title_id: s.title_id.clone(),
                        video_id: s.video_id.clone(),
                        name: s.name.clone(),
                        poster: s.poster.clone(),
                        background: s.background.clone(),
                        video: s.meta.as_ref().and_then(|m| m.video(&s.video_id)).cloned(),
                        stream,
                    };
                    self.args.select = None;
                    self.play(ctx, request);
                }
            }
        }
        if let Some(path) = self.args.screenshot.clone() {
            if !self.screenshot_requested && self.started.elapsed() > Duration::from_secs_f64(self.args.screenshot_after) {
                self.screenshot_requested = true;
                let repaint = ctx.clone();
                ctx.request_screenshot(move |image| {
                    save_png(&path, &image);
                    repaint.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(seconds) = self.args.quit_after {
            if self.started.elapsed().as_secs_f64() > seconds {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn track_window(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            let viewport = i.viewport();
            if viewport.fullscreen == Some(true) {
                return;
            }
            if let (Some(outer), Some(inner)) = (viewport.outer_rect, viewport.inner_rect) {
                let maximized = viewport.maximized.unwrap_or(false);
                // A maximized window reports the screen's size; keep the size it had before instead.
                let previous = self.window.filter(|_| maximized);
                self.window = Some(WindowState {
                    x: previous.map(|w| w.x).unwrap_or(outer.min.x),
                    y: previous.map(|w| w.y).unwrap_or(outer.min.y),
                    width: previous.map(|w| w.width).unwrap_or(inner.width()),
                    height: previous.map(|w| w.height).unwrap_or(inner.height()),
                    maximized,
                });
            }
        });
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|(_, at)| at.elapsed() < TOAST_FOR);
        let Some((text, at)) = self.toasts.last() else { return };
        let age = at.elapsed().as_secs_f32();
        let total = TOAST_FOR.as_secs_f32();
        let alpha = (age / 0.18).min(1.0) * ((total - age) / 0.3).clamp(0.0, 1.0);
        let t = theme::get();
        egui::Area::new(egui::Id::unique("toast")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -34.0 + 8.0 * (1.0 - (age / 0.18).min(1.0)))).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            let galley = ui_kit::galley(ui, text, theme::semibold(theme::size::BODY_MD), t.text, 560.0, 2);
            let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(44.0, 26.0), egui::Sense::hover());
            glass_panel(ui, rect, rect.height() / 2.0, alpha);
            ui.painter().galley(rect.min + vec2(22.0, 13.0), galley, ui_kit::with_alpha(t.text, alpha));
        });
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_messages();
        self.svc.images.begin_frame();
        if !self.opened_from_args {
            self.open_from_args();
        }
        let t = theme::get();
        let screen = ctx.content_rect();
        ui.painter().rect_filled(screen, 0.0, t.background);

        let route = self.current().clone();
        {
            let mut cx = Cx { state: &mut self.state, svc: &self.svc, actions: &mut self.actions, view: &mut self.view };
            match &route {
                Route::Tab(Tab::Home) => screens::home::show(&mut cx, ui),
                Route::Tab(Tab::Search) => screens::search::show(&mut cx, ui),
                Route::Tab(Tab::Library) => screens::library::show(&mut cx, ui),
                Route::Tab(Tab::Settings) => screens::settings::show(&mut cx, ui),
                Route::Details => screens::details::show(&mut cx, ui),
                Route::Streams => screens::streams::show(&mut cx, ui),
                Route::Player => {}
            }
        }
        if route == Route::Player {
            if let Some(session) = &mut self.session {
                player_view::show(session, ui, &mut self.actions, self.view.fullscreen, self.state.settings.auto_play_next);
            }
        } else if let Route::Tab(tab) = route {
            if let Some(clicked) = nav::show(&ctx, tab) {
                self.actions.push(Action::Tab(clicked));
            }
            // Escape and the mouse's back button leave a page, never a root tab.
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.button_pressed(egui::PointerButton::Extra1)) && !ctx.memory(|m| m.focused().is_some()) {
            self.actions.push(Action::Back);
        }

        self.run_auto_pick(&ctx);
        self.save_progress(false);
        self.drive_tests(&ctx);
        self.track_window(&ctx);
        self.toasts_ui(&ctx);
        self.apply_actions(&ctx);
    }

    /// `--mouse x,y`: a pointer that keeps moving by a pixel, so hover states and the player's controls show up in
    /// screenshots taken without anyone at the mouse.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some((x, y)) = self.args.mouse {
            let nudge = (ctx.cumulative_pass_nr() % 2) as f32;
            raw_input.events.push(egui::Event::PointerMoved(Pos2::new(x + nudge, y)));
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_session();
        if let Some(window) = self.window {
            self.state.settings.window = Some(window);
        }
        self.state.save_settings();
        self.state.save_progress();
    }
}

fn open_in_browser(url: &str) {
    // `rundll32 url.dll` hands the address to the default browser without a shell, so `&` in a URL stays literal.
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
}

fn save_png(path: &std::path::Path, image: &Arc<egui::ColorImage>) {
    let [width, height] = image.size;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), 255]);
    }
    if let Some(buffer) = image::RgbaImage::from_raw(width as u32, height as u32, rgba) {
        if let Err(error) = buffer.save(path) {
            tracing::error!(%error, "could not save the screenshot");
        }
    }
}
