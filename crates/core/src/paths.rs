//! Where Nuvio keeps its files. Settings, the library and watch progress follow the user around (`%APPDATA%`);
//! caches and logs stay on this machine (`%LOCALAPPDATA%`). The folder name differs from the Kotlin build's `Nuvio`
//! so both can be installed side by side.

use std::path::PathBuf;
use std::sync::OnceLock;

const FOLDER: &str = "NuvioRust";

static OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Redirects both folders under `dir` (`--data-dir`), for tests that must not touch the real profile. Only the first
/// call has an effect.
pub fn use_data_dir(dir: PathBuf) {
    let _ = OVERRIDE.set(dir);
}

fn base(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(fallback))
        .join(FOLDER)
}

pub fn roaming_data_dir() -> PathBuf {
    match OVERRIDE.get() {
        Some(dir) => dir.join("roaming"),
        None => base("APPDATA", "nuvio-roaming"),
    }
}

pub fn local_data_dir() -> PathBuf {
    match OVERRIDE.get() {
        Some(dir) => dir.join("local"),
        None => base("LOCALAPPDATA", "nuvio-local"),
    }
}

pub fn cache_dir() -> PathBuf {
    local_data_dir().join("cache")
}
