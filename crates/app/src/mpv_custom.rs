//! Custom mpv: the scripts, key bindings and options a person keeps in the `mpv` folder of Nuvio's data folder.
//!
//! The folder holds what mpv's own configuration folder would:
//!
//! * `scripts/`: Lua or JavaScript files, or folders with a `main.lua` or `main.js`, loaded when a stream opens;
//! * `script-opts/`: the `.conf` files the scripts read their settings from;
//! * `input.conf`: key and mouse bindings, in mpv's format.
//!
//! Nuvio does not read `mpv.conf`: its own options decide how the picture reaches the window, and a stray `vo=gpu` there
//! would leave the window black. Options go in the Playback settings instead.
//!
//! Input reaches the scripts because the player forwards what Nuvio itself has no use for (see `player_view.rs`); what they
//! put on screen (overlays, messages) is drawn by mpv into the picture, under Nuvio's controls.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use nuvio_core::paths;
use nuvio_core::store::Settings;

const INPUT_TEMPLATE: &str = "# Key and mouse bindings for the player, in mpv's input.conf format.\n\
# Turn on \"Custom mpv scripts and key bindings\" in Settings, Playback. Nuvio keeps its own keys (Space, the arrows, M, F,\n\
# J, K, L and Esc) unless you bind them here. Examples:\n\
#\n\
# a        show-text \"Hello from input.conf\"\n\
# ctrl+s   screenshot\n\
# WHEEL_UP    add volume 2\n\
# WHEEL_DOWN  add volume -2\n";

pub fn dir() -> PathBuf {
    paths::roaming_data_dir().join("mpv")
}

/// Makes the folder, with an input.conf holding examples, so there is something to open.
pub fn ensure_folder() -> PathBuf {
    let dir = dir();
    let _ = fs::create_dir_all(dir.join("scripts"));
    let _ = fs::create_dir_all(dir.join("script-opts"));
    let input = dir.join("input.conf");
    if !input.exists() {
        let _ = fs::write(&input, INPUT_TEMPLATE);
    }
    dir
}

/// What gets loaded from `scripts/`, by name order: scripts, and folders that hold one.
pub fn scripts() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir().join("scripts")) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name.starts_with('.') || name.starts_with('_') {
                return false;
            }
            if path.is_dir() {
                return path.join("main.lua").exists() || path.join("main.js").exists();
            }
            matches!(path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("lua" | "js" | "so" | "dll" | "dylib"))
        })
        .collect();
    found.sort();
    found
}

/// The separator of mpv's path lists.
const PATH_LIST_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };

/// What the player is given besides its own options. Cheap to clone: the player screen takes a copy every frame.
#[derive(Default, Clone)]
pub struct Custom {
    pub enabled: bool,
    /// Options, set before the player starts and after Nuvio's own, so they win.
    pub options: Vec<(String, String)>,
    /// The keys `input.conf` binds, as `normalize` writes them. Those go to mpv even when Nuvio uses them itself.
    pub bound: Arc<HashSet<String>>,
}

impl Custom {
    pub fn load(settings: &Settings) -> Self {
        if !settings.mpv_scripts {
            return Self::default();
        }
        let mut options: Vec<(String, String)> = Vec::new();
        let scripts: Vec<String> = scripts().iter().map(|p| p.display().to_string()).collect();
        if !scripts.is_empty() {
            options.push(("scripts".into(), scripts.join(PATH_LIST_SEPARATOR)));
        }
        // mpv only honours the key bindings that scripts register by default (`mp.add_key_binding`) while its own default
        // bindings are on, so they are on, with the few that would end the player or the file blocked first. The
        // person's input.conf comes after them and wins.
        let text = fs::read_to_string(dir().join("input.conf")).unwrap_or_default();
        let bound = bound_keys(&text);
        let generated = paths::cache_dir().join("mpv-input.conf");
        let _ = fs::create_dir_all(paths::cache_dir());
        if fs::write(&generated, format!("{BLOCKED}\n{text}")).is_ok() {
            options.push(("input-conf".into(), generated.display().to_string()));
            options.push(("input-default-bindings".into(), "yes".into()));
        }
        let script_options = script_options();
        if !script_options.is_empty() {
            options.push(("script-opts".into(), script_options.join(",")));
        }
        options.push(("osd-level".into(), if settings.mpv_osd { "1" } else { "0" }.into()));
        options.extend(extra_options(&settings.mpv_options));
        Self { enabled: true, options, bound: Arc::new(bound) }
    }

