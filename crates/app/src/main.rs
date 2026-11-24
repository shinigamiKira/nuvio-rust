#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod args;
mod compositor;
mod glass;
mod icons;
mod images;
mod logging;
mod mpv_custom;
mod p2p;
mod platform;
mod player;
mod screens;
mod services;
mod state;
mod subtitles;
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
    install_panic_hook();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Nuvio starting");
    platform::start();

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

/// The release build aborts on a panic. The hook runs first: it writes the message to the log and tells the user where it
/// is, instead of the window just vanishing.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info.payload().downcast_ref::<&str>().map(|s| (*s).to_owned()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        tracing::error!(%location, %message, "Nuvio crashed");
        let text: Vec<u16> = format!("Nuvio hit an error and has to close.\n\nDetails are in:\n{}\n", logging::log_dir().join("nuvio.log").display()).encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "Nuvio".encode_utf16().chain(Some(0)).collect();
        // SAFETY: both strings are NUL terminated and outlive the call.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR);
        }
    }));
}

fn window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../resources/icon.png")).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}
