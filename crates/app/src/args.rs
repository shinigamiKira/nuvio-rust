//! Command line. Besides the plain app these are the hooks the tests use to open a screen and take a picture of it.

use std::path::PathBuf;

use eframe::egui::{Key, Modifiers};

#[derive(Debug, Default, Clone)]
pub struct Args {
    /// `--play <url>`: open the player on a URL right away.
    pub play: Option<String>,
    /// `--start <seconds>`: where `--play` starts.
    pub start: f64,
    /// `--open <screen>`: `home`, `search`, `library`, `settings`, `detail:<type>:<id>`, `streams:<type>:<id>[:<video id>]`.
    pub open: Option<String>,
    /// `--screenshot <file.png>`: save the window after `--screenshot-after` seconds, then quit.
    pub screenshot: Option<PathBuf>,
    pub screenshot_after: f64,
    /// `--quit-after <seconds>`: close the window after a while (for timing runs).
    pub quit_after: Option<f64>,
    /// `--data-dir <dir>`: keep settings, library and caches there instead of in the user profile.
    pub data_dir: Option<PathBuf>,
    /// `--scale <factor>`: force the UI scale (what a 125 % or 150 % display looks like).
    pub scale: Option<f32>,
    /// `--size <w>x<h>`: window size in points.
    pub size: Option<(f32, f32)>,
    /// `--fullscreen`.
    pub fullscreen: bool,
    /// `--mouse <x>,<y>`: with `--screenshot`, move the pointer there first (hover states).
    pub mouse: Option<(f32, f32)>,
    /// `--select <n>`: with `--open streams:...`, play the n-th stream of the list.
    pub select: Option<usize>,
    /// `--key <seconds>:<Key>`, repeatable: press a key (egui names: `Space`, `ArrowUp`, `F`, with `cmd+`, `ctrl+`,
    /// `alt+` or `shift+` in front) after a while.
    pub keys: Vec<(f64, String)>,
    /// `--click <seconds>:<x>,<y>`, repeatable: click there after a while (points, not pixels).
    pub clicks: Vec<(f64, f32, f32)>,
    /// `--type <seconds>:<text>`, repeatable: type into the focused field after a while.
    pub texts: Vec<(f64, String)>,
}

impl Args {
    pub fn parse() -> Self {
        // A stream URL holds a token, so a test script passes it in the environment rather than on the command line.
        let mut args = Args { screenshot_after: 4.0, play: std::env::var("NUVIO_PLAY_URL").ok(), ..Args::default() };
        let mut iter = std::env::args().skip(1);
        while let Some(flag) = iter.next() {
            let mut value = || iter.next().unwrap_or_default();
            match flag.as_str() {
                "--play" => args.play = Some(value()),
                "--start" => args.start = value().parse().unwrap_or(0.0),
                "--open" => args.open = Some(value()),
                "--screenshot" => args.screenshot = Some(PathBuf::from(value())),
                "--screenshot-after" => args.screenshot_after = value().parse().unwrap_or(4.0),
                "--quit-after" => args.quit_after = value().parse().ok(),
                "--data-dir" => args.data_dir = Some(PathBuf::from(value())),
                "--scale" => args.scale = value().parse().ok(),
                "--size" => {
                    let text = value();
                    args.size = text.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                }
                "--fullscreen" => args.fullscreen = true,
                "--mouse" => {
                    let text = value();
                    args.mouse = text.split_once(',').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)));
                }
                "--select" => args.select = value().parse().ok(),
                "--key" => {
                    let text = value();
                    if let Some((at, key)) = text.split_once(':') {
                        args.keys.push((at.parse().unwrap_or(0.0), key.to_owned()));
                    }
                }
                "--type" => {
                    let text = value();
                    if let Some((at, typed)) = text.split_once(':') {
                        args.texts.push((at.parse().unwrap_or(0.0), typed.to_owned()));
                    }
                }
                "--click" => {
                    let text = value();
                    let parsed = text.split_once(':').and_then(|(at, pos)| {
                        let (x, y) = pos.split_once(',')?;
                        Some((at.parse().ok()?, x.parse().ok()?, y.parse().ok()?))
                    });
                    args.clicks.extend(parsed);
                }
                other => eprintln!("unknown option {other}"),
            }
        }
        args
    }
}

/// `cmd+ctrl+F` as the modifiers and the key. `cmd` is the Command key of a Mac.
pub fn parse_chord(chord: &str) -> Option<(Modifiers, Key)> {
    let mut parts: Vec<&str> = chord.split('+').collect();
    let key = Key::from_name(parts.pop()?)?;
    let mut modifiers = Modifiers::NONE;
    for part in parts {
        match part {
            "cmd" => (modifiers.mac_cmd, modifiers.command) = (true, true),
            "ctrl" => modifiers.ctrl = true,
            "alt" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            _ => return None,
        }
    }
    Some((modifiers, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_names_its_modifiers_and_key() {
        assert_eq!(parse_chord("Space"), Some((Modifiers::NONE, Key::Space)));
        let (modifiers, key) = parse_chord("cmd+ctrl+F").unwrap();
        assert!(modifiers.mac_cmd && modifiers.command && modifiers.ctrl && !modifiers.alt);
        assert_eq!(key, Key::F);
        assert_eq!(parse_chord("hyper+F"), None);
        assert_eq!(parse_chord("cmd+"), None);
    }
}
