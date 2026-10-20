//! One module per screen. A screen draws itself from `State` and reports what the user did as `Action`s, which the app
//! applies after the frame, so no screen needs to know how navigation or playback is wired.

pub mod details;
pub mod home;
pub mod library;
pub mod nav;
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
    /// Opens the stream list of a title (a film) or episode.
    OpenStreams { video: Option<Video> },
    Play(Box<PlayRequest>),
    ToggleLibrary(MetaPreview),
    Toast(String),
    AddAddon(String),
    ImportKotlinAddons,
    Fullscreen(bool),
    OpenExternal(String),
    /// Continue an episode or film from where it stopped (the id is the video id in the progress book).
    Resume(String),
}

/// What a screen is given to draw with.
pub struct Cx<'a> {
    pub state: &'a mut State,
    pub svc: &'a Services,
    pub actions: &'a mut Vec<Action>,
    pub view: &'a mut ViewState,
}

impl Cx<'_> {
    pub fn push(&mut self, action: Action) {
        self.actions.push(action);
    }
}

/// UI-only state that has to survive between frames: the hero's position, text being typed, open pickers.
pub struct ViewState {
    pub hero_index: usize,
    pub hero_prev: usize,
    pub hero_since: Instant,
    pub add_addon_text: String,
    pub add_addon_error: Option<String>,
    pub settings_page: settings::Page,
    pub library_filter: library::Filter,
    pub open_facet: Option<nuvio_core::stream::Facet>,
    pub fullscreen: bool,
    pub search_focused: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            hero_index: 0,
            hero_prev: 0,
            hero_since: Instant::now(),
            add_addon_text: String::new(),
            add_addon_error: None,
            settings_page: settings::Page::Addons,
            library_filter: library::Filter::All,
            open_facet: None,
            fullscreen: false,
            search_focused: false,
        }
    }
}
