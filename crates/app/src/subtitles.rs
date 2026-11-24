//! Subtitle files from addons. They are downloaded here, off the thread that plays, and handed to the player as files:
//! mpv opens a subtitle URL on the thread that decodes, so playback stops until the addon's server has answered.

use std::time::Duration;

use nuvio_core::cache::DiskCache;
use nuvio_core::client::AddonClient;
use nuvio_core::paths;

const TIMEOUT: Duration = Duration::from_secs(15);
/// Subtitle files are small, but nothing else cleans the folder.
pub const MAX_CACHE_BYTES: u64 = 20 * 1024 * 1024;

pub fn cache() -> DiskCache {
    DiskCache::new(paths::cache_dir().join("subtitles"))
}

/// Downloads `url` and returns the path of the file it was saved to.
pub async fn download(client: &AddonClient, url: &str) -> Result<String, String> {
    let bytes = client.http().get_with_retry(url, TIMEOUT).await.map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("the subtitle file is empty".into());
    }
    let path = cache().put_file(url, extension(url, &bytes), &bytes).ok_or("the subtitle file could not be saved")?;
    Ok(path.to_string_lossy().into_owned())
}

/// What mpv should treat the file as. A subtitle URL often ends in a query string or in nothing at all, so the name is
/// trusted only when it says something, and the content decides otherwise.
fn extension(url: &str, bytes: &[u8]) -> &'static str {
    let name = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    for known in ["srt", "vtt", "ass", "ssa"] {
        if name.ends_with(&format!(".{known}")) {
            return known;
        }
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        "vtt"
    } else if head.starts_with("[Script Info]") {
        "ass"
    } else {
        "srt"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_decides_when_it_says_something() {
        assert_eq!(extension("https://a.test/x/en.VTT?token=1", b"1\n00:00:01,000 --> 00:00:02,000\nhi"), "vtt");
        assert_eq!(extension("https://a.test/en.ass#frag", b""), "ass");
    }

    #[test]
    fn the_content_decides_otherwise() {
        assert_eq!(extension("https://a.test/subtitle/12345", "\u{feff}WEBVTT\n\n".as_bytes()), "vtt");
        assert_eq!(extension("https://a.test/subtitle/12345", b"[Script Info]\nTitle: x"), "ass");
        assert_eq!(extension("https://a.test/subtitle/12345", b"1\n00:00:01,000 --> 00:00:02,000\nhi"), "srt");
    }
}
