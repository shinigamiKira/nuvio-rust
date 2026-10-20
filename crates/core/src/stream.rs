//! Streams: what the `stream` resource returns, and the facets a long list of them can be narrowed by.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json::JsonExt;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Stream {
    pub name: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub info_hash: Option<String>,
    pub file_idx: Option<i64>,
    pub external_url: Option<String>,
    pub yt_id: Option<String>,
    /// `tracker:...` and `dht:...` entries of a torrent stream.
    pub sources: Vec<String>,
    pub addon_name: String,
    pub addon_id: String,
    pub addon_logo: Option<String>,
    pub filename: Option<String>,
    pub video_size: Option<u64>,
    pub binge_group: Option<String>,
    /// Streams that need request headers (`proxyHeaders`) are not plain web streams.
    pub not_web_ready: bool,
    pub request_headers: Vec<(String, String)>,
    pub subtitles: Vec<StreamSubtitle>,
    /// Languages the addon parsed out of the release name (`clientResolve.stream.raw.parsed.languages`).
    pub parsed_languages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamSubtitle {
    pub url: String,
    pub language: String,
}

/// How a stream can be played, in the order the app tries them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playback {
    /// A URL mpv can open.
    Url(String),
    /// A torrent to stream with the built-in engine.
    Torrent { info_hash: String, file_idx: Option<i64>, trackers: Vec<String> },
    /// A page to open in the browser (a website or a YouTube video).
    External(String),
}

impl Stream {
    pub fn label(&self) -> &str {
        self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or("Stream")
    }

    pub fn direct_url(&self) -> Option<&str> {
        let url = self.url.as_deref()?.trim();
        (!url.is_empty() && !is_magnet(url) && !url.to_ascii_lowercase().starts_with("torrent://")).then_some(url)
    }

    fn magnet(&self) -> Option<&str> {
        [self.url.as_deref(), self.external_url.as_deref()].into_iter().flatten().map(str::trim).find(|u| is_magnet(u))
    }

    pub fn torrent_hash(&self) -> Option<String> {
        self.info_hash
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .or_else(|| self.magnet().and_then(btih_of_magnet))
    }

    pub fn playback(&self) -> Option<Playback> {
        if let Some(url) = self.direct_url() {
            return Some(Playback::Url(url.to_owned()));
        }
        if let Some(info_hash) = self.torrent_hash() {
            let trackers = self
                .sources
                .iter()
                .filter_map(|s| s.strip_prefix("tracker:"))
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
                .collect();
            return Some(Playback::Torrent { info_hash, file_idx: self.file_idx, trackers });
        }
        if let Some(url) = self.external_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            return Some(Playback::External(url.to_owned()));
        }
        self.yt_id.as_deref().map(str::trim).filter(|y| !y.is_empty()).map(|y| Playback::External(format!("https://www.youtube.com/watch?v={y}")))
    }

    pub fn is_playable(&self) -> bool {
        self.playback().is_some()
    }
}

fn is_magnet(url: &str) -> bool {
    url.trim_start().to_ascii_lowercase().starts_with("magnet:")
}

fn btih_of_magnet(magnet: &str) -> Option<String> {
    let lower = magnet.to_ascii_lowercase();
    let start = lower.find("btih:")? + 5;
    let end = magnet[start..].find('&').map(|i| start + i).unwrap_or(magnet.len());
    let hash = magnet[start..end].trim();
    (!hash.is_empty()).then(|| hash.to_owned())
}

