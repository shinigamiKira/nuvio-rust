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
use nuvio_core::store::{ContinueItem, NextEpisode, Progress, WindowState, unix_seconds};
use nuvio_core::stream::{Playback, Stream};
use tokio::runtime::Handle;

use crate::args::Args;
use crate::compositor::Compositor;
use crate::images::Images;
use crate::player::{Player, Source, SubtitleFile};
use crate::screens::player_view::{self, Carry, Session, SubtitlePick, Torrent};
use crate::screens::{self, Action, Cx, PlayRequest, Route, Tab, ViewState, nav, opening};
use crate::services::{Msg, Services};
use crate::state::{RefreshDue, State, StreamsState};
use crate::theme;
use crate::ui_kit::{self, glass_panel};

const PROGRESS_SAVE_EVERY: Duration = Duration::from_secs(10);
const TOAST_FOR: Duration = Duration::from_millis(2800);

pub struct App {
    compositor: Compositor,
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
    carried: Option<Carried>,
    opened_from_args: bool,
    keys_fired: Vec<usize>,
    logged_first_frame: bool,
    logged_first_content: bool,
    perf: bool,
    frames: u32,
    frame_nr_before: u64,
    causes: std::collections::HashMap<String, u32>,
    frames_since: Instant,
    /// Seconds between the starts of consecutive passes, and how long each pass took, for `NUVIO_PERF`.
    gaps: Vec<f32>,
    work: Vec<f32>,
    last_pass: Option<Instant>,
}

/// After "next episode" (or "previous"): the player screen stays up, showing the title as it does while a stream opens, until
/// an addon lists the stream that matches the one just watched. That one is played with what the viewer had chosen.
struct AutoPick {
    request: u64,
    binge_group: Option<String>,
    addon_id: String,
    resolution: Option<String>,
    carry: Carry,
    meta: Option<Arc<MetaDetails>>,
    /// What the opening screen shows.
    name: String,
    artwork: Option<String>,
    logo: Option<String>,
    since: Instant,
}