    /// Whether `input.conf` binds `key` (in any spelling `normalize` understands).
    pub fn binds(&self, key: &str) -> bool {
        self.enabled && !self.bound.is_empty() && self.bound.contains(&normalize(key))
    }
}

/// mpv's own default bindings that would do harm here: ending the player, or the file (which opens the next one).
const BLOCKED: &str = "q ignore\nQ ignore\nctrl+q ignore\nESC ignore\nENTER ignore\nKP_ENTER ignore\nSTOP ignore\nCLOSE_WIN ignore";

/// The settings of the scripts, for mpv's `script-opts`: each `script-opts/<script>.conf` line `key=value` becomes
/// `<script>-key=value`, the form mpv's own option parser passes on to a script that reads `<script>.conf`.
pub fn script_options() -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir().join("script-opts")) else { return Vec::new() };
    let mut out = Vec::new();
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        if path.extension().and_then(|e| e.to_str()) != Some("conf") {
            continue;
        }
        let Some(script) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Ok(text) = fs::read_to_string(&path) else { continue };
        out.extend(script_option_lines(script, &text));
    }
    out
}

/// `key=value` lines of one `.conf` file as `script-opts` entries. A value goes in with its length in front (`%5%a,b c`) so
/// commas and spaces in it cannot be taken for the end of the entry.
pub fn script_option_lines(script: &str, text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let (key, value) = (key.trim(), value.trim());
            (!key.is_empty()).then(|| format!("{script}-{key}=%{}%{value}", value.len()))
        })
        .collect()
}

/// `name=value` lines. The ones that would take the picture away from the window are dropped.
pub fn extra_options(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let name = name.trim().trim_start_matches("--").to_owned();
            (!name.is_empty() && !matches!(name.as_str(), "vo" | "wid" | "config" | "terminal")).then(|| (name, value.trim().to_owned()))
        })
        .collect()
}

/// The keys an `input.conf` binds: the first word of every line that is not a comment.
pub fn bound_keys(text: &str) -> HashSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_whitespace().next())
        .map(normalize)
        .collect()
}

/// One spelling for a key, so `Ctrl+Left`, `ctrl+LEFT` and `left` with ctrl held compare equal: modifiers sorted and in lower
/// case, key names in lower case, a single character as typed (`A` is shift and `a`).
pub fn normalize(key: &str) -> String {
    let (modifiers, base) = match key.strip_suffix("++") {
        Some(modifiers) => (modifiers, "+"),
        None => key.rsplit_once('+').unwrap_or(("", key)),
    };
    let mut modifiers: Vec<String> = modifiers.split('+').filter(|m| !m.is_empty()).map(str::to_lowercase).collect();
    modifiers.sort();
    let base = if base.chars().count() == 1 { base.to_owned() } else { base.to_lowercase() };
    modifiers.push(base);
    modifiers.join("+")
}

/// mpv's name for a key that types no text: the arrows, Enter, the function keys.
pub fn named_key(key: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::ArrowLeft => "LEFT",
        Key::ArrowRight => "RIGHT",
        Key::ArrowUp => "UP",
        Key::ArrowDown => "DOWN",
        Key::Escape => "ESC",
        Key::Enter => "ENTER",
        Key::Tab => "TAB",
        Key::Backspace => "BS",
        Key::Delete => "DEL",
        Key::Insert => "INS",
        Key::Home => "HOME",
        Key::End => "END",
        Key::PageUp => "PGUP",
        Key::PageDown => "PGDWN",
        Key::Space => "SPACE",
        other => {
            let name = other.name();
            // `F1` to `F35`.
            return (name.starts_with('F') && name[1..].parse::<u8>().is_ok()).then_some(name);
        }
    })
}

