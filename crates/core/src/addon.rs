//! Stremio addons: the manifest an addon publishes and the URLs its resources live at.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json::JsonExt;

/// Everything but `A-Z a-z 0-9 - _ . ~`, the characters the mobile app leaves alone in an id.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub logo: Option<String>,
    pub resources: Vec<Resource>,
    pub types: Vec<String>,
    pub id_prefixes: Vec<String>,
    pub catalogs: Vec<CatalogDef>,
    pub configurable: bool,
    pub configuration_required: bool,
    pub adult: bool,
    pub p2p: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub name: String,
    pub types: Vec<String>,
    pub id_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogDef {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub extra: Vec<ExtraDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraDef {
    pub name: String,
    pub required: bool,
    pub options: Vec<String>,
}

impl CatalogDef {
    pub fn supports_search(&self) -> bool {
        self.extra.iter().any(|e| e.name == "search")
    }

    pub fn supports_skip(&self) -> bool {
        self.extra.iter().any(|e| e.name == "skip")
    }

    /// A catalog the home screen can show without asking the user for anything.
    pub fn is_browsable(&self) -> bool {
        self.extra.iter().all(|e| !e.required)
    }

    /// A catalog that only answers searches (it has `search` as a required extra).
    pub fn is_search_only(&self) -> bool {
        self.extra.iter().any(|e| e.name == "search" && e.required)
    }
}

impl Manifest {
    /// Whether the addon serves `resource` for this content type and id.
    pub fn supports(&self, resource: &str, kind: &str, id: &str) -> bool {
        self.resources.iter().any(|r| {
            r.name == resource
                && r.types.iter().any(|t| t == kind)
                && (r.id_prefixes.is_empty() || r.id_prefixes.iter().any(|p| id.starts_with(p.as_str())))
        })
    }

    pub fn has_resource(&self, resource: &str) -> bool {
        self.resources.iter().any(|r| r.name == resource)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("the response is not JSON: {0}")]
    NotJson(String),
    #[error("the manifest has no \"{0}\"")]
    Missing(&'static str),
}

pub fn parse_manifest(payload: &[u8]) -> Result<Manifest, ManifestError> {
    let root: Value = serde_json::from_slice(payload).map_err(|e| ManifestError::NotJson(e.to_string()))?;
    let required = |key: &'static str| root.text(key).ok_or(ManifestError::Missing(key));
    let default_types = root.texts("types");
    let default_prefixes = root.texts("idPrefixes");

    let resources = root
        .array("resources")
        .iter()
        .filter_map(|entry| match entry {
            Value::String(name) if !name.trim().is_empty() => Some(Resource {
                name: name.trim().to_owned(),
                types: default_types.clone(),
                id_prefixes: default_prefixes.clone(),
            }),
            Value::Object(_) => {
                let name = entry.text("name")?;
                let types = Some(entry.texts("types")).filter(|t| !t.is_empty()).unwrap_or_else(|| default_types.clone());
                let prefixes =
                    Some(entry.texts("idPrefixes")).filter(|t| !t.is_empty()).unwrap_or_else(|| default_prefixes.clone());
                Some(Resource { name, types, id_prefixes: prefixes })
            }
            _ => None,
        })
        .collect();

    let catalogs = root
        .array("catalogs")
        .iter()
        .filter_map(|entry| {
            let kind = entry.text("type")?;
            let id = entry.text("id")?;
            let name = entry.text("name").unwrap_or_else(|| id.clone());
            // `extraSupported` is the older spelling of `extra`: a list of names, none of them required.
            let mut extra: Vec<ExtraDef> = entry
                .array("extra")
                .iter()
                .filter_map(|e| {
                    Some(ExtraDef {
                        name: e.text("name")?,
                        required: e.flag("isRequired").unwrap_or(false),
                        options: e.texts("options"),
                    })
                })
                .collect();
            for name in entry.texts("extraSupported") {
                if !extra.iter().any(|e| e.name == name) {
                    extra.push(ExtraDef { name, required: false, options: Vec::new() });
                }
            }
            Some(CatalogDef { kind, id, name, extra })
        })
        .collect();

    let hints = root.object("behaviorHints");
    let hint = |key: &str| hints.and_then(|h| h.flag(key)).unwrap_or(false);

    Ok(Manifest {
        id: required("id")?,
        name: required("name")?,
        description: root.text("description").unwrap_or_default(),
        version: root.text("version").unwrap_or_default(),
        logo: root.text("logo"),
        resources,
        types: default_types,
        id_prefixes: default_prefixes,
        catalogs,
        configurable: hint("configurable"),
        configuration_required: hint("configurationRequired"),
        adult: hint("adult"),
        p2p: hint("p2p"),
    })
}

/// Turns what a user typed or pasted into a manifest URL: `stremio://` becomes `https://`, and a bare addon address
/// gets `/manifest.json` appended.
pub fn normalize_manifest_url(input: &str) -> Option<String> {
    let mut url = input.trim().to_owned();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url.strip_prefix("stremio://") {
        url = format!("https://{rest}");
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        url = format!("https://{url}");
    }
    let (path, query) = match url.split_once('?') {
        Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
        None => (url.clone(), None),
    };
    let path = path.trim_end_matches('/');
    let path = if path.ends_with("/manifest.json") { path.to_owned() } else { format!("{path}/manifest.json") };
    Some(match query {
        Some(q) => format!("{path}?{q}"),
        None => path,
    })
}

/// The part of a manifest URL every resource URL is built on.
pub fn transport_base(manifest_url: &str) -> &str {
    let without_query = manifest_url.split('?').next().unwrap_or(manifest_url);
    without_query.strip_suffix("/manifest.json").unwrap_or(without_query)
}

pub fn encode_segment(segment: &str) -> String {
    utf8_percent_encode(segment, UNRESERVED).to_string()
}

/// `{base}/{resource}/{type}/{id}[/{extra}].json`, with the query string of the manifest URL carried over (some
/// addons keep their configuration there).
pub fn resource_url(manifest_url: &str, resource: &str, kind: &str, id: &str, extra: Option<&str>) -> String {
    let base = transport_base(manifest_url);
    let query = manifest_url.split_once('?').map(|(_, q)| q).filter(|q| !q.is_empty());
    let id = encode_segment(id);
    let mut url = match extra.filter(|e| !e.is_empty()) {
        Some(extra) => format!("{base}/{resource}/{kind}/{id}/{extra}.json"),
        None => format!("{base}/{resource}/{kind}/{id}.json"),
    };
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    url
}

/// Builds the `extra` path segment: `skip=20`, `search=dune`, `genre=Action&skip=40`.
pub fn extra_segment(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode_segment(name), encode_segment(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Resolves a logo path against the manifest URL. Addons write `logo` as an absolute URL, a `//host` URL or a path.
pub fn resolve_against(manifest_url: &str, reference: &str) -> String {
    if reference.starts_with("http://") || reference.starts_with("https://") || reference.starts_with("data:") {
        return reference.to_owned();
    }
    if let Some(rest) = reference.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let base = manifest_url.split('?').next().unwrap_or(manifest_url);
    let directory = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
    if let Some(path) = reference.strip_prefix('/') {
        let origin_end = directory
            .find("://")
            .and_then(|i| directory[i + 3..].find('/').map(|j| i + 3 + j))
            .unwrap_or(directory.len());
        return format!("{}/{}", &directory[..origin_end], path);
    }
    format!("{directory}/{reference}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "id": "org.example", "name": "Example", "version": "1.2.3",
        "types": ["movie", "series"], "idPrefixes": ["tt"],
        "resources": ["catalog", {"name": "stream", "types": ["movie"]}, {"name": "meta", "types": ["series"], "idPrefixes": ["kitsu"]}],
        "catalogs": [
            {"type": "movie", "id": "top", "name": "Top", "extra": [{"name": "genre", "options": ["Action"]}, {"name": "skip"}]},
            {"type": "movie", "id": "search", "extra": [{"name": "search", "isRequired": true}]},
            {"type": "series", "id": "old", "extraSupported": ["skip"]}
        ],
        "behaviorHints": {"configurable": true, "p2p": "true"}
    }"#;

    #[test]
    fn parses_manifest_with_inherited_types_and_prefixes() {
        let m = parse_manifest(MANIFEST.as_bytes()).unwrap();
        assert_eq!(m.name, "Example");
        assert!(m.supports("catalog", "series", "tt1"));
        assert!(m.supports("stream", "movie", "tt1"));
        assert!(!m.supports("stream", "series", "tt1"));
        assert!(m.supports("meta", "series", "kitsu:1"));
        assert!(!m.supports("meta", "series", "tt1"));
        assert!(m.configurable && m.p2p && !m.adult);
    }

    #[test]
    fn catalog_flags() {
        let m = parse_manifest(MANIFEST.as_bytes()).unwrap();
        assert!(m.catalogs[0].is_browsable() && m.catalogs[0].supports_skip());
        assert_eq!(m.catalogs[1].name, "search");
        assert!(m.catalogs[1].is_search_only() && !m.catalogs[1].is_browsable());
        assert!(m.catalogs[2].supports_skip());
    }

    #[test]
    fn manifest_without_id_is_rejected() {
        let error = parse_manifest(br#"{"name":"x","version":"1"}"#).unwrap_err();
        assert!(matches!(error, ManifestError::Missing("id")));
    }

    #[test]
    fn builds_resource_urls_and_keeps_the_query() {
        let url = resource_url("https://a.example/cfg123/manifest.json?token=abc", "stream", "series", "tt1:2:3", None);
        assert_eq!(url, "https://a.example/cfg123/stream/series/tt1%3A2%3A3.json?token=abc");
        let url = resource_url("https://a.example/manifest.json", "catalog", "movie", "top", Some("skip=20"));
        assert_eq!(url, "https://a.example/catalog/movie/top/skip=20.json");
    }

    #[test]
    fn normalizes_what_users_paste() {
        assert_eq!(normalize_manifest_url("stremio://a.example/x/manifest.json").unwrap(), "https://a.example/x/manifest.json");
        assert_eq!(normalize_manifest_url(" a.example/x/ ").unwrap(), "https://a.example/x/manifest.json");
        assert_eq!(normalize_manifest_url("https://a.example/manifest.json?k=v").unwrap(), "https://a.example/manifest.json?k=v");
        assert_eq!(normalize_manifest_url("   "), None);
    }

    #[test]
    fn encodes_extras() {
        assert_eq!(extra_segment(&[("search", "the dune & more")]), "search=the%20dune%20%26%20more");
        assert_eq!(extra_segment(&[("genre", "Action"), ("skip", "40")]), "genre=Action&skip=40");
    }

    #[test]
    fn resolves_logo_paths() {
        let m = "https://a.example/cfg/manifest.json?x=1";
        assert_eq!(resolve_against(m, "https://cdn.example/l.png"), "https://cdn.example/l.png");
        assert_eq!(resolve_against(m, "//cdn.example/l.png"), "https://cdn.example/l.png");
        assert_eq!(resolve_against(m, "/l.png"), "https://a.example/l.png");
        assert_eq!(resolve_against(m, "l.png"), "https://a.example/cfg/l.png");
    }
}
