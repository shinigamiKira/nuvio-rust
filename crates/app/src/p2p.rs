//! Torrent streams. Opens a magnet with librqbit, picks the video file, and serves it to mpv from a loopback HTTP port
//! (librqbit's own streaming endpoint, which answers Range requests and asks the swarm for the pieces mpv is about to
//! need). The port is bound to 127.0.0.1 and protected with a random password: other programs on the machine should not
//! be able to read the files of a stream.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use librqbit::api::TorrentIdOrHash;
use librqbit::dht::DhtPersistenceConfig;
use librqbit::http_api::{HttpApi, HttpApiOptions};
use librqbit_dualstack_sockets::{BindOpts, TcpListener};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, DhtSessionConfig, ListenerMode, ListenerOptions, Session, SessionOptions};
use nuvio_core::paths;

/// Public trackers added to every magnet. Most torrents list a few of their own; these only help the ones that do not.
const EXTRA_TRACKERS: [&str; 6] = [
    "udp://tracker.opentrackers.org:8191/announce",
    "udp://open.tracker.cl:1337/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
];

const VIDEO_EXTENSIONS: [&str; 12] = ["mkv", "mp4", "avi", "mov", "m4v", "webm", "ts", "m2ts", "wmv", "flv", "mpg", "mpeg"];

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Asking trackers and the DHT for peers that have the torrent's description.
    FindingPeers,
    /// Downloading the first pieces of the chosen file.
    Buffering,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub phase: Phase,
    pub peers: usize,
    pub download_bytes_per_sec: f64,
    pub file_name: Option<String>,
    pub file_size: u64,
    pub downloaded: u64,
}

/// What the player's overlay shows while the swarm is being found. Cheap to clone, shared with the task that updates it.
#[derive(Clone)]
pub struct Status {
    inner: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
}

impl Status {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Snapshot { phase: Phase::FindingPeers, peers: 0, download_bytes_per_sec: 0.0, file_name: None, file_size: 0, downloaded: 0 })),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().map(|s| s.clone()).unwrap_or_else(|e| e.into_inner().clone())
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        match self.inner.lock() {
            Ok(mut guard) => change(&mut guard),
            Err(poisoned) => change(&mut poisoned.into_inner()),
        }
    }

    /// Stops the task that polls the engine for numbers.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What the stream list knew about a torrent.
pub struct Request {
    pub info_hash: String,
    pub trackers: Vec<String>,
    pub file_index: Option<usize>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

pub struct Engine {
    session: Arc<Session>,
    port: u16,
    user: String,
    password: String,
}

fn random_token() -> String {
    // Not cryptography: only keeps a casual local process out. The standard hasher is seeded randomly per process.
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:016x}", hasher.finish())
}

impl Engine {
    pub async fn start() -> anyhow::Result<Self> {
        let downloads: PathBuf = paths::local_data_dir().join("torrents");
        // Streams are not kept: whatever an earlier run left behind is cache.
        let _ = std::fs::remove_dir_all(&downloads);
        std::fs::create_dir_all(&downloads).context("cannot create the torrent folder")?;

        let options = SessionOptions {
            dht: Some(DhtSessionConfig {
                persistence: Some(DhtPersistenceConfig {
                    dump_interval: Some(Duration::from_secs(60)),
                    config_filename: Some(paths::local_data_dir().join("dht.json")),
                }),
                ..DhtSessionConfig::default()
            }),
            persistence: None,
            listen: Some(ListenerOptions { mode: ListenerMode::TcpAndUtp, ..ListenerOptions::default() }),
            ..SessionOptions::default()
        };
        let session = Session::new_with_opts(downloads, options).await.context("cannot start the torrent session")?;

        let listener = TcpListener::bind_tcp(
            "127.0.0.1:0".parse().expect("a valid socket address"),
            BindOpts { request_dualstack: false, ..BindOpts::default() },
        )
        .context("cannot open the stream port")?;
        let port = listener.bind_addr().port();
        let (user, password) = (random_token(), random_token());
        let api = HttpApi::new(
            Api::new(session.clone(), None, None),
            Some(HttpApiOptions { read_only: true, basic_auth: Some((user.clone(), password.clone())), ..HttpApiOptions::default() }),
        );
        tokio::spawn(async move {
            if let Err(error) = api.make_http_api_and_run(listener, None).await {
                tracing::warn!(%error, "the torrent stream server stopped");
            }
        });
        tracing::info!(port, "torrent engine started");
        Ok(Self { session, port, user, password })
    }

    fn magnet(request: &Request) -> String {
        let mut magnet = format!("magnet:?xt=urn:btih:{}", request.info_hash);
        for tracker in request.trackers.iter().map(String::as_str).chain(EXTRA_TRACKERS) {
            magnet.push_str("&tr=");
            magnet.push_str(&nuvio_core::addon::encode_segment(tracker));
        }
        magnet
    }

