//! Where the intro, the recap and the credits of an episode are, from IntroDB (the community database the Nuvio apps ask
//! for skip times). The lookup is by IMDb id, season and episode; an episode nobody has marked simply has no segments.

use std::time::Duration;

use serde::Deserialize;

use crate::http::{Http, HttpError};

const ENDPOINT: &str = "https://api.introdb.app/segments";
const TIMEOUT: Duration = Duration::from_secs(8);

/// A stretch of an episode, in seconds from its start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    pub fn contains(&self, seconds: f64) -> bool {
        seconds >= self.start && seconds < self.end
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Segments {
    pub intro: Option<Span>,
    pub recap: Option<Span>,
    /// The end credits.
    pub outro: Option<Span>,
}

#[derive(Deserialize)]
struct Raw {
    intro: Option<RawSpan>,
    recap: Option<RawSpan>,
    outro: Option<RawSpan>,
}

#[derive(Deserialize)]
struct RawSpan {
    start_sec: f64,
    end_sec: f64,
}

fn span(raw: Option<RawSpan>) -> Option<Span> {
    // A span that is empty or runs backwards is bad data, not something to skip.
    raw.filter(|s| s.start_sec >= 0.0 && s.end_sec - s.start_sec >= 3.0).map(|s| Span { start: s.start_sec, end: s.end_sec })
}

pub fn parse(body: &[u8]) -> Segments {
    match serde_json::from_slice::<Raw>(body) {
        Ok(raw) => Segments { intro: span(raw.intro), recap: span(raw.recap), outro: span(raw.outro) },
        Err(_) => Segments::default(),
    }
}

/// The segments of one episode. An unknown episode answers with all three empty; only a failed request is an error.
pub async fn fetch(http: &Http, imdb_id: &str, season: i64, episode: i64) -> Result<Segments, HttpError> {
    let url = format!("{ENDPOINT}?imdb_id={imdb_id}&season={season}&episode={episode}");
    Ok(parse(&http.get_with_retry(&url, TIMEOUT).await?))
}
