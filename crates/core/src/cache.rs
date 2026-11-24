//! A folder of files named after the hash of what they cache. Used for addon responses (so the home screen can show
//! last session's shelves immediately) and for downloaded images.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: PathBuf,
}

pub struct Cached {
    pub bytes: Vec<u8>,
    /// Time since the entry was written.
    pub age: Duration,
}

impl DiskCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = fs::create_dir_all(&dir);
        Self { dir }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let digest = Sha256::digest(key.as_bytes());
        let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        self.dir.join(name)
    }

    pub fn get(&self, key: &str) -> Option<Cached> {
        let path = self.path_for(key);
        let bytes = fs::read(&path).ok()?;
        let age = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .unwrap_or(Duration::MAX);
        Some(Cached { bytes, age })
    }

    /// Writes through a temporary file so a crash never leaves half an entry behind.
    pub fn put(&self, key: &str, bytes: &[u8]) {
        write_atomically(&self.path_for(key), bytes);
    }

    /// Like `put`, for a program that opens the entry by path and reads its format from the extension (the player,
    /// given a subtitle file). Returns the path, or `None` when the write failed.
    pub fn put_file(&self, key: &str, extension: &str, bytes: &[u8]) -> Option<PathBuf> {
        let path = self.path_for(key).with_extension(extension);
        write_atomically(&path, bytes).then_some(path)
    }

    pub fn remove_all(&self) {
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// Deletes the least recently written files until the folder is under `max_bytes`.
    pub fn trim(&self, max_bytes: u64) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                meta.is_file().then(|| (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= max_bytes {
            return;
        }
        files.sort_by_key(|f| f.0);
        for (_, size, path) in files {
            if total <= max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
            }
        }
    }

    pub fn size_bytes(&self) -> u64 {
        dir_size(&self.dir)
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> bool {
    let temp = path.with_extension("tmp");
    let written = fs::File::create(&temp).and_then(|mut f| f.write_all(bytes)).and_then(|_| fs::rename(&temp, path));
    if let Err(error) = &written {
        tracing::debug!(%error, "cache write failed");
        let _ = fs::remove_file(&temp);
    }
    written.is_ok()
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuvio-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trips_and_reports_age() {
        let cache = DiskCache::new(temp_dir("roundtrip"));
        assert!(cache.get("a").is_none());
        cache.put("a", b"hello");
        let hit = cache.get("a").unwrap();
        assert_eq!(hit.bytes, b"hello");
        assert!(hit.age < Duration::from_secs(5));
    }

    #[test]
    fn files_keep_their_extension_and_content() {
        let cache = DiskCache::new(temp_dir("file"));
        let path = cache.put_file("https://addon.test/a.srt", "srt", b"1\n").unwrap();
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("srt"));
        assert_eq!(fs::read(&path).unwrap(), b"1\n");
        assert_eq!(cache.put_file("https://addon.test/a.srt", "srt", b"2\n"), Some(path.clone()), "the same key is the same file");
        assert_eq!(fs::read(&path).unwrap(), b"2\n");
    }

    #[test]
    fn trims_oldest_first() {
        let cache = DiskCache::new(temp_dir("trim"));
        for key in ["one", "two", "three"] {
            cache.put(key, &[0u8; 100]);
            std::thread::sleep(Duration::from_millis(20));
        }
        cache.trim(150);
        assert!(cache.get("one").is_none());
        assert!(cache.get("three").is_some());
        assert!(cache.size_bytes() <= 150);
    }
}