    /// Finds peers, reads the torrent's file list, and returns the loopback URL mpv can play.
    pub async fn open(&self, request: &Request, status: &Status) -> anyhow::Result<String> {
        let magnet = Self::magnet(request);
        // First only the description, so the right file can be chosen before anything is downloaded.
        let listed = match self
            .session
            .add_torrent(AddTorrent::from_url(magnet), Some(AddTorrentOptions { list_only: true, ..AddTorrentOptions::default() }))
            .await
            .context("the torrent could not be reached")?
        {
            AddTorrentResponse::ListOnly(listed) => listed,
            _ => bail!("unexpected answer while reading the torrent"),
        };
        let files: Vec<(usize, String, u64)> =
            listed.info.iter_file_details().enumerate().map(|(index, details)| (index, details.filename.to_string(), details.len)).collect();
        let index = choose_file(&files, request.file_index, request.season, request.episode).ok_or_else(|| anyhow!("the torrent has no video file"))?;
        let (name, size) = (files[index].1.clone(), files[index].2);
        status.update(|s| {
            s.phase = Phase::Buffering;
            s.file_name = Some(name.clone());
            s.file_size = size;
        });
        tracing::info!(files = files.len(), index, size, "torrent file chosen");

        let response = self
            .session
            .add_torrent(
                AddTorrent::from_bytes(listed.torrent_bytes),
                Some(AddTorrentOptions { only_files: Some(vec![index]), overwrite: true, ..AddTorrentOptions::default() }),
            )
            .await
            .context("the torrent could not be started")?;
        let handle = response.into_handle().ok_or_else(|| anyhow!("the torrent could not be started"))?;
        handle.wait_until_initialized().await.context("the torrent could not be prepared")?;
        let hash = handle.info_hash().as_string();

        let (watched, status) = (handle.clone(), status.clone());
        tokio::spawn(async move {
            while !status.stop.load(Ordering::Relaxed) {
                let stats = watched.stats();
                status.update(|s| {
                    s.downloaded = stats.progress_bytes;
                    if let Some(live) = &stats.live {
                        s.download_bytes_per_sec = live.download_speed.mbps * 1024.0 * 1024.0;
                        s.peers = live.snapshot.peer_stats.live as usize;
                    }
                    if let Some(error) = &stats.error {
                        s.phase = Phase::Failed(error.clone());
                    }
                });
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        Ok(format!("http://{}:{}@127.0.0.1:{}/torrents/{hash}/stream/{index}", self.user, self.password, self.port))
    }

    /// Drops the torrent and the pieces downloaded for it.
    pub async fn close(&self, info_hash: &str) {
        let Ok(hash) = librqbit::dht::Id20::from_str(info_hash) else { return };
        if let Err(error) = self.session.delete(TorrentIdOrHash::Hash(hash), true).await {
            tracing::debug!(%error, "could not remove the torrent");
        }
    }

    pub async fn shutdown(&self) {
        self.session.stop().await;
    }
}

/// The file to play: the one the addon named, else the episode that matches `S01E02` in a season pack, else the biggest
/// video.
fn choose_file(files: &[(usize, String, u64)], hint: Option<usize>, season: Option<i64>, episode: Option<i64>) -> Option<usize> {
    let is_video = |name: &str| name.rsplit('.').next().is_some_and(|ext| VIDEO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
    if let Some(index) = hint.filter(|i| files.get(*i).is_some_and(|f| is_video(&f.1))) {
        return Some(index);
    }
    let videos: Vec<&(usize, String, u64)> = files.iter().filter(|f| is_video(&f.1)).collect();
    if let (Some(season), Some(episode)) = (season, episode) {
        let patterns = [format!("s{season:02}e{episode:02}"), format!("s{season}e{episode:02}"), format!("{season}x{episode:02}"), format!("s{season:02}.e{episode:02}")];
        let matching = videos.iter().filter(|f| {
            let name = f.1.to_ascii_lowercase();
            patterns.iter().any(|p| name.contains(p.as_str()))
        });
        if let Some(best) = matching.max_by_key(|f| f.2) {
            return Some(best.0);
        }
    }
    videos.iter().map(|f| (f.0, f.2)).max_by_key(|f| f.1).or_else(|| files.iter().map(|f| (f.0, f.2)).max_by_key(|f| f.1)).map(|f| f.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[(&str, u64)]) -> Vec<(usize, String, u64)> {
        names.iter().enumerate().map(|(i, (n, s))| (i, (*n).to_owned(), *s)).collect()
    }

    #[test]
    fn prefers_the_file_the_addon_named() {
        let list = files(&[("a.mkv", 10), ("b.mkv", 99)]);
        assert_eq!(choose_file(&list, Some(0), None, None), Some(0));
    }

    #[test]
    fn ignores_a_hint_that_is_not_a_video() {
        let list = files(&[("readme.txt", 10), ("movie.mkv", 99)]);
        assert_eq!(choose_file(&list, Some(0), None, None), Some(1));
    }

    #[test]
    fn season_packs_pick_the_episode() {
        let list = files(&[("Show.S01E01.mkv", 100), ("Show.S01E02.mkv", 90), ("Show.S01E02.sample.mkv", 5), ("Show.S01E03.mkv", 120)]);
        assert_eq!(choose_file(&list, None, Some(1), Some(2)), Some(1));
    }

    #[test]
    fn falls_back_to_the_biggest_video() {
        let list = files(&[("sample.mp4", 5), ("film.mkv", 900), ("cover.jpg", 1000)]);
        assert_eq!(choose_file(&list, None, None, None), Some(1));
    }

    #[test]
    fn a_torrent_without_video_falls_back_to_the_biggest_file() {
        let list = files(&[("a.bin", 5), ("b.bin", 9)]);
        assert_eq!(choose_file(&list, None, None, None), Some(1));
    }

    #[test]
    fn magnets_carry_the_trackers_of_the_stream_and_ours() {
        let request = Request { info_hash: "abc".into(), trackers: vec!["udp://t.example:1/announce".into()], file_index: None, season: None, episode: None };
        let magnet = Engine::magnet(&request);
        assert!(magnet.starts_with("magnet:?xt=urn:btih:abc&tr=udp%3A%2F%2Ft.example%3A1%2Fannounce"));
        assert!(magnet.contains("open.tracker.cl"));
    }
}
