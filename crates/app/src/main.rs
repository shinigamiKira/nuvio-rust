#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod args;
mod glass;
mod icons;
mod images;
mod logging;
mod p2p;
mod player;
mod screens;
mod services;
mod state;
mod theme;
mod ui_kit;
mod window;

use std::sync::OnceLock;
use std::time::Instant;

use eframe::egui;

/// When the process started, for the timing lines in the log.
pub static START: OnceLock<Instant> = OnceLock::new();

fn main() -> anyhow::Result<()> {
    START.get_or_init(Instant::now);
    let args = args::Args::parse();
    if let Some(dir) = &args.data_dir {
        nuvio_core::paths::use_data_dir(dir.clone());
    }
    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Nuvio starting");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("nuvio-io")
        .enable_all()
        .build()?;

    let saved = nuvio_core::store::JsonFile::<nuvio_core::store::Settings>::new("settings.json").load().window;
    let (mut viewport, centered) = window::initial_viewport(args.fullscreen, args.size, saved);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, centered, multisampling: 0, ..Default::default() };
    let handle = runtime.handle().clone();
    eframe::run_native(
        "Nuvio",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, args, handle)))),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}

fn window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../resources/icon.png")).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}