/// What a stream that starts after another brings with it, see `App::play_episode`.
struct Carried {
    carry: Carry,
    meta: Option<Arc<MetaDetails>>,
    logo: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args, runtime: Handle) -> Self {
        if let Some(gl) = cc.gl.as_ref() {
            use eframe::glow::HasContext as _;
            // SAFETY: plain string queries on the window's current GL context.
            let (renderer, version) = unsafe { (gl.get_parameter_string(eframe::glow::RENDERER), gl.get_parameter_string(eframe::glow::VERSION)) };
            tracing::info!(renderer, version, "graphics");
        }
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
        crate::subtitles::cache().trim(crate::subtitles::MAX_CACHE_BYTES);
        let images = Images::new(cc.egui_ctx.clone(), runtime.clone(), http, image_cache);
        let (tx, rx) = channel();
        let svc = Services::new(runtime, client, images, cc.egui_ctx.clone(), tx);

        let mut app = Self {
            compositor: Compositor::start(),
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
            carried: None,
            opened_from_args: false,
            keys_fired: Vec::new(),
            logged_first_frame: false,
            logged_first_content: false,
            perf: std::env::var_os("NUVIO_PERF").is_some(),
            frames: 0,
            frame_nr_before: 0,
            causes: Default::default(),
            frames_since: Instant::now(),
            gaps: Vec::new(),
            work: Vec::new(),
            last_pass: None,
        };
        app.state.start_addons(&app.svc);
        if let Some(url) = app.args.play.clone() {
            app.play_test_url(&cc.egui_ctx, url);
        }
        app.svc.post(Msg::KotlinAddons(nuvio_core::import::kotlin_addon_urls()));
        app
    }

    /// Timing lines for the startup budget: first frame, and first poster shelf with content.
    fn log_milestones(&mut self, ctx: &egui::Context) {
        // `NUVIO_PERF=1` logs how many frames were drawn each second, to find a screen that keeps repainting.
        if self.perf {
            self.frames += 1;
            for cause in ctx.repaint_causes() {
                *self.causes.entry(cause.to_string()).or_insert(0u32) += 1;
            }
            if self.frames_since.elapsed() >= Duration::from_secs(1) {
                let painted = ctx.cumulative_frame_nr() - self.frame_nr_before;
                self.frame_nr_before = ctx.cumulative_frame_nr();
                tracing::info!(passes = self.frames, painted, "frames in the last second");
                let (mut gaps, mut work) = (std::mem::take(&mut self.gaps), std::mem::take(&mut self.work));
                if gaps.len() > 2 {
                    // How many gaps fall in each band, in ms: a smooth animation sits in one band.
                    let band = |lo: f32, hi: f32| gaps.iter().filter(|g| **g * 1000.0 >= lo && **g * 1000.0 < hi).count();
                    tracing::info!(under8 = band(0.0, 8.0), to14 = band(8.0, 14.0), to20 = band(14.0, 20.0), to26 = band(20.0, 26.0), to40 = band(26.0, 40.0), to100 = band(40.0, 100.0), "gap bands (ms)");
                    gaps.sort_by(|a, b| a.total_cmp(b));
                    work.sort_by(|a, b| a.total_cmp(b));
                    let pick = |v: &Vec<f32>, q: f32| v[((v.len() - 1) as f32 * q) as usize] * 1000.0;
                    tracing::info!(
                        gap_median_ms = pick(&gaps, 0.5),
                        gap_p95_ms = pick(&gaps, 0.95),
                        gap_max_ms = pick(&gaps, 1.0),
                        work_median_ms = pick(&work, 0.5),
                        work_max_ms = pick(&work, 1.0),
                        "pass spacing and cost"
                    );
                }
                // What asked for them: the call sites that requested a repaint, most frequent first.
                let mut causes: Vec<_> = self.causes.drain().collect();
                causes.sort_by_key(|c| std::cmp::Reverse(c.1));
                for (cause, count) in causes.into_iter().take(3) {
                    tracing::info!(count, %cause, "repaint requested by");
                }
                self.frames = 0;
                self.frames_since = Instant::now();
            }
        }
        let since_start = || crate::START.get().map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
        if !self.logged_first_frame {
            self.logged_first_frame = true;
            tracing::info!(ms = since_start(), "first frame");
        }
        if !self.logged_first_content && self.state.sections.iter().any(|s| !s.items.is_empty()) {
            self.logged_first_content = true;
            tracing::info!(ms = since_start(), "first shelf with titles");
        }
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Msg::Torrent { request, result } => {
                    self.torrent_ready(request, result);
                    continue;
                }
                Msg::Subtitles { request, entries } => {
                    self.subtitles_arrived(request, entries);
                    continue;
                }
                Msg::SubtitleFile { request, title, language, select, result } => {
                    self.subtitle_downloaded(request, title, language, select, result);
                    continue;
                }
                Msg::Segments { request, segments } => {
                    if let Some(session) = self.session.as_mut().filter(|s| s.segments_request == request) {
                        session.segments = segments;
                    }
                    continue;
                }
                _ => {}
            }
            let meta_for_streams = match &message {
                Msg::Meta { result: Ok(meta), .. } => Some(meta.clone()),
                _ => None,
            };
            let was_signed_in = self.state.account.session.is_some();
            self.state.apply(message, &self.svc);
            match (was_signed_in, self.state.account.session.as_ref()) {
                (false, Some(session)) => {
                    let who = session.email.clone().unwrap_or_else(|| "your account".into());
                    self.view.account_email.clear();
                    self.view.account_password.clear();
                    self.push_toast(format!("Signed in as {who}"));
                }
                (true, None) => self.push_toast("Your session has ended. Sign in again."),
                _ => {}
            }
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
                    self.auto_pick = None;
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
                Action::OpenSeason { kind, id, preview, season } => {
                    self.state.open_details(&self.svc, &kind, &id, preview);
                    if let Some(details) = self.state.details.as_mut() {
                        details.season = Some(season);
                    }
                    self.routes.push(Route::Details);
                }
                Action::RemoveFromContinue { kind, id } => {
                    self.state.progress.hide_title(&kind, &id);
                    self.state.save_progress();
                    self.push_toast("Removed from Continue Watching");
                }
                Action::PlayEpisode(video) => self.play_episode(video),
                Action::OpenStreams { video } => self.open_streams(video),
                Action::Resume(item) => self.resume(*item),
                Action::Play(request) => self.play(ctx, *request),
                Action::ToggleLibrary(item) => {
                    let name = item.name.clone();
                    let added = self.state.library.toggle(item);
                    self.state.save_library();
                    self.push_toast(if added { format!("{name} added to your library") } else { format!("{name} removed from your library") });
                }
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
                Action::AddSubtitle { url, title, language } => {
                    if let Some(request) = self.session.as_ref().map(|s| s.subtitles_request) {
                        self.load_subtitle(request, url, title, language, true);
                    }
                }
                Action::OpenAccount => {
                    self.stop_session();
                    self.auto_pick = None;
                    self.routes = vec![Route::Tab(Tab::Settings)];
                    self.view.settings_page = screens::settings::Page::Account;
                }
                Action::SignIn { email, password, create } => self.state.sign_in(&self.svc, email, password, create),
                Action::SignOut => {
                    self.state.sign_out(&self.svc);
                    self.push_toast("Signed out");
                }
            }
        }
    }

    /// Keys that work on every screen except while a text field has the keyboard (and the player, which has its own).
    fn global_shortcuts(&mut self, ctx: &egui::Context, route: &Route) {
        if matches!(route, Route::Player) || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let (f11, search, back) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F11),
                (i.modifiers.command && i.key_pressed(egui::Key::K)) || i.key_pressed(egui::Key::Slash),
                i.key_pressed(egui::Key::Backspace) || (i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft)),
            )
        });
        if f11 {
            self.actions.push(Action::Fullscreen(!self.view.fullscreen));
        }
        if search && !matches!(route, Route::Tab(Tab::Search)) {
            self.actions.push(Action::Tab(Tab::Search));
        }
        if back && !matches!(route, Route::Tab(_)) {
            self.actions.push(Action::Back);
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
        if matches!(self.current(), Route::Player) {
            self.stop_session();
            self.routes.pop();
        }
        self.auto_pick = None;
        let Some(details) = self.state.details.as_ref() else { return };
        let meta = details.meta.ready().cloned();
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
        };
        self.state.open_streams(&self.svc, state);
        if !matches!(self.current(), Route::Streams) {
            self.routes.push(Route::Streams);
        }
    }

    /// "Next episode" and "Previous episode" in the player, and the card that offers the next one: the other episode opens
    /// without leaving the player. Its stream is the one that matches what was playing (the same binge group, else the same
    /// addon at the same resolution), the volume, audio track, subtitles, speed and picture size are the ones the viewer had,
    /// and the screen in between is the one shown while any stream opens. Only when no stream matches does the list open.
    fn play_episode(&mut self, video: nuvio_core::meta::Video) {
        let Some(session) = self.session.as_ref() else { return };
        let carry = session.carry_out();
        let previous = session.request.stream.clone();
        let request = &session.request;
        let state = StreamsState {
            kind: request.kind.clone(),
            title_id: request.title_id.clone(),
            video_id: video.id.clone(),
            name: request.name.clone(),
            episode_label: video.code(),
            episode_title: Some(video.title.clone()).filter(|t| !t.is_empty()),
            poster: request.poster.clone(),
            background: request.background.clone(),
            meta: session.meta.clone(),
            request: 0,
            groups: Vec::new(),
            selection: Default::default(),
        };
        let (meta, logo) = (session.meta.clone(), session.logo.clone());
        let (name, artwork) = (request.name.clone(), request.background.clone().or_else(|| request.poster.clone()));
        // Saves how far the episode that is being left got.
        self.stop_session();
        self.state.open_streams(&self.svc, state);
        let request = self.state.streams.as_ref().map(|s| s.request).unwrap_or(0);
        self.auto_pick = Some(AutoPick {
            request,
            binge_group: previous.binge_group.clone(),
            addon_id: previous.addon_id.clone(),
            resolution: previous.facet_values().get(&nuvio_core::stream::Facet::Resolution).and_then(|v| v.first().cloned()),
            carry,
            meta,
            name,
            artwork,
            logo,
            since: Instant::now(),
        });
    }

    /// Continue Watching: open the stream list of the saved episode, or of the one after it.
    fn resume(&mut self, item: ContinueItem) {
        let progress = item.progress.clone();
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
            video_id: item.video_id().to_owned(),
            name: progress.name.clone(),
            episode_label: item.season().zip(item.episode()).map(|(s, e)| format!("S{s} E{e}")),
            episode_title: item.episode_title().map(str::to_owned),
            poster: progress.poster.clone(),
            background: progress.background.clone(),
            meta: None,
            request: 0,
            groups: Vec::new(),
            selection: Default::default(),
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
            Some(Playback::Torrent { info_hash, file_idx, trackers }) => {
                self.start_torrent(ctx, request.clone(), info_hash, file_idx, trackers);
            }
            Some(Playback::External(url)) => {
                open_in_browser(&url);
                self.push_toast("Opened in your browser");
            }
            None => self.push_toast("This stream has nothing to play"),
        }
    }

    /// A player with the settings applied. `carried` is what the episode before it left behind (volume, speed, the languages
    /// of the audio and the subtitles), which wins over the settings.
    fn make_player(&mut self, ctx: &egui::Context, carried: Option<&Carried>) -> Option<Player> {
        let settings = &self.state.settings;
        let mut player = match Player::new(ctx, settings.hardware_decoding, crate::mpv_custom::Custom::load(settings)) {
            Ok(player) => player,
            Err(error) => {
                self.push_toast(format!("The player could not start: {error}"));
                return None;
            }
        };
        let settings = &self.state.settings;
        player.set_volume(carried.map_or(settings.volume, |c| c.carry.volume));
        let (audio, subtitles, subtitles_on) = match carried {
            Some(c) => c.carry.track_preferences(settings),
            None => (settings.preferred_audio_language.clone(), settings.preferred_subtitle_language.clone(), settings.subtitles_on),
        };
        player.set_track_preferences(&audio, &subtitles, subtitles_on);
        if let Some(c) = carried {
            player.set_muted(c.carry.muted);
            player.set_speed(c.carry.speed);
        }
        Some(player)
    }

    /// Asks IntroDB where the intro and the credits of an episode are. The answer arrives as `Msg::Segments`; a title that is
    /// not a series with an IMDb id, or an episode nobody has marked, gets none.
    fn fetch_segments(&mut self, request: &PlayRequest) -> u64 {
        let id = self.state.request_id();
        let mut parts = request.video_id.split(':');
        let imdb = parts.next().filter(|id| id.starts_with("tt") && id.len() > 2 && id.bytes().skip(2).all(|b| b.is_ascii_digit())).map(str::to_owned);
        let season = request.video.as_ref().and_then(|v| v.season).or_else(|| parts.next().and_then(|s| s.parse().ok()));
        let episode = request.video.as_ref().and_then(|v| v.episode).or_else(|| parts.next().and_then(|e| e.parse().ok()));
        if let (Some(imdb), Some(season), Some(episode)) = (imdb, season, episode) {
            let http = self.svc.client.http().clone();
            self.svc.spawn(async move {
                let segments = nuvio_core::segments::fetch(&http, &imdb, season, episode).await.unwrap_or_default();
                Msg::Segments { request: id, segments }
            });
        }
        id
    }

    fn start_player(&mut self, ctx: &egui::Context, request: PlayRequest, url: String) {
        let carried = self.carried.take();
        let Some(mut player) = self.make_player(ctx, carried.as_ref()) else { return };
        let start = self.state.progress.get(&request.video_id).filter(|p| p.is_resumable()).map(|p| p.position).unwrap_or(0.0);
        player.open(Source { url, start: if self.args.start > 0.0 { self.args.start } else { start }, headers: request.stream.request_headers.clone() });
        let meta = self.state.details.as_ref().and_then(|d| d.meta.ready().cloned()).or_else(|| carried.as_ref().and_then(|c| c.meta.clone())).filter(|m| m.id == request.title_id);
        let subtitles_request = self.fetch_subtitles(&request);
        let segments_request = self.fetch_segments(&request);
        let mut session = Session::new(player, request, meta);
        session.subtitles_request = subtitles_request;
        session.segments_request = segments_request;
        session.logo = session.logo.take().or_else(|| carried.as_ref().and_then(|c| c.logo.clone())).or_else(|| self.preview_logo());
        if let Some(carried) = carried {
            session.take_carry(carried.carry);
        }
        self.session = Some(session);
        crate::platform::playback(true);
        self.auto_pick = None;
        if !matches!(self.current(), Route::Player) {
            self.routes.push(Route::Player);
        }
    }

    /// The wordmark of the title being opened, from the catalog entry it was picked from, while its details load.
    fn preview_logo(&self) -> Option<String> {
        self.state.details.as_ref().and_then(|d| d.preview.as_ref()).and_then(|p| p.logo.clone())
    }

    /// A torrent: the player opens now (its video surface has to exist before a file loads), and the file follows when
    /// the swarm has been found and the first pieces chosen.
    fn start_torrent(&mut self, ctx: &egui::Context, request: PlayRequest, info_hash: String, file_idx: Option<i64>, trackers: Vec<String>) {
        let carried = self.carried.take();
        let Some(player) = self.make_player(ctx, carried.as_ref()) else { return };
        let meta = self.state.details.as_ref().and_then(|d| d.meta.ready().cloned()).or_else(|| carried.as_ref().and_then(|c| c.meta.clone())).filter(|m| m.id == request.title_id);
        let torrent_request = self.state.request_id();
        let status = crate::p2p::Status::new();
        let wanted = crate::p2p::Request {
            info_hash: info_hash.clone(),
            trackers,
            file_index: file_idx.and_then(|i| usize::try_from(i).ok()),
            season: request.video.as_ref().and_then(|v| v.season),
            episode: request.video.as_ref().and_then(|v| v.episode),
        };
        let subtitles_request = self.fetch_subtitles(&request);
        let segments_request = self.fetch_segments(&request);
        let mut session = Session::new(player, request, meta);
        session.subtitles_request = subtitles_request;
        session.segments_request = segments_request;
        session.logo = session.logo.take().or_else(|| carried.as_ref().and_then(|c| c.logo.clone())).or_else(|| self.preview_logo());
        if let Some(carried) = carried {
            session.take_carry(carried.carry);
        }
        session.torrent = Some(Torrent { request: torrent_request, status: status.clone(), info_hash });
        self.session = Some(session);
        crate::platform::playback(true);
        self.auto_pick = None;
        if !matches!(self.current(), Route::Player) {
            self.routes.push(Route::Player);
        }
        let svc = self.svc.clone();
        self.svc.spawn(async move {
            let result = match svc.engine().await {
                Ok(engine) => engine.open(&wanted, &status).await.map_err(|e| format!("{e:#}")),
                Err(error) => Err(error),
            };
            Msg::Torrent { request: torrent_request, result }
        });
    }

    /// Asks the addons that serve subtitles for what they have for this video. Answers arrive as `Msg::Subtitles`.
    fn fetch_subtitles(&mut self, request: &PlayRequest) -> u64 {
        let id = self.state.request_id();
        for source in self.state.subtitle_sources(&request.kind, &request.video_id) {
            let (client, kind, video_id, filename) = (self.svc.client.clone(), request.kind.clone(), request.video_id.clone(), request.stream.filename.clone());
            self.svc.spawn(async move {
                let entries = client.subtitles(&source, &kind, &video_id, filename.as_deref()).await.unwrap_or_default();
                Msg::Subtitles { request: id, entries }
            });
        }
        // Subtitles the stream itself came with.
        for subtitle in &request.stream.subtitles {
            let title = match crate::player::language_label(&subtitle.language) {
                label if label.is_empty() => "Subtitles".to_owned(),
                label => label,
            };
            self.load_subtitle(id, subtitle.url.clone(), title, subtitle.language.clone(), false);
        }
        id
    }

    /// Downloads a subtitle file in the background. It reaches the player as `Msg::SubtitleFile`.
    fn load_subtitle(&self, request: u64, url: String, title: String, language: String, select: bool) {
        let client = self.svc.client.clone();
        self.svc.spawn(async move {
            let result = crate::subtitles::download(&client, &url).await;
            Msg::SubtitleFile { request, title, language, select, result }
        });
    }

    fn subtitle_downloaded(&mut self, request: u64, title: String, language: String, select: bool, result: Result<String, String>) {
        let Some(session) = self.session.as_mut().filter(|s| s.subtitles_request == request) else { return };
        match result {
            Ok(path) => session.player.add_subtitle(SubtitleFile { path, title, language, select }),
            Err(error) => {
                tracing::warn!(%error, "could not load a subtitle file");
                if select {
                    self.push_toast("That subtitle could not be loaded");
                }
            }
        }
    }

    fn subtitles_arrived(&mut self, request: u64, entries: Vec<nuvio_core::stream::AddonSubtitle>) {
        let mut preferred = self.state.settings.preferred_subtitle_language.clone();
        let mut wanted = self.state.settings.subtitles_on;
        let Some(session) = self.session.as_mut().filter(|s| s.subtitles_request == request) else { return };
        // After another episode, the subtitles that were on there (or off) decide, and the addon they came from is preferred.
        let mut preferred_addon = None;
        match session.carry.as_ref().map(|c| &c.subtitles) {
            Some(SubtitlePick::On { language, addon, .. }) => {
                wanted = true;
                if let Some(language) = language {
                    preferred = language.clone();
                }
                preferred_addon = addon.clone();
            }
            Some(SubtitlePick::Off) => wanted = false,
            _ => {}
        }
        session.online_subtitles.extend(entries);
        // With subtitles switched on by default and none in the file in that language, take the first one an addon has.
        let has_selected = session.player.tracks_of("sub").any(|t| t.selected);
        let mut chosen = None;
        if wanted && !has_selected && !session.subtitle_chosen && !preferred.is_empty() {
            let wanted_name = crate::player::language_label(&preferred);
            let matching = |s: &&nuvio_core::stream::AddonSubtitle| crate::player::language_label(&s.language) == wanted_name;
            let from_addon = preferred_addon.as_ref().and_then(|addon| session.online_subtitles.iter().filter(matching).find(|s| &s.addon == addon));
            if let Some(subtitle) = from_addon.or_else(|| session.online_subtitles.iter().find(matching)).cloned() {
                session.subtitle_chosen = true;
                chosen = Some((subtitle.url, format!("{} · {}", wanted_name, subtitle.addon), subtitle.language));
            }
        }
        if let Some((url, label, language)) = chosen {
            self.load_subtitle(request, url, label, language, true);
        }
    }

    /// The loopback URL of a torrent arrived: hand it to the player.
    fn torrent_ready(&mut self, request: u64, result: Result<String, String>) {
        let start_override = self.args.start;
        let Some(session) = self.session.as_mut().filter(|s| s.torrent.as_ref().is_some_and(|t| t.request == request)) else {
            return;
        };
        match result {
            Ok(url) => {
                let resume = self.state.progress.get(&session.request.video_id).filter(|p| p.is_resumable()).map(|p| p.position).unwrap_or(0.0);
                session.player.open(Source { url, start: if start_override > 0.0 { start_override } else { resume }, headers: Vec::new() });
            }
            Err(error) => session.player.fail(error),
        }
    }

    /// Records how far the viewer got and releases the player.
    fn stop_session(&mut self) {
        self.save_progress(true);
        crate::platform::playback(false);
        if let Some(session) = self.session.take() {
            if let Some(torrent) = &session.torrent {
                torrent.status.stop();
                let (svc, hash) = (self.svc.clone(), torrent.info_hash.clone());
                self.svc.runtime.spawn(async move {
                    if let Some(engine) = svc.engine_if_started() {
                        engine.close(&hash).await;
                    }
                });
            }
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
        let next = session.next_episode().map(|v| NextEpisode { video_id: v.id.clone(), season: v.season, episode: v.episode, title: Some(v.title.clone()).filter(|t| !t.is_empty()) });
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
            next,
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
                if let Some(pick) = self.auto_pick.take() {
                    self.carried = Some(Carried { carry: pick.carry, meta: pick.meta, logo: pick.logo });
                }
                self.play(ctx, request);
                self.carried = None;
            }
            // Every addon has answered and none has the stream that was playing: the list is for the viewer to choose from,
            // never a stream picked at random.
            None if finished => {
                self.auto_pick = None;
                if matches!(self.current(), Route::Player) && self.session.is_none() {
                    self.routes.pop();
                    if !matches!(self.current(), Route::Streams) {
                        self.routes.push(Route::Streams);
                    }
                    self.push_toast("The stream you were watching is not available for this episode. Pick another.");
                }
            }
            None => {}
        }
    }

    /// `--play <url>`: open the player on a bare URL with no title behind it. `NUVIO_TEST_TITLE`, `NUVIO_TEST_BACKGROUND` and
    /// `NUVIO_TEST_LOGO` give it a name, artwork and a wordmark, to look at the screen shown while a stream opens, and
    /// `NUVIO_TEST_SUBTITLES=<n>` makes n addon subtitles appear in the Subtitles menu.
    fn play_test_url(&mut self, ctx: &egui::Context, url: String) {
        let name = std::env::var("NUVIO_TEST_TITLE").unwrap_or_else(|_| url.rsplit(['/', '\\']).next().unwrap_or("Video").split('?').next().unwrap_or("Video").to_owned());
        let request = PlayRequest {
            kind: "movie".into(),
            title_id: "test".into(),
            video_id: "test".into(),
            name,
            poster: None,
            background: std::env::var("NUVIO_TEST_BACKGROUND").ok(),
            video: None,
            stream: Stream { url: Some(url.clone()), ..Stream::default() },
        };
        self.play(ctx, request);
        if let Some(session) = &mut self.session {
            session.logo = std::env::var("NUVIO_TEST_LOGO").ok();
            let count = std::env::var("NUVIO_TEST_SUBTITLES").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
            const LANGUAGES: [&str; 8] = ["eng", "spa", "fre", "ger", "ita", "por", "jpn", "kor"];
            session.online_subtitles = (0..count)
                .map(|i| nuvio_core::stream::AddonSubtitle { addon: format!("Test addon {}", i / LANGUAGES.len() + 1), language: LANGUAGES[i % LANGUAGES.len()].into(), url: format!("http://127.0.0.1:9/{i}.srt") })
                .collect();
        }
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
            (Some("search"), query, _) => {
                self.routes = vec![Route::Tab(Tab::Search)];
                if let Some(query) = query {
                    self.state.search.query = query.to_owned();
                    self.state.run_search(&self.svc);
                }
            }
            (Some("library"), ..) => self.routes = vec![Route::Tab(Tab::Library)],
            (Some("settings"), page, _) => {
                self.routes = vec![Route::Tab(Tab::Settings)];
                self.view.settings_page = match page {
                    Some("playback") => screens::settings::Page::Playback,
                    Some("appearance") => screens::settings::Page::Appearance,
                    Some("storage") => screens::settings::Page::Storage,
                    Some("about") => screens::settings::Page::About,
                    Some("account") => screens::settings::Page::Account,
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

    /// Keeps the signed-in session fresh: renews it shortly before its access token runs out, and wakes the window for that
    /// moment and no other.
    fn drive_account(&mut self, ctx: &egui::Context) {
        match self.state.account.refresh_due() {
            RefreshDue::Now => self.state.refresh_session(&self.svc),
            RefreshDue::In(wait) => ctx.request_repaint_after(wait.min(Duration::from_secs(3600))),
            RefreshDue::Never => {}
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
        crate::services::animate_on(ctx);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Each frame starts right after the compositor's last pass; see `compositor.rs`.
        self.compositor.wait_for_pass();
        let ctx = ui.ctx().clone();
        let began = Instant::now();
        if self.perf {
            if let Some(last) = self.last_pass.replace(began) {
                self.gaps.push((began - last).as_secs_f32());
            }
        }
        self.log_milestones(&ctx);
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
                player_view::show(session, ui, &self.svc.images, &mut self.actions, self.view.fullscreen, self.state.settings.auto_play_next, self.state.settings.auto_skip_segments);
            } else if let Some(pick) = &self.auto_pick {
                // Between two episodes: the title, as while any stream opens, until the stream that matches has answered.
                let rect = ui.max_rect();
                let content = opening::Opening { artwork: pick.artwork.as_deref(), logo: pick.logo.as_deref(), title: &pick.name, message: Some("Finding the same stream for the next episode…") };
                if opening::show(ui, rect, &self.svc.images, &content, pick.since.elapsed().as_secs_f32(), 1.0) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.actions.push(Action::Back);
                }
            }
        } else if let Route::Tab(tab) = route {
            if let Some(clicked) = nav::show(&ctx, tab) {
                self.actions.push(Action::Tab(clicked));
            }
            if nav::account_button(&ctx, self.state.account.session.as_ref().map(|s| s.email.as_deref().unwrap_or("?"))) {
                self.actions.push(Action::OpenAccount);
            }
            // Escape and the mouse's back button leave a page, never a root tab.
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.button_pressed(egui::PointerButton::Extra1)) && !ctx.memory(|m| m.focused().is_some()) {
            self.actions.push(Action::Back);
        }

        if route == Route::Tab(Tab::Home) && ctx.input(|i| i.key_pressed(egui::Key::F5) || (i.modifiers.command && i.key_pressed(egui::Key::R))) {
            self.state.refresh_home(&self.svc);
            self.push_toast("Refreshing Home…");
        }
        self.global_shortcuts(&ctx, &route);
        self.run_auto_pick(&ctx);
        self.save_progress(false);
        self.drive_account(&ctx);
        self.drive_tests(&ctx);
        self.track_window(&ctx);
        self.toasts_ui(&ctx);
        self.apply_actions(&ctx);
        if self.perf {
            self.work.push(began.elapsed().as_secs_f32());
        }
    }

    /// `--mouse x,y`: a pointer that keeps moving by a pixel, so hover states and the player's controls show up in
    /// screenshots taken without anyone at the mouse.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some((x, y)) = self.args.mouse {
            let nudge = (ctx.cumulative_pass_nr() % 2) as f32;
            raw_input.events.push(egui::Event::PointerMoved(Pos2::new(x + nudge, y)));
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        for (index, (at, name)) in self.args.keys.iter().enumerate() {
            if elapsed >= *at && !self.keys_fired.contains(&index) {
                self.keys_fired.push(index);
                if let Some(key) = egui::Key::from_name(name) {
                    for pressed in [true, false] {
                        raw_input.events.push(egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: egui::Modifiers::NONE });
                    }
                }
            }
        }
        for (index, (at, x, y)) in self.args.clicks.iter().enumerate() {
            let slot = 1000 + index;
            if elapsed >= *at && !self.keys_fired.contains(&slot) {
                self.keys_fired.push(slot);
                let pos = Pos2::new(*x, *y);
                raw_input.events.push(egui::Event::PointerMoved(pos));
                for pressed in [true, false] {
                    raw_input.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                }
            }
        }
        for (index, (at, text)) in self.args.texts.iter().enumerate() {
            let slot = 2000 + index;
            if elapsed >= *at && !self.keys_fired.contains(&slot) {
                self.keys_fired.push(slot);
                raw_input.events.push(egui::Event::Text(text.clone()));
            }
        }
        if !self.args.keys.is_empty() || !self.args.clicks.is_empty() || !self.args.texts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_session();
        // Let the torrent engine save its DHT table; a few seconds is all it gets.
        if let Some(engine) = self.svc.engine_if_started() {
            let _ = self.svc.runtime.block_on(async { tokio::time::timeout(Duration::from_secs(3), engine.shutdown()).await });
        }
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
