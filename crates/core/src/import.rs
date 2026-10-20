//! Reads the addons out of an existing Nuvio for Windows (Kotlin) profile, so nothing has to be pasted twice. That
//! build keeps its preferences as JSON files of keys and values; the addon list is one of them and its exact layout has
//! changed between releases, so this looks for manifest URLs anywhere in the file instead of for a particular key.

use std::path::PathBuf;

use serde_json::Value;

pub fn kotlin_profile_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("Nuvio").join("prefs"))
}

/// Manifest URLs found in `payload`, in file order, without duplicates.
pub fn manifest_urls(payload: &[u8]) -> Vec<String> {
    let Ok(root) = serde_json::from_slice::<Value>(payload) else { return Vec::new() };
    let mut found = Vec::new();
    collect(&root, &mut found, 0);
    let mut seen = std::collections::HashSet::new();
    found.retain(|url| seen.insert(url.clone()));
    found
}

fn collect(value: &Value, found: &mut Vec<String>, depth: usize) {
    if depth > 6 {
        return;
    }
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if is_manifest_url(trimmed) {
                found.push(trimmed.to_owned());
            } else if trimmed.starts_with(['[', '{']) {
                // Preference files store lists as JSON text inside a string.
                if let Ok(inner) = serde_json::from_str::<Value>(trimmed) {
                    collect(&inner, found, depth + 1);
                }
            }
        }
        Value::Array(list) => list.iter().for_each(|v| collect(v, found, depth + 1)),
        Value::Object(map) => map.values().for_each(|v| collect(v, found, depth + 1)),
        _ => {}
    }
}

fn is_manifest_url(text: &str) -> bool {
    (text.starts_with("https://") || text.starts_with("http://"))
        && text.split('?').next().is_some_and(|path| path.ends_with("/manifest.json"))
}

/// The addon URLs of the Kotlin build installed for this user, if there is one.
pub fn kotlin_addon_urls() -> Vec<String> {
    let Some(dir) = kotlin_profile_dir() else { return Vec::new() };
    std::fs::read(dir.join("nuvio_addons.json")).map(|bytes| manifest_urls(&bytes)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_urls_nested_in_json_strings() {
        let payload = br#"{
            "installed_urls_1": "[\"https://a.example/manifest.json\", \"https://b.example/t/manifest.json?x=1\"]",
            "other": {"deep": ["https://a.example/manifest.json", "https://c.example/logo.png"]},
            "n": 4
        }"#;
        assert_eq!(manifest_urls(payload), vec!["https://a.example/manifest.json", "https://b.example/t/manifest.json?x=1"]);
    }

    #[test]
    fn garbage_is_empty() {
        assert!(manifest_urls(b"not json").is_empty());
    }
}
