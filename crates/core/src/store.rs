//! What the app remembers between runs, as small JSON files under `%APPDATA%\NuvioRust`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::meta::MetaPreview;
use crate::paths;

pub fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One JSON file. A missing or unreadable file is an empty state, never an error: the app must start.
pub struct JsonFile<T> {
    path: PathBuf,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned + Default> JsonFile<T> {
    pub fn new(name: &str) -> Self {
        Self { path: paths::roaming_data_dir().join(name), _marker: std::marker::PhantomData }
    }

    pub fn load(&self) -> T {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(file = %self.path.display(), %error, "unreadable state file, starting empty");
                // Keep the broken file for inspection instead of overwriting it on the next save.
                let _ = fs::rename(&self.path, self.path.with_extension("broken"));
                T::default()
            }),
            Err(_) => T::default(),
        }
    }

    /// Written through a temporary file and a rename, so a crash cannot truncate the saved state.
    pub fn save(&self, value: &T) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let temp = self.path.with_extension("tmp");
        let result = serde_json::to_vec_pretty(value)
            .map_err(std::io::Error::other)
            .and_then(|bytes| fs::write(&temp, bytes))
            .and_then(|_| fs::rename(&temp, &self.path));
        if let Err(error) = result {
            tracing::warn!(file = %self.path.display(), %error, "could not save state");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddonEntry {
    pub manifest_url: String,
    pub enabled: bool,
    /// A name the user chose instead of the one in the manifest.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddonList {
    pub addons: Vec<AddonEntry>,
    /// Cinemeta goes in on the very first run only; removing it afterwards sticks.
    pub defaults_installed: bool,
}

impl AddonList {
    pub fn contains(&self, manifest_url: &str) -> bool {
        self.addons.iter().any(|a| a.manifest_url == manifest_url)
    }

    pub fn add(&mut self, manifest_url: String) -> bool {
        if self.contains(&manifest_url) {
            return false;
        }
        self.addons.push(AddonEntry { manifest_url, enabled: true, name: None });
        true
    }
}

pub const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";

// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub item: MetaPreview,
    pub added_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    pub items: Vec<LibraryItem>,
}

impl Library {
    pub fn contains(&self, kind: &str, id: &str) -> bool {
        self.items.iter().any(|i| i.item.kind == kind && i.item.id == id)
    }

    /// Adds the title when it is not there, removes it when it is. Returns whether it is in the library afterwards.
    pub fn toggle(&mut self, item: MetaPreview) -> bool {
        if let Some(pos) = self.items.iter().position(|i| i.item.kind == item.kind && i.item.id == item.id) {
            self.items.remove(pos);
            return false;
        }
        self.items.insert(0, LibraryItem { item, added_at: unix_seconds() });
        true
    }
}

// ---------------------------------------------------------------------------------------------------------------------

/// The episode after a finished one. It is kept with the progress, so Continue Watching can offer it without looking
/// the series up again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextEpisode {
    pub video_id: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub title: Option<String>,
}

/// How far through a title or episode the user is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub kind: String,
    pub id: String,
    pub video_id: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub episode_title: Option<String>,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub position: f64,
    pub duration: f64,
    pub updated_at: u64,
    #[serde(default)]
    pub next: Option<NextEpisode>,
}

/// A few seconds in is a start worth coming back to; less is a misclick.
const RESUME_MIN_SECS: f64 = 10.0;

