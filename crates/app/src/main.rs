#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod args;
mod icons;
mod images;
mod logging;
mod player;
mod screens;
mod services;
mod state;
mod theme;
mod ui_kit;

use eframe::egui;

fn main() -> anyhow::Result<()> {
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

    let (width, height) = args.size.unwrap_or((1280.0, 800.0));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Nuvio")
        .with_inner_size([width, height])
        .with_min_inner_size([960.0, 600.0])
        .with_fullscreen(args.fullscreen);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        multisampling: 0,
        ..Default::default()
    };
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
