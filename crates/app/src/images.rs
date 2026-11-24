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

/// Textures that were not drawn for this long are dropped once there are more than `KEEP_MIN` of them. Time, not frames: a
/// high refresh display draws two or three times as many frames in the same minute, and a poster that was dropped while its
/// shelf was scrolled out of sight comes back as a flash of placeholder.
const UNUSED_FOR: Duration = Duration::from_secs(120);
const KEEP_MIN: usize = 300;
/// The age given to a picture that must not fade in: it replaces another size of the same image, or stands in for one.
const SETTLED: Duration = Duration::from_secs(60);
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
    Ready { texture: TextureHandle, ready_at: Instant, last_used: Instant },
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

    /// Call once per frame: now and then frees textures nobody looks at.
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
            Entry::Ready { last_used, .. } => last_used.elapsed() < UNUSED_FOR,
            _ => true,
        });
    }

    /// The image at `url`, ready to draw at up to `width_px` physical pixels wide. Starts loading on first use. While the
    /// exact size is being prepared, another size of the same image stands in for it, so a card that grows under the
    /// pointer or a window that is resized never drops its picture for a placeholder.
    pub fn get(&self, url: &str, width_px: f32) -> Image {
        let key = Key { url: url.to_owned(), width_bucket: bucket(width_px) };
        let mut entries = self.0.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get_mut(&key) {
            Some(Entry::Ready { texture, ready_at, last_used }) => {
                *last_used = Instant::now();
                Image::Ready { texture: texture.clone(), age: ready_at.elapsed() }
            }
            Some(Entry::Loading) => stand_in(&entries, url),
            Some(Entry::Failed(at)) if at.elapsed() < RETRY_FAILED_AFTER => Image::Failed,
            _ => {
                entries.insert(key.clone(), Entry::Loading);
                let image = stand_in(&entries, url);
                drop(entries);
                self.spawn_load(key);
                image
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
            // The size of the picture that is fetched follows the size it is drawn at; the address the addon named is the
            // fallback when the larger one is not there.
            let wanted = sized_url(&key.url, key.width_bucket as f32);
            let mut bytes = fetch(&inner, &wanted).await;
            if bytes.is_err() && wanted != key.url {
                bytes = fetch(&inner, &key.url).await;
            }
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
                    // Another size of this picture is already on screen: swap without a fade.
                    let replaces = inner.entries.lock().map(|entries| has_ready(&entries, &key.url)).unwrap_or(false);
                    let now = Instant::now();
                    let ready_at = if replaces { now.checked_sub(SETTLED).unwrap_or(now) } else { now };
                    Entry::Ready { texture, ready_at, last_used: now }
                }
                Err(error) => {
                    tracing::debug!(%error, "image failed");
                    Entry::Failed(Instant::now())
                }
            };
            if let Ok(mut entries) = inner.entries.lock() {
                entries.insert(key, entry);
            }
            crate::services::wake(&inner.ctx);
        });
    }
}

async fn fetch(inner: &Inner, url: &str) -> Result<Vec<u8>, String> {
    if let Some(hit) = inner.cache.get(url) {
        return Ok(hit.bytes);
    }
    let bytes = inner.http.get_with_retry(url, DOWNLOAD_TIMEOUT).await.map_err(|e| e.to_string())?;
    inner.cache.put(url, &bytes);
    Ok(bytes)
}

