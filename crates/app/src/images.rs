//! Poster and backdrop loading. Images are fetched once (and kept on disk), decoded off the UI thread and shrunk to the
//! size they are drawn at with a Lanczos filter. Showing a 1280 px backdrop through a texture sampler at a third of its
//! size is what makes thumbnails look soft or jagged, so the shrinking is done here, once, properly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use fast_image_resize::images::Image as ResizeImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use nuvio_core::cache::DiskCache;
use nuvio_core::http::Http;
use tokio::runtime::Handle;

/// Textures that were not drawn for this many frames are dropped once there are more than `KEEP_MIN` of them.
const UNUSED_FRAMES: u64 = 2400;
const KEEP_MIN: usize = 300;
const RETRY_FAILED_AFTER: Duration = Duration::from_secs(45);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(25);
/// Downloads and decodes at once. More just fights the UI thread for CPU.
const PARALLEL_LOADS: usize = 8;

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    url: String,
    width_bucket: u32,
}

enum Entry {
    Loading,
    Ready { texture: TextureHandle, ready_at: Instant, last_used: u64 },
    Failed(Instant),
}

pub enum Image {
    Loading,
    Ready { texture: TextureHandle, age: Duration },
    Failed,
}

struct Inner {
    ctx: egui::Context,
    runtime: Handle,
    http: Http,
    cache: DiskCache,
    entries: Mutex<HashMap<Key, Entry>>,
    frame: AtomicU64,
    permits: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone)]
pub struct Images(Arc<Inner>);

impl Images {
    pub fn new(ctx: egui::Context, runtime: Handle, http: Http, cache: DiskCache) -> Self {
        Self(Arc::new(Inner {
            ctx,
            runtime,
            http,
            cache,
            entries: Mutex::new(HashMap::new()),
            frame: AtomicU64::new(0),
            permits: Arc::new(tokio::sync::Semaphore::new(PARALLEL_LOADS)),
        }))
    }

    /// Call once per frame: advances the usage clock and now and then frees textures nobody looks at.
    pub fn begin_frame(&self) {
        let frame = self.0.frame.fetch_add(1, Ordering::Relaxed) + 1;
        if frame % 600 != 0 {
            return;
        }
        let Ok(mut entries) = self.0.entries.lock() else { return };
        if entries.len() <= KEEP_MIN {
            return;
        }
        entries.retain(|_, entry| match entry {
            Entry::Ready { last_used, .. } => frame.saturating_sub(*last_used) < UNUSED_FRAMES,
            _ => true,
        });
    }

    /// The image at `url`, ready to draw at up to `width_px` physical pixels wide. Starts loading on first use.
    pub fn get(&self, url: &str, width_px: f32) -> Image {
        let key = Key { url: url.to_owned(), width_bucket: bucket(width_px) };
        let frame = self.0.frame.load(Ordering::Relaxed);
        let mut entries = self.0.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get_mut(&key) {
            Some(Entry::Ready { texture, ready_at, last_used }) => {
                *last_used = frame;
                Image::Ready { texture: texture.clone(), age: ready_at.elapsed() }
            }
            Some(Entry::Loading) => Image::Loading,
            Some(Entry::Failed(at)) if at.elapsed() < RETRY_FAILED_AFTER => Image::Failed,
            _ => {
                entries.insert(key.clone(), Entry::Loading);
                drop(entries);
                self.spawn_load(key);
                Image::Loading
            }
        }
    }

    pub fn prefetch(&self, url: &str, width_px: f32) {
        let _ = self.get(url, width_px);
    }

    fn spawn_load(&self, key: Key) {
        let inner = self.0.clone();
        self.0.runtime.spawn(async move {
            let Ok(_permit) = inner.permits.clone().acquire_owned().await else { return };
            let bytes = match inner.cache.get(&key.url) {
                Some(hit) => Ok(hit.bytes),
                None => match inner.http.get_with_retry(&key.url, DOWNLOAD_TIMEOUT).await {
                    Ok(bytes) => {
                        inner.cache.put(&key.url, &bytes);
                        Ok(bytes)
                    }
                    Err(error) => Err(error.to_string()),
                },
            };
            let decoded = match bytes {
                Ok(bytes) => {
                    let width = key.width_bucket;
                    tokio::task::spawn_blocking(move || decode(&bytes, width)).await.map_err(|e| e.to_string()).and_then(|r| r)
                }
                Err(error) => Err(error),
            };
            let entry = match decoded {
                Ok(image) => {
                    let texture = inner.ctx.load_texture(format!("img-{}", key.url.len()), image, TextureOptions::LINEAR);
                    Entry::Ready { texture, ready_at: Instant::now(), last_used: inner.frame.load(Ordering::Relaxed) }
                }
                Err(error) => {
                    tracing::debug!(%error, "image failed");
                    Entry::Failed(Instant::now())
                }
            };
            if let Ok(mut entries) = inner.entries.lock() {
                entries.insert(key, entry);
            }
            inner.ctx.request_repaint();
        });
    }
}

/// Requested widths are rounded up to a multiple of 64 so a window being resized does not decode every frame.
fn bucket(width_px: f32) -> u32 {
    let width = width_px.max(32.0).ceil() as u32;
    width.div_ceil(64) * 64
}

fn decode(bytes: &[u8], target_width: u32) -> Result<ColorImage, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?.into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err("empty image".into());
    }
    // Never enlarge: a small source stays small and the sampler stretches it.
    if width <= target_width {
        return Ok(ColorImage::from_rgba_unmultiplied([width as usize, height as usize], image.as_raw()));
    }
    let target_height = ((height as f64) * (target_width as f64) / (width as f64)).round().max(1.0) as u32;
    let source = ResizeImage::from_vec_u8(width, height, image.into_raw(), PixelType::U8x4).map_err(|e| e.to_string())?;
    let mut shrunk = ResizeImage::new(target_width, target_height, PixelType::U8x4);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new().resize(&source, &mut shrunk, &options).map_err(|e| e.to_string())?;
    Ok(ColorImage::from_rgba_unmultiplied([target_width as usize, target_height as usize], shrunk.buffer()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn shrinks_to_the_bucket_and_keeps_the_aspect_ratio() {
        let decoded = decode(&png(1280, 720), 320).unwrap();
        assert_eq!(decoded.size, [320, 180]);
    }

    #[test]
    fn never_enlarges() {
        let decoded = decode(&png(100, 150), 512).unwrap();
        assert_eq!(decoded.size, [100, 150]);
    }

    #[test]
    fn buckets_round_up() {
        assert_eq!(bucket(1.0), 64);
        assert_eq!(bucket(200.0), 256);
        assert_eq!(bucket(256.0), 256);
    }
}
