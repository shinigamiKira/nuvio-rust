//! Command line. Besides the plain app these are the hooks the tests use to open a screen and take a picture of it.

use std::path::PathBuf;

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
                other => eprintln!("unknown option {other}"),
            }
        }
        args
    }
}
