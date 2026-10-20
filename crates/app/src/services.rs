//! Background work. A task runs on the tokio runtime, and its result comes back to the UI thread as a `Msg` that the
//! app applies at the start of the next frame.

use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use eframe::egui;
use nuvio_core::addon::Manifest;
use nuvio_core::client::AddonClient;
use nuvio_core::meta::{CatalogPage, MetaDetails};
use nuvio_core::stream::Stream;
use tokio::runtime::Handle;

use crate::images::Images;
use crate::p2p::Engine;

pub enum Msg {
    Manifest { url: String, result: Result<Manifest, String>, from_cache: bool },
    Catalog { key: String, result: Result<CatalogPage, String>, from_cache: bool },
    CatalogMore { key: String, skip: usize, result: Result<CatalogPage, String> },
    Meta { request: u64, result: Result<MetaDetails, String> },
    Streams { request: u64, addon_url: String, result: Result<Vec<Stream>, String> },
    Search { request: u64, key: String, title: String, subtitle: String, result: Result<CatalogPage, String> },
    Subtitles { request: u64, entries: Vec<nuvio_core::stream::StreamSubtitle> },
    KotlinAddons(Vec<String>),
    /// A torrent stream is ready to play (its loopback URL), or could not be opened.
    Torrent { request: u64, result: Result<String, String> },
}

#[derive(Clone)]
pub struct Services {
    pub runtime: Handle,
    pub client: AddonClient,
    pub images: Images,
    pub ctx: egui::Context,
    tx: Sender<Msg>,
    p2p: Arc<tokio::sync::OnceCell<Result<Arc<Engine>, String>>>,
}

impl Services {
    pub fn new(runtime: Handle, client: AddonClient, images: Images, ctx: egui::Context, tx: Sender<Msg>) -> Self {
        Self { runtime, client, images, ctx, tx, p2p: Arc::new(tokio::sync::OnceCell::new()) }
    }

    /// The torrent engine, started the first time a torrent stream is played.
    pub async fn engine(&self) -> Result<Arc<Engine>, String> {
        self.p2p.get_or_init(|| async { Engine::start().await.map(Arc::new).map_err(|e| format!("{e:#}")) }).await.clone()
    }

    /// The engine if it was ever started (for shutting it down).
    pub fn engine_if_started(&self) -> Option<Arc<Engine>> {
        self.p2p.get().and_then(|r| r.as_ref().ok().cloned())
    }

    /// Runs `work` in the background and delivers its message to the UI.
    pub fn spawn<F>(&self, work: F)
    where
        F: Future<Output = Msg> + Send + 'static,
    {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let message = work.await;
            // The receiver is gone when the app is closing; there is nobody left to tell.
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    /// Delivers a message produced on the UI thread through the same path as background results.
    pub fn post(&self, message: Msg) {
        let _ = self.tx.send(message);
        self.ctx.request_repaint();
    }
}