/// Metahub serves each poster and backdrop in several sizes, and addons name the small one: a 300 px poster. Drawn at
/// 210 points on a 1080p screen (or a 1280 px backdrop across 1920 pixels) that is enlarged by the sampler, which is what
/// made everything look smudged. The size that is fetched is the smallest one that covers `width_px`.
fn sized_url(url: &str, width_px: f32) -> String {
    const HOST: &str = "images.metahub.space/";
    let Some(at) = url.find(HOST) else { return url.to_owned() };
    let rest = &url[at + HOST.len()..];
    let mut parts = rest.splitn(3, '/');
    let (Some(kind), Some(_), Some(tail)) = (parts.next(), parts.next(), parts.next()) else { return url.to_owned() };
    let tier = match kind {
        "poster" if width_px > 500.0 => "large",
        "poster" if width_px > 300.0 => "medium",
        "poster" => "small",
        "background" if width_px > 1280.0 => "large",
        "background" if width_px > 780.0 => "medium",
        "background" => "small",
        _ => return url.to_owned(),
    };
    format!("{}{HOST}{kind}/{tier}/{tail}", &url[..at])
}

fn has_ready(entries: &HashMap<Key, Entry>, url: &str) -> bool {
    entries.iter().any(|(key, entry)| key.url == url && matches!(entry, Entry::Ready { .. }))
}

/// The largest ready size of `url`, drawn as it is, or the placeholder when there is none.
fn stand_in(entries: &HashMap<Key, Entry>, url: &str) -> Image {
    entries
        .iter()
        .filter_map(|(key, entry)| match entry {
            Entry::Ready { texture, .. } if key.url == url => Some((key.width_bucket, texture)),
            _ => None,
        })
        .max_by_key(|(width, _)| *width)
        .map(|(_, texture)| Image::Ready { texture: texture.clone(), age: SETTLED })
        .unwrap_or(Image::Loading)
}

/// A card is decoded at the width it is drawn at, so every texel lands on one pixel; any other size goes through the
/// sampler's bilinear filter, which blurs. Wide pictures (backdrops) are rounded up to a multiple of 8 so a window being
/// resized does not decode them every frame.
fn bucket(width_px: f32) -> u32 {
    let width = width_px.max(32.0).round() as u32;
    if width > 640 { width.div_ceil(8) * 8 } else { width }
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
    fn another_size_stands_in_while_the_right_one_loads() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture("t", ColorImage::filled([2, 2], egui::Color32::WHITE), TextureOptions::LINEAR);
        let ready = || Entry::Ready { texture: texture.clone(), ready_at: Instant::now(), last_used: Instant::now() };
        let key = |url: &str, width_bucket| Key { url: url.into(), width_bucket };
        let mut entries = HashMap::new();
        entries.insert(key("a", 128), ready());
        entries.insert(key("a", 256), ready());
        entries.insert(key("a", 320), Entry::Loading);
        match stand_in(&entries, "a") {
            Image::Ready { age, .. } => assert!(age >= SETTLED, "a stand-in must not fade in"),
            _ => panic!("expected the ready size"),
        }
        assert!(matches!(stand_in(&entries, "b"), Image::Loading));
        assert!(has_ready(&entries, "a") && !has_ready(&entries, "b"));
    }

    #[test]
    fn small_pictures_keep_their_exact_width_and_wide_ones_round_up() {
        assert_eq!(bucket(1.0), 32);
        assert_eq!(bucket(210.0), 210);
        assert_eq!(bucket(640.0), 640);
        assert_eq!(bucket(1919.0), 1920);
        assert_eq!(bucket(1920.0), 1920);
    }

    #[test]
    fn metahub_pictures_are_fetched_at_the_size_they_are_drawn() {
        let poster = "https://images.metahub.space/poster/small/tt1/img";
        assert_eq!(sized_url(poster, 210.0), poster);
        assert_eq!(sized_url(poster, 315.0), "https://images.metahub.space/poster/medium/tt1/img");
        assert_eq!(sized_url(poster, 600.0), "https://images.metahub.space/poster/large/tt1/img");
        let background = "https://images.metahub.space/background/medium/tt1/img";
        assert_eq!(sized_url(background, 1920.0), "https://images.metahub.space/background/large/tt1/img");
        assert_eq!(sized_url(background, 1000.0), background);
        let other = "https://example.com/poster/small/x.jpg";
        assert_eq!(sized_url(other, 1000.0), other);
    }
}
