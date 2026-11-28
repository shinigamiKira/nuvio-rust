//! Where Nuvio keeps its files. Settings, the library and watch progress are the person's data; caches and logs can be
//! rebuilt. On Windows those are `%APPDATA%` and `%LOCALAPPDATA%`; on macOS `~/Library/Application Support`,
//! `~/Library/Caches` and `~/Library/Logs`. The folder name differs from the Kotlin build's `Nuvio` so both can be
//! installed side by side.

use std::path::PathBuf;
use std::sync::OnceLock;

const FOLDER: &str = "NuvioRust";

static OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Redirects every folder under `dir` (`--data-dir`), for tests that must not touch the real profile. Only the first
/// call has an effect.
pub fn use_data_dir(dir: PathBuf) {
    let _ = OVERRIDE.set(dir);
}

#[derive(Clone, Copy)]
enum Kind {
    Roaming,
    Local,
    Logs,
}

#[cfg(target_os = "macos")]
fn platform_dir(kind: Kind) -> PathBuf {
    mac_dir(std::env::var_os("HOME").map(PathBuf::from), kind)
}

/// The folder of `kind` in the home folder `home`. Without one (which a login session always has) the temporary folder
/// stands in, so the app still starts.
#[cfg(any(target_os = "macos", test))]
fn mac_dir(home: Option<PathBuf>, kind: Kind) -> PathBuf {
    let library = home.filter(|h| h.is_absolute()).unwrap_or_else(std::env::temp_dir).join("Library");
    match kind {
        Kind::Roaming => library.join("Application Support"),
        Kind::Local => library.join("Caches"),
        Kind::Logs => library.join("Logs"),
    }
    .join(FOLDER)
}

#[cfg(not(target_os = "macos"))]
fn platform_dir(kind: Kind) -> PathBuf {
    let (var, fallback) = match kind {
        Kind::Roaming => ("APPDATA", "nuvio-roaming"),
        Kind::Local | Kind::Logs => ("LOCALAPPDATA", "nuvio-local"),
    };
    let base = std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(fallback)).join(FOLDER);
    match kind {
        Kind::Logs => base.join("logs"),
        _ => base,
    }
}

pub fn roaming_data_dir() -> PathBuf {
    match OVERRIDE.get() {
        Some(dir) => dir.join("roaming"),
        None => platform_dir(Kind::Roaming),
    }
}

pub fn local_data_dir() -> PathBuf {
    match OVERRIDE.get() {
        Some(dir) => dir.join("local"),
        None => platform_dir(Kind::Local),
    }
}

pub fn log_dir() -> PathBuf {
    match OVERRIDE.get() {
        Some(dir) => dir.join("local").join("logs"),
        None => platform_dir(Kind::Logs),
    }
}

pub fn cache_dir() -> PathBuf {
    local_data_dir().join("cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_folders_are_under_the_library_of_the_home_folder() {
        let home = || Some(PathBuf::from("/Users/someone"));
        assert_eq!(mac_dir(home(), Kind::Roaming), PathBuf::from("/Users/someone/Library/Application Support/NuvioRust"));
        assert_eq!(mac_dir(home(), Kind::Local), PathBuf::from("/Users/someone/Library/Caches/NuvioRust"));
        assert_eq!(mac_dir(home(), Kind::Logs), PathBuf::from("/Users/someone/Library/Logs/NuvioRust"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn nothing_is_kept_in_the_temporary_folder() {
        for dir in [roaming_data_dir(), local_data_dir(), log_dir()] {
            assert!(!dir.starts_with(std::env::temp_dir()), "{}", dir.display());
        }
    }
}