pub fn parse_streams(payload: &[u8], addon_name: &str, addon_id: &str, addon_logo: Option<&str>) -> Vec<Stream> {
    let Ok(root) = serde_json::from_slice::<Value>(payload) else { return Vec::new() };
    root.array("streams")
        .iter()
        .filter_map(|s| {
            if !s.is_object() {
                return None;
            }
            let url = s.text("url");
            let info_hash = s.text("infoHash");
            let external_url = s.text("externalUrl");
            let yt_id = s.text("ytId");
            let client_resolve = s.object("clientResolve");
            if url.is_none() && info_hash.is_none() && external_url.is_none() && yt_id.is_none() && client_resolve.is_none() {
                return None;
            }
            let hints = s.object("behaviorHints");
            let headers = hints.and_then(|h| h.object("proxyHeaders")).and_then(|p| p.object("request"));
            let request_headers: Vec<(String, String)> = headers
                .and_then(Value::as_object)
                .map(|map| map.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect())
                .unwrap_or_default();
            let parsed_languages = client_resolve
                .and_then(|c| c.object("stream"))
                .and_then(|c| c.object("raw"))
                .and_then(|c| c.object("parsed"))
                .map(|p| p.texts("languages"))
                .unwrap_or_default();
            Some(Stream {
                name: s.text("name"),
                description: s.text("description").or_else(|| s.text("title")),
                url,
                info_hash,
                file_idx: s.int("fileIdx"),
                external_url,
                yt_id,
                sources: s.texts("sources"),
                addon_name: addon_name.to_owned(),
                addon_id: addon_id.to_owned(),
                addon_logo: addon_logo.map(str::to_owned),
                filename: hints.and_then(|h| h.text("filename")),
                video_size: hints.and_then(|h| h.int("videoSize")).and_then(|v| u64::try_from(v).ok()),
                binge_group: hints.and_then(|h| h.text("bingeGroup")),
                not_web_ready: hints.and_then(|h| h.flag("notWebReady")).unwrap_or(false) || !request_headers.is_empty(),
                request_headers,
                subtitles: s
                    .array("subtitles")
                    .iter()
                    .filter_map(|t| Some(StreamSubtitle { url: t.text("url")?, language: t.text("lang").unwrap_or_else(|| "und".into()) }))
                    .collect(),
                parsed_languages,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Facets

/// What a stream list can be narrowed by. Values are read from the stream's own text, so any addon works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facet {
    Resolution,
    Addon,
    Service,
    Hdr,
    Language,
}

impl Facet {
    pub const ALL: [Facet; 5] = [Facet::Resolution, Facet::Addon, Facet::Service, Facet::Hdr, Facet::Language];

    pub fn label(self) -> &'static str {
        match self {
            Facet::Resolution => "Resolution",
            Facet::Addon => "Addon",
            Facet::Service => "Service",
            Facet::Hdr => "HDR",
            Facet::Language => "Language",
        }
    }
}

pub const HDR: &str = "HDR";
pub const DOLBY_VISION: &str = "Dolby Vision";
pub const SDR: &str = "SDR";

const RESOLUTIONS: [&str; 6] = ["2160p", "1440p", "1080p", "720p", "576p", "480p"];
const HDR_ORDER: [&str; 3] = [DOLBY_VISION, HDR, SDR];

struct Patterns {
    resolution: Regex,
    uhd: Regex,
    dolby_vision: Regex,
    hdr: Regex,
    addon_line: Regex,
    language_line: Regex,
    trailing_parens: Regex,
    list_separator: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let re = |p: &str| Regex::new(p).expect("valid facet pattern");
        Patterns {
            resolution: re(r"(?i)\b(2160|1440|1080|720|576|480)\s?p\b"),
            uhd: re(r"(?i)\b(4k|uhd)\b"),
            dolby_vision: re(r"(?i)\b(dv|dovi|dolby[\s.\-]?vision)\b"),
            hdr: re(r"(?i)\bhdr(10)?\b|\bhdr10\+|\bhlg\b"),
            addon_line: re(r"(?im)^[ \t]*addon[ \t]*:[ \t]*(.+?)[ \t]*$"),
            language_line: re(r"(?im)^[ \t]*(?:\S{1,2}[ \t]+)?languages?[ \t]*:[ \t]*(.*?)[ \t]*$"),
            trailing_parens: re(r"\s*\([^)]*\)\s*$"),
            list_separator: re(r"\s*[,/|;]\s*"),
        }
    })
}

impl Stream {
    /// The values this stream has for every facet it can be told apart by. A stream with no recognisable value for a
    /// facet simply has none for it.
    pub fn facet_values(&self) -> BTreeMap<Facet, Vec<String>> {
        let p = patterns();
        let title = [self.name.as_deref(), self.description.as_deref(), self.filename.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        let mut values: BTreeMap<Facet, Vec<String>> = BTreeMap::new();

        let from_name = self.name.as_deref().and_then(|n| p.resolution.captures(n)).map(|c| c[1].to_owned());
        if let Some(resolution) = from_name.or_else(|| p.resolution.captures(&title).map(|c| c[1].to_owned())) {
            values.insert(Facet::Resolution, vec![format!("{resolution}p")]);
        } else if p.uhd.is_match(&title) {
            values.insert(Facet::Resolution, vec!["2160p".to_owned()]);
        }

        values.insert(
            Facet::Hdr,
            if p.dolby_vision.is_match(&title) {
                vec![DOLBY_VISION.to_owned(), HDR.to_owned()]
            } else if p.hdr.is_match(&title) {
                vec![HDR.to_owned()]
            } else {
                vec![SDR.to_owned()]
            },
        );

        if let Some(line) = self.description.as_deref().and_then(|d| p.addon_line.captures(d)).map(|c| c[1].to_owned()) {
            let parts: Vec<&str> = line.split('|').map(str::trim).collect();
            if let Some(addon) = parts.first().map(|a| p.trailing_parens.replace(a, "").trim().to_owned()).filter(|a| !a.is_empty()) {
                values.insert(Facet::Addon, vec![addon]);
            }
            if let Some(service) = parts.get(1).filter(|s| !s.is_empty()) {
                values.insert(Facet::Service, vec![service.to_uppercase()]);
            }
        }

        let mut languages: Vec<String> = Vec::new();
        if let Some(line) = self.description.as_deref().and_then(|d| p.language_line.captures(d)).map(|c| c[1].to_owned()) {
            languages.extend(p.list_separator.split(&line).map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()));
        }
        languages.extend(self.parsed_languages.iter().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()));
        let mut seen = std::collections::HashSet::new();
        languages.retain(|l| seen.insert(l.to_lowercase()));
        if !languages.is_empty() {
            values.insert(Facet::Language, languages);
        }
        values
    }

    /// Kept when, for every selected facet, the stream carries the selected value.
    pub fn matches(&self, selection: &BTreeMap<Facet, String>, except: Option<Facet>) -> bool {
        if selection.is_empty() {
            return true;
        }
        let values = self.facet_values();
        selection
            .iter()
            .all(|(facet, wanted)| Some(*facet) == except || values.get(facet).is_some_and(|v| v.iter().any(|x| x == wanted)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetOption {
    pub value: String,
    pub count: usize,
}

/// The choices to offer for `facet` among `streams`, counted over the streams that already pass the other selected
/// facets, so a choice that would leave nothing is not offered.
pub fn facet_options(streams: &[Stream], facet: Facet, selection: &BTreeMap<Facet, String>) -> Vec<FacetOption> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for stream in streams {
        if !stream.matches(selection, Some(facet)) {
            continue;
        }
        if let Some(values) = stream.facet_values().get(&facet) {
            for value in values {
                match counts.iter_mut().find(|(v, _)| v == value) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((value.clone(), 1)),
                }
            }
        }
    }
    let mut options: Vec<FacetOption> = counts.into_iter().map(|(value, count)| FacetOption { value, count }).collect();
    match facet {
        Facet::Resolution => options.sort_by_key(|o| RESOLUTIONS.iter().position(|r| *r == o.value).unwrap_or(RESOLUTIONS.len())),
        Facet::Hdr => options.sort_by_key(|o| HDR_ORDER.iter().position(|r| *r == o.value).unwrap_or(HDR_ORDER.len())),
        _ => options.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.to_lowercase().cmp(&b.value.to_lowercase()))),
    }
    options
}

