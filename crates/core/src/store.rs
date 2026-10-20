//! What the app remembers between runs, as small JSON files under `%APPDATA%\NuvioRust`.

use std::collections::BTreeMap;
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
}

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
        self.position >= 30.0 && !self.is_finished()
    }

    pub fn remaining_minutes(&self) -> i64 {
        (((self.duration - self.position).max(0.0)) / 60.0).ceil() as i64
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgressBook {
    /// Keyed by video id (`tt1:2:3` for an episode, the title id for a film).
    pub entries: BTreeMap<String, Progress>,
}

impl ProgressBook {
    pub fn record(&mut self, progress: Progress) {
        self.entries.insert(progress.video_id.clone(), progress);
    }

    pub fn get(&self, video_id: &str) -> Option<&Progress> {
        self.entries.get(video_id)
    }

    /// The most recent resumable entry of every title, newest first. A series shows once, at its latest episode.
    pub fn continue_watching(&self) -> Vec<&Progress> {
        let mut latest: BTreeMap<(&str, &str), &Progress> = BTreeMap::new();
        for entry in self.entries.values() {
            let slot = latest.entry((entry.kind.as_str(), entry.id.as_str())).or_insert(entry);
            if entry.updated_at > slot.updated_at {
                *slot = entry;
            }
        }
        let mut list: Vec<&Progress> = latest.into_values().filter(|p| p.is_resumable()).collect();
        list.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        list
    }

    pub fn is_watched(&self, video_id: &str) -> bool {
        self.entries.get(video_id).is_some_and(Progress::is_finished)
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
    pub preferred_audio_language: String,
    pub preferred_subtitle_language: String,
    pub subtitles_on: bool,
    pub volume: f64,
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
            preferred_audio_language: String::new(),
            preferred_subtitle_language: "eng".into(),
            subtitles_on: false,
            volume: 100.0,
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
        }
    }

    #[test]
    fn finished_means_95_percent_or_credits() {
        assert!(progress("a", "x", 2900.0, 1).is_finished());
        assert!(progress("a", "x", 2915.0, 1).is_finished());
        assert!(!progress("a", "x", 1500.0, 1).is_finished());
        assert!(!progress("a", "x", 10.0, 1).is_resumable());
    }

    #[test]
    fn continue_watching_shows_a_series_once_at_its_latest_episode() {
        let mut book = ProgressBook::default();
        book.record(progress("tt1:1:1", "tt1", 1000.0, 10));
        book.record(progress("tt1:1:2", "tt1", 500.0, 20));
        book.record(progress("tt2:1:1", "tt2", 600.0, 5));
        book.record(progress("tt3:1:1", "tt3", 2990.0, 30));
        let list = book.continue_watching();
        assert_eq!(list.iter().map(|p| p.video_id.as_str()).collect::<Vec<_>>(), vec!["tt1:1:2", "tt2:1:1"]);
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