impl Progress {
    pub fn fraction(&self) -> f64 {
        if self.duration > 0.0 { (self.position / self.duration).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Past 95 % (or within the credits) a title counts as watched.
    pub fn is_finished(&self) -> bool {
        self.duration > 0.0 && (self.fraction() >= 0.95 || self.duration - self.position < 90.0)
    }

    /// Worth showing under Continue Watching: started, not finished.
    pub fn is_resumable(&self) -> bool {
        self.position >= RESUME_MIN_SECS && !self.is_finished()
    }

    pub fn remaining_minutes(&self) -> i64 {
        (((self.duration - self.position).max(0.0)) / 60.0).ceil() as i64
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressBook {
    /// Keyed by video id (`tt1:2:3` for an episode, the title id for a film).
    pub entries: BTreeMap<String, Progress>,
    /// Titles taken off Continue Watching by hand (`kind|id`). Watching anything of the title again brings it back.
    #[serde(default)]
    pub hidden: BTreeSet<String>,
}

fn title_key(kind: &str, id: &str) -> String {
    format!("{kind}|{id}")
}

impl ProgressBook {
    pub fn record(&mut self, progress: Progress) {
        self.hidden.remove(&title_key(&progress.kind, &progress.id));
        self.entries.insert(progress.video_id.clone(), progress);
    }

    /// Takes a title off Continue Watching. What was watched stays marked as watched.
    pub fn hide_title(&mut self, kind: &str, id: &str) {
        self.hidden.insert(title_key(kind, id));
    }

    pub fn get(&self, video_id: &str) -> Option<&Progress> {
        self.entries.get(video_id)
    }

    pub fn remove(&mut self, video_id: &str) -> bool {
        self.entries.remove(video_id).is_some()
    }

    /// What to offer under Continue Watching, newest first, one entry per title: the episode or film that was left
    /// part way, or, when the last one was watched to the end, the episode after it.
    pub fn continue_watching(&self) -> Vec<ContinueItem> {
        let mut by_title: BTreeMap<(&str, &str), Vec<&Progress>> = BTreeMap::new();
        for entry in self.entries.values() {
            by_title.entry((entry.kind.as_str(), entry.id.as_str())).or_default().push(entry);
        }
        let mut list: Vec<ContinueItem> = Vec::new();
        for ((kind, id), mut entries) in by_title {
            if self.hidden.contains(&title_key(kind, id)) {
                continue;
            }
            entries.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
            let found = entries.iter().find_map(|p| {
                if p.is_resumable() {
                    Some(ContinueItem { progress: (*p).clone(), up_next: None })
                } else if p.is_finished() {
                    p.next.clone().filter(|next| !self.is_watched(&next.video_id)).map(|next| ContinueItem { progress: (*p).clone(), up_next: Some(next) })
                } else {
                    None
                }
            });
            list.extend(found);
        }
        list.sort_by_key(|item| std::cmp::Reverse(item.progress.updated_at));
        list
    }

    pub fn is_watched(&self, video_id: &str) -> bool {
        self.entries.get(video_id).is_some_and(Progress::is_finished)
    }
}

/// One entry of Continue Watching: the progress it comes from, and the episode to open instead when that one is done.
#[derive(Debug, Clone, PartialEq)]
pub struct ContinueItem {
    pub progress: Progress,
    pub up_next: Option<NextEpisode>,
}

impl ContinueItem {
    /// The video that opens when the card is picked.
    pub fn video_id(&self) -> &str {
        self.up_next.as_ref().map(|n| n.video_id.as_str()).unwrap_or(&self.progress.video_id)
    }

    pub fn season(&self) -> Option<i64> {
        self.up_next.as_ref().map(|n| n.season).unwrap_or(self.progress.season)
    }

    pub fn episode(&self) -> Option<i64> {
        self.up_next.as_ref().map(|n| n.episode).unwrap_or(self.progress.episode)
    }

    pub fn episode_title(&self) -> Option<&str> {
        match &self.up_next {
            Some(next) => next.title.as_deref(),
            None => self.progress.episode_title.as_deref(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------

/// What was searched for, most recent first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchHistory {
    pub entries: Vec<String>,
}

const SEARCH_HISTORY_MAX: usize = 20;

impl SearchHistory {
    /// Puts `query` at the top. The same words typed in another case move up instead of repeating.
    pub fn record(&mut self, query: &str) {
        let query = query.trim();
        if query.chars().count() < 2 {
            return;
        }
        let lower = query.to_lowercase();
        self.entries.retain(|e| e.to_lowercase() != lower);
        self.entries.insert(0, query.to_owned());
        self.entries.truncate(SEARCH_HISTORY_MAX);
    }

    pub fn remove(&mut self, query: &str) {
        self.entries.retain(|e| e != query);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub amoled: bool,
    pub hardware_decoding: bool,
    pub hero: bool,
    pub show_catalog_type: bool,
    pub auto_play_next: bool,
    /// Skip intros, recaps and end credits by themselves, a few seconds after they begin.
    pub auto_skip_segments: bool,
    pub preferred_audio_language: String,
    pub preferred_subtitle_language: String,
    pub subtitles_on: bool,
    /// `auto` follows each catalog's own shape; `portrait` and `landscape` force one.
    pub poster_style: String,
    /// The offer to import addons from an installed Nuvio for Windows (Kotlin) was answered.
    pub import_prompt_dismissed: bool,
    pub volume: f64,
    /// Load the scripts, key bindings and script options in the mpv folder into the player.
    pub mpv_scripts: bool,
    /// Let mpv draw its own on-screen messages and what the scripts draw (their overlays) over the picture.
    pub mpv_osd: bool,
    /// Extra mpv options, one `name=value` per line.
    pub mpv_options: String,
    pub window: Option<WindowState>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "white".into(),
            amoled: true,
            hardware_decoding: true,
            hero: true,
            show_catalog_type: true,
            auto_play_next: true,
            auto_skip_segments: true,
            preferred_audio_language: String::new(),
            preferred_subtitle_language: "eng".into(),
            subtitles_on: false,
            poster_style: "auto".into(),
            import_prompt_dismissed: false,
            volume: 100.0,
            mpv_scripts: false,
            mpv_osd: true,
            mpv_options: String::new(),
            window: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(video: &str, id: &str, position: f64, at: u64) -> Progress {
        Progress {
            kind: "series".into(),
            id: id.into(),
            video_id: video.into(),
            season: Some(1),
            episode: Some(1),
            episode_title: None,
            name: "Show".into(),
            poster: None,
            background: None,
            position,
            duration: 3000.0,
            updated_at: at,
            next: Some(NextEpisode { video_id: format!("{id}:1:2"), season: Some(1), episode: Some(2), title: Some("Two".into()) }),
        }
    }

    #[test]
    fn finished_means_95_percent_or_credits() {
        assert!(progress("a", "x", 2900.0, 1).is_finished());
        assert!(progress("a", "x", 2915.0, 1).is_finished());
        assert!(!progress("a", "x", 1500.0, 1).is_finished());
        assert!(!progress("a", "x", 5.0, 1).is_resumable());
        assert!(progress("a", "x", 12.0, 1).is_resumable());
    }

    #[test]
    fn continue_watching_shows_a_series_once_at_its_latest_episode() {
        let mut book = ProgressBook::default();
        book.record(progress("tt1:1:1", "tt1", 1000.0, 10));
        book.record(progress("tt1:1:2", "tt1", 500.0, 20));
        book.record(progress("tt2:1:1", "tt2", 600.0, 5));
        book.record(progress("tt3:1:1", "tt3", 2990.0, 30));
        let list = book.continue_watching();
        // tt3 was watched to the end, so what is offered is the episode after it.
        assert_eq!(list.iter().map(|p| p.video_id()).collect::<Vec<_>>(), vec!["tt3:1:2", "tt1:1:2", "tt2:1:1"]);
        assert!(list[0].up_next.is_some() && list[1].up_next.is_none());
    }

    #[test]
    fn a_barely_started_episode_does_not_hide_the_one_before_it() {
        let mut book = ProgressBook::default();
        book.record(progress("tt1:1:1", "tt1", 2990.0, 10));
        book.record(progress("tt1:1:2", "tt1", 3.0, 20));
        let list = book.continue_watching();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].video_id(), "tt1:1:2");
        assert!(list[0].up_next.is_some());
    }

    #[test]
    fn the_next_episode_is_not_offered_once_it_is_watched() {
        let mut book = ProgressBook::default();
        book.record(progress("tt1:1:1", "tt1", 2990.0, 10));
        let mut second = progress("tt1:1:2", "tt1", 2990.0, 20);
        second.next = Some(NextEpisode { video_id: "tt1:1:3".into(), season: Some(1), episode: Some(3), title: None });
        book.record(second);
        let list = book.continue_watching();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].video_id(), "tt1:1:3");
    }

    #[test]
    fn search_history_keeps_the_latest_first_without_repeats() {
        let mut history = SearchHistory::default();
        for query in ["dune", "a", "Batman", "DUNE ", "heat"] {
            history.record(query);
        }
        assert_eq!(history.entries, vec!["heat", "DUNE", "Batman"]);
        history.remove("heat");
        assert_eq!(history.entries.len(), 2);
        history.clear();
        assert!(history.entries.is_empty());
    }

    #[test]
    fn library_toggles() {
        let mut library = Library::default();
        let item = MetaPreview { id: "tt1".into(), kind: "movie".into(), name: "A".into(), ..MetaPreview::default() };
        assert!(library.toggle(item.clone()));
        assert!(library.contains("movie", "tt1"));
        assert!(!library.toggle(item));
        assert!(library.items.is_empty());
    }

    #[test]
    fn settings_fill_in_missing_fields() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"amber"}"#).unwrap();
        assert_eq!(settings.theme, "amber");
        assert!(settings.hardware_decoding && settings.amoled);
    }
}