/// Formats a byte count the way stream cards show it: `3.51 GB`.
pub fn format_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB { format!("{:.2} GB", b / GB) } else { format!("{:.0} MB", b / MB) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(name: &str, description: &str) -> Stream {
        Stream {
            name: Some(name.into()),
            description: Some(description.into()),
            url: Some("https://x.test/v.mkv".into()),
            ..Stream::default()
        }
    }

    #[test]
    fn reads_facets_from_the_text() {
        let s = stream(
            "2160p - WEB-DL - HEVC - DV - [NTb]",
            "Cyberpunk.S02E01.2160p.NF.WEB-DL.DDP5.1.DV.HEVC-NTb.mkv\nSize: 3.51 GB\nLanguage: English, French\nAddon : Debridio (Your Media) | TB",
        );
        let v = s.facet_values();
        assert_eq!(v[&Facet::Resolution], vec!["2160p"]);
        assert_eq!(v[&Facet::Hdr], vec![DOLBY_VISION, HDR]);
        assert_eq!(v[&Facet::Addon], vec!["Debridio"]);
        assert_eq!(v[&Facet::Service], vec!["TB"]);
        assert_eq!(v[&Facet::Language], vec!["English", "French"]);
    }

    #[test]
    fn plain_streams_are_sdr_and_uhd_means_2160() {
        let v = stream("4K Remux", "plain").facet_values();
        assert_eq!(v[&Facet::Resolution], vec!["2160p"]);
        assert_eq!(v[&Facet::Hdr], vec![SDR]);
        assert!(!v.contains_key(&Facet::Addon));
    }

    #[test]
    fn options_ignore_their_own_selection_but_respect_the_others() {
        let streams = vec![
            stream("2160p DV", "Addon : A | RD"),
            stream("2160p HDR", "Addon : B | TB"),
            stream("1080p", "Addon : A | RD"),
        ];
        let mut selection = BTreeMap::new();
        selection.insert(Facet::Addon, "A".to_owned());
        let resolutions = facet_options(&streams, Facet::Resolution, &selection);
        assert_eq!(resolutions.iter().map(|o| (o.value.as_str(), o.count)).collect::<Vec<_>>(), vec![("2160p", 1), ("1080p", 1)]);
        let addons = facet_options(&streams, Facet::Addon, &selection);
        assert_eq!(addons.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
        let hdr = facet_options(&streams, Facet::Hdr, &selection);
        assert_eq!(hdr.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), vec![DOLBY_VISION, HDR, SDR]);
    }

    #[test]
    fn selection_filters() {
        let s = stream("2160p DV", "Addon : A | RD");
        let mut selection = BTreeMap::new();
        selection.insert(Facet::Hdr, DOLBY_VISION.to_owned());
        assert!(s.matches(&selection, None));
        selection.insert(Facet::Service, "TB".to_owned());
        assert!(!s.matches(&selection, None));
        assert!(s.matches(&selection, Some(Facet::Service)));
    }

    #[test]
    fn parses_stream_responses() {
        let payload = br#"{"streams":[
            {"name":"A","url":"https://h/v.mp4","behaviorHints":{"videoSize":1073741824,"filename":"v.mp4","proxyHeaders":{"request":{"Referer":"r"}}}},
            {"name":"T","infoHash":"abcdef","fileIdx":2,"sources":["tracker:udp://t:1","dht:abc"]},
            {"title":"nothing playable"}
        ]}"#;
        let streams = parse_streams(payload, "Addon", "id", None);
        assert_eq!(streams.len(), 2);
        assert!(streams[0].not_web_ready);
        assert_eq!(streams[0].request_headers, vec![("Referer".to_owned(), "r".to_owned())]);
        assert_eq!(streams[0].video_size, Some(1 << 30));
        assert_eq!(
            streams[1].playback(),
            Some(Playback::Torrent { info_hash: "abcdef".into(), file_idx: Some(2), trackers: vec!["udp://t:1".into()] })
        );
    }

    #[test]
    fn magnets_and_external_links_are_not_urls() {
        let magnet = Stream { url: Some("magnet:?xt=urn:btih:ABC123&dn=x".into()), ..Stream::default() };
        assert_eq!(magnet.direct_url(), None);
        assert_eq!(magnet.torrent_hash().as_deref(), Some("ABC123"));
        let site = Stream { external_url: Some("https://site.test/watch".into()), ..Stream::default() };
        assert_eq!(site.playback(), Some(Playback::External("https://site.test/watch".into())));
        let yt = Stream { yt_id: Some("dQw4w9WgXcQ".into()), ..Stream::default() };
        assert!(matches!(yt.playback(), Some(Playback::External(u)) if u.contains("dQw4w9WgXcQ")));
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(3_769_341_000), "3.51 GB");
        assert_eq!(format_size(734_003_200), "700 MB");
    }
}