/// The character a letter or digit key types without Shift, for bindings with Ctrl, Alt or Command held (those type no
/// text).
pub fn plain_key(key: egui::Key, shift: bool) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(if shift { first.to_ascii_uppercase() } else { first.to_ascii_lowercase() })
}

/// `name` with the modifiers that are held, the way mpv writes them (`ctrl+alt+LEFT`).
pub fn with_modifiers(name: &str, modifiers: &egui::Modifiers) -> String {
    let mut out = String::new();
    if modifiers.ctrl {
        out.push_str("ctrl+");
    }
    if modifiers.alt {
        out.push_str("alt+");
    }
    // The Command key. Elsewhere egui never reports it.
    if modifiers.mac_cmd {
        out.push_str("meta+");
    }
    out.push_str(name);
    out
}

/// Keys the player screen already does something with. They stay Nuvio's unless `input.conf` binds them.
pub fn is_nuvios(name: &str) -> bool {
    let lower = normalize(name).to_lowercase();
    matches!(lower.as_str(), "space" | "left" | "right" | "up" | "down" | "esc" | "f11" | "k" | "j" | "l" | "m" | "f" | "<" | ">")
        || lower.ends_with("+left")
        || lower.ends_with("+right")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_the_player_already_uses_are_recognised_in_any_case() {
        for name in ["SPACE", "k", "K", "ctrl+LEFT", "shift+RIGHT", "F11", "<"] {
            assert!(is_nuvios(name), "{name}");
        }
        for name in ["a", "g", "F5", "ctrl+s", "UP2", "WHEEL_UP", "MBTN_RIGHT", "?"] {
            assert!(!is_nuvios(name), "{name}");
        }
    }

    #[test]
    fn key_names_follow_mpv() {
        assert_eq!(named_key(egui::Key::PageDown), Some("PGDWN"));
        assert_eq!(named_key(egui::Key::F5), Some("F5"));
        assert_eq!(named_key(egui::Key::A), None);
        assert_eq!(plain_key(egui::Key::S, false), Some('s'));
        assert_eq!(plain_key(egui::Key::S, true), Some('S'));
        assert_eq!(plain_key(egui::Key::ArrowLeft, false), None);
        let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        assert_eq!(with_modifiers("LEFT", &ctrl), "ctrl+LEFT");
        let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
        assert_eq!(with_modifiers("s", &cmd), "meta+s");
        assert_eq!(normalize("Meta+s"), normalize("meta+s"));
    }

    #[test]
    fn keys_are_written_one_way() {
        assert_eq!(normalize("Ctrl+LEFT"), "ctrl+left");
        assert_eq!(normalize("shift+ctrl+a"), normalize("ctrl+shift+a"));
        assert_eq!(normalize("A"), "A");
        assert_eq!(normalize("a"), "a");
        assert_eq!(normalize("ctrl++"), "ctrl++");
        assert_eq!(normalize("WHEEL_UP"), "wheel_up");
    }

    #[test]
    fn input_conf_binds_the_first_word_of_a_line() {
        let keys = bound_keys("# comment\n\nSPACE cycle pause\nctrl+Left seek -60  # trailing\n  WHEEL_UP add volume 2\n");
        assert!(keys.contains("space") && keys.contains("ctrl+left") && keys.contains("wheel_up"));
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn script_settings_become_script_opts_entries() {
        let lines = script_option_lines("uosc", "# colours\ncolor=foreground=ffffff,background=000000\n\nspeed_step = 0.25\n");
        assert_eq!(lines, vec!["uosc-color=%35%foreground=ffffff,background=000000".to_owned(), "uosc-speed_step=%4%0.25".to_owned()]);
    }

    #[test]
    fn extra_options_skip_comments_and_the_ones_that_break_the_window() {
        let options = extra_options("# hi\nvolume-max=200\n--deband=yes\nvo=gpu\nbad line\n");
        assert_eq!(options, vec![("volume-max".to_owned(), "200".to_owned()), ("deband".to_owned(), "yes".to_owned())]);
    }
}
