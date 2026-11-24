//! One module per screen. A screen draws itself from `State` and reports what the user did as `Action`s, which the app
//! applies after the frame, so no screen needs to know how navigation or playback is wired.

pub mod context;
pub mod details;
pub mod home;
pub mod library;
pub mod nav;
pub mod opening;
pub mod player_view;
pub mod search;
pub mod settings;
pub mod streams;

use std::time::Instant;

use nuvio_core::meta::{MetaPreview, Video};
use nuvio_core::stream::Stream;

use crate::services::Services;
use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Search,
    Library,
    Settings,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Tab(Tab),
    Details,
    Streams,
    Player,
}

/// What the user chose to play.
#[derive(Clone)]
pub struct PlayRequest {
    pub kind: String,
    pub title_id: String,
    pub video_id: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub video: Option<Video>,
    pub stream: Stream,
}

pub enum Action {
    Tab(Tab),
    Back,
    OpenDetails { kind: String, id: String, preview: Option<MetaPreview> },
    /// The page of a title, on one of its seasons.
    OpenSeason { kind: String, id: String, preview: Option<MetaPreview>, season: i64 },
    /// Takes a title off Continue Watching.
    RemoveFromContinue { kind: String, id: String },
    /// Plays another episode of the title that is playing, from inside the player: same source, same choices.
    PlayEpisode(Video),
    /// Opens the stream list of a title (a film) or episode.
    OpenStreams { video: Option<Video> },
    Play(Box<PlayRequest>),
    ToggleLibrary(MetaPreview),
    AddAddon(String),
    ImportKotlinAddons,
    Fullscreen(bool),
    OpenExternal(String),
    /// Load a subtitle file from an addon into the player and switch to it.
    AddSubtitle { url: String, title: String, language: String },
    /// Continue an episode or film from where it stopped, or open the episode after the one that was finished.
    Resume(Box<nuvio_core::store::ContinueItem>),
    /// The account page of Settings.
    OpenAccount,
    /// Sign in with an email address and password, or make the account (`create`).
    SignIn { email: String, password: String, create: bool },
    SignOut,
}

/// What a screen is given to draw with.
pub struct Cx<'a> {
    pub state: &'a mut State,
    pub svc: &'a Services,
    pub actions: &'a mut Vec<Action>,
    pub view: &'a mut ViewState,
}

/// UI-only state that has to survive between frames: the hero's position, text being typed, open pickers.
pub struct ViewState {
    pub hero_index: usize,
    pub hero_prev: usize,
    pub hero_since: Instant,
    pub add_addon_text: String,
    pub add_addon_error: Option<String>,
    pub account_email: String,
    pub account_password: String,
    /// The account form is making an account instead of signing in.
    pub account_create: bool,
    pub settings_page: settings::Page,
    pub library_filter: library::Filter,
    pub fullscreen: bool,
    pub search_focused: bool,
    /// Streams already warmed up, so resting on one card twice asks once.
    pub warmed: std::collections::HashSet<String>,
    pub hovered_stream: Option<(usize, Instant)>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            hero_index: 0,
            hero_prev: 0,
            hero_since: Instant::now(),
            add_addon_text: String::new(),
            add_addon_error: None,
            account_email: String::new(),
            account_password: String::new(),
            account_create: false,
            settings_page: settings::Page::Addons,
            library_filter: library::Filter::All,
            fullscreen: false,
            search_focused: false,
            warmed: Default::default(),
            hovered_stream: None,
        }
    }
}
