//! Addon URLs carry the user's token in the path or query. Anything that reaches a log file or the screen goes through
//! here first.

/// Replaces every `http(s)://...` in `text` with `<url>`, keeping the host so the line stays useful when it is short
/// enough to be harmless. Tokens live in the path, so only scheme and host are kept.
pub fn urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = find_scheme(rest) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | '<' | ','))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        let host_end = url
            .find("://")
            .map(|i| i + 3)
            .and_then(|from| url[from..].find(['/', '?', '#']).map(|j| from + j))
            .unwrap_or(url.len());
        out.push_str(&url[..host_end]);
        if host_end < url.len() {
            out.push_str("/<redacted>");
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn find_scheme(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    match (lower.find("https://"), lower.find("http://")) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::urls;

    #[test]
    fn keeps_host_and_hides_path() {
        let line = "Failed to open https://addon.example.com/abc123token/manifest.json?x=1 (403)";
        assert_eq!(urls(line), "Failed to open https://addon.example.com/<redacted> (403)");
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert_eq!(urls("no urls here"), "no urls here");
    }

    #[test]
    fn handles_several_urls_and_bare_hosts() {
        let line = "a http://one.test/x b https://two.test";
        assert_eq!(urls(line), "a http://one.test/<redacted> b https://two.test");
    }
}
