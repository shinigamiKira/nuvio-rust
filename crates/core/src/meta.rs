//! What catalogs and meta endpoints return: the posters on a shelf and the full page of a title.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json::JsonExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PosterShape {
    #[default]
    Poster,
    Square,
    Landscape,
}

/// One entry of a catalog.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MetaPreview {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub shape: PosterShape,
    pub description: Option<String>,
    pub release_info: Option<String>,
    pub imdb_rating: Option<String>,
    pub genres: Vec<String>,
}

impl MetaPreview {
    /// Identifies the title across catalogs: the same film listed by two addons is one entry.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    pub role: Option<String>,
    pub photo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub released: Option<String>,
    pub thumbnail: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i64>,
    pub rating: Option<f64>,
}

impl Video {
    pub fn is_special(&self) -> bool {
        self.season == Some(0)
    }

    /// `S1 E3`, as the episode is labelled across the app.
    pub fn code(&self) -> Option<String> {
        Some(format!("S{} E{}", self.season?, self.episode?))
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MetaDetails {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub imdb_id: Option<String>,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub description: Option<String>,
    pub release_info: Option<String>,
    pub status: Option<String>,
    pub imdb_rating: Option<String>,
    pub age_rating: Option<String>,
    pub runtime: Option<String>,
    pub genres: Vec<String>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub cast: Vec<Person>,
    pub country: Option<String>,
    pub awards: Option<String>,
    pub language: Option<String>,
    pub website: Option<String>,
    pub default_video_id: Option<String>,
    pub trailers: Vec<Trailer>,
    pub videos: Vec<Video>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trailer {
    pub key: String,
    pub name: String,
    pub site: String,
}

impl MetaDetails {
    pub fn is_series(&self) -> bool {
        self.kind == "series" || self.kind == "anime" || self.videos.iter().filter(|v| v.season.is_some()).count() > 1
    }

    /// Season numbers in ascending order, with specials (season 0) last.
    pub fn seasons(&self) -> Vec<i64> {
        let mut seasons: Vec<i64> = self.videos.iter().filter_map(|v| v.season).collect();
        seasons.sort_unstable();
        seasons.dedup();
        if let Some(pos) = seasons.iter().position(|s| *s == 0) {
            let zero = seasons.remove(pos);
            seasons.push(zero);
        }
        seasons
    }

    pub fn episodes_of(&self, season: i64) -> Vec<&Video> {
        let mut list: Vec<&Video> = self.videos.iter().filter(|v| v.season == Some(season)).collect();
        list.sort_by_key(|v| v.episode.unwrap_or(0));
        list
    }

    pub fn video(&self, id: &str) -> Option<&Video> {
        self.videos.iter().find(|v| v.id == id)
    }

    pub fn as_preview(&self) -> MetaPreview {
        MetaPreview {
            id: self.id.clone(),
            kind: self.kind.clone(),
            name: self.name.clone(),
            poster: self.poster.clone(),
            background: self.background.clone(),
            logo: self.logo.clone(),
            shape: PosterShape::Poster,
            description: self.description.clone(),
            release_info: self.release_info.clone(),
            imdb_rating: self.imdb_rating.clone(),
            genres: self.genres.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MetaError {
    #[error("the response is not JSON: {0}")]
    NotJson(String),
    #[error("the response has no meta object")]
    NoMeta,
}

pub struct CatalogPage {
    pub items: Vec<MetaPreview>,
    /// Entries in the response before duplicates and incomplete ones were dropped. Decides whether another page may
    /// exist: addons page in fixed sizes, so a short page is the last one.
    pub raw_count: usize,
}

pub fn parse_catalog(payload: &[u8], max_items: Option<usize>) -> Result<CatalogPage, MetaError> {
    let root: Value = serde_json::from_slice(payload).map_err(|e| MetaError::NotJson(e.to_string()))?;
    let metas = root.array("metas");
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for meta in metas {
        if max_items.is_some_and(|max| items.len() >= max) {
            break;
        }
        let (Some(id), Some(kind), Some(name)) = (meta.text("id"), meta.text("type"), meta.text("name")) else {
            continue;
        };
        let landscape = meta.text("landscapePoster");
        let item = MetaPreview {
            poster: meta.text("poster"),
            background: meta.text("background").or_else(|| meta.text("banner")).or_else(|| landscape.clone()),
            logo: meta.text("logo"),
            shape: match meta.text("posterShape").as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("square") => PosterShape::Square,
                Some("landscape") => PosterShape::Landscape,
                _ => PosterShape::Poster,
            },
            description: meta.text("description"),
            release_info: meta.text("releaseInfo"),
            imdb_rating: meta.text("imdbRating"),
            genres: meta.texts("genres"),
            id,
            kind,
            name,
        };
        if seen.insert(item.key()) {
            items.push(item);
        }
    }
    Ok(CatalogPage { items, raw_count: metas.len() })
}

pub fn parse_meta(payload: &[u8]) -> Result<MetaDetails, MetaError> {
    let root: Value = serde_json::from_slice(payload).map_err(|e| MetaError::NotJson(e.to_string()))?;
    let looks_like_meta = |v: &Value| v.text("id").is_some() && v.text("type").is_some() && v.text("name").is_some();
    // Servers wrap the object in `meta`, in `data.meta`, in `data`, or send it bare.
    let meta = [root.object("meta"), root.object("data").and_then(|d| d.object("meta")), root.object("data").filter(|d| looks_like_meta(d)), Some(&root).filter(|r| looks_like_meta(r))]
        .into_iter()
        .flatten()
        .next()
        .ok_or(MetaError::NoMeta)?;

    let extras = meta.object("app_extras");
    let links = meta.array("links");
    let linked = |categories: &[&str]| -> Vec<String> {
        links
            .iter()
            .filter(|l| l.text("category").is_some_and(|c| categories.iter().any(|x| c.eq_ignore_ascii_case(x))))
            .filter_map(|l| l.text("name"))
            .collect()
    };

    let mut directors = csv_or_list(meta, "director");
    directors.extend(extras.map(|e| names_of(e, "directors")).unwrap_or_default());
    directors.extend(linked(&["director", "directors"]));
    let mut writers = csv_or_list(meta, "writer");
    writers.extend(extras.map(|e| names_of(e, "writers")).unwrap_or_default());
    writers.extend(linked(&["writer", "writers", "screenplay"]));

    let mut cast: Vec<Person> = Vec::new();
    let mut add_person = |person: Person| {
        let key = person.name.to_lowercase();
        match cast.iter_mut().find(|p| p.name.to_lowercase() == key) {
            Some(existing) => {
                existing.role = existing.role.take().or(person.role);
                existing.photo = existing.photo.take().or(person.photo);
            }
            None => cast.push(person),
        }
    };
    if let Some(extras) = extras {
        for person in people_of(extras, "cast") {
            add_person(person);
        }
    }
    for name in csv_or_list(meta, "cast").into_iter().chain(linked(&["cast", "actor", "actors"])) {
        add_person(Person { name, role: None, photo: None });
    }

    let age_rating = [
        meta.text("ageRating"),
        extras.and_then(|e| e.text("certificationLocal")),
        extras.and_then(|e| e.text("certification")),
    ]
    .into_iter()
    .flatten()
    .next();

    let videos = meta
        .array("videos")
        .iter()
        .filter_map(|v| {
            let id = v.text("id")?;
            let title = v.text("title").or_else(|| v.text("name"))?;
            Some(Video {
                id,
                title,
                released: v.text("released"),
                thumbnail: v.text("thumbnail"),
                season: v.int("season"),
                episode: v.int("episode").or_else(|| v.int("number")),
                overview: v.text("overview").or_else(|| v.text("description")),
                runtime_minutes: v.text("runtime").and_then(|r| parse_runtime_minutes(&r)),
                rating: v.float("rating").filter(|r| *r > 0.0),
            })
        })
        .collect();

    let trailers = meta
        .array("trailers")
        .iter()
        .filter_map(|t| {
            let key = t.text("key").or_else(|| t.text("source")).or_else(|| t.text("ytId")).or_else(|| t.text("ytid"))?;
            Some(Trailer {
                name: t.text("name").unwrap_or_else(|| "Trailer".into()),
                site: t.text("site").unwrap_or_else(|| "YouTube".into()),
                key,
            })
        })
        .collect();

    let hints = meta.object("behaviorHints");
    Ok(MetaDetails {
        id: meta.text("id").ok_or(MetaError::NoMeta)?,
        kind: meta.text("type").ok_or(MetaError::NoMeta)?,
        name: meta.text("name").ok_or(MetaError::NoMeta)?,
        imdb_id: meta.text("imdb_id"),
        poster: meta.text("poster"),
        background: meta.text("background").or_else(|| meta.text("landscapePoster")),
        logo: meta.text("logo"),
        description: meta.text("description"),
        release_info: meta.text("releaseInfo"),
        status: meta.text("status"),
        imdb_rating: meta.text("imdbRating"),
        age_rating,
        runtime: meta.text("runtime"),
        genres: meta.texts("genres"),
        directors: dedup(directors),
        writers: dedup(writers),
        cast,
        country: meta.text("country"),
        awards: meta.text("awards"),
        language: meta.text("language"),
        website: meta.text("website"),
        default_video_id: hints.and_then(|h| h.text("defaultVideoId")),
        trailers,
        videos,
    })
}

fn dedup(names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names.into_iter().map(|n| n.trim().to_owned()).filter(|n| !n.is_empty() && seen.insert(n.to_lowercase())).collect()
}

/// `director` is a list in some addons and a comma separated string in others.
fn csv_or_list(meta: &Value, key: &str) -> Vec<String> {
    match meta.get(key) {
        Some(Value::Array(_)) => meta.texts(key),
        Some(Value::String(s)) => s.split(',').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect(),
        _ => Vec::new(),
    }
}

fn people_of(extras: &Value, key: &str) -> Vec<Person> {
    match extras.get(key) {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|entry| match entry {
                Value::Object(_) => Some(Person {
                    name: entry.text("name")?,
                    role: entry.text("character"),
                    photo: entry.text("photo"),
                }),
                Value::String(name) if !name.trim().is_empty() => {
                    Some(Person { name: name.trim().to_owned(), role: None, photo: None })
                }
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|name| Person { name: name.to_owned(), role: None, photo: None })
            .collect(),
        _ => Vec::new(),
    }
}

fn names_of(extras: &Value, key: &str) -> Vec<String> {
    people_of(extras, key).into_iter().map(|p| p.name).collect()
}

/// Runtimes arrive as `"148 min"`, `"2h 28m"`, `"148"` or `"PT2H28M"`.
pub fn parse_runtime_minutes(text: &str) -> Option<i64> {
    let lower = text.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let lower = lower.strip_prefix("pt").unwrap_or(&lower).to_owned();
    let mut total = 0i64;
    let mut number = String::new();
    let mut found_unit = false;
    for c in lower.chars() {
        if c.is_ascii_digit() {
            number.push(c);
        } else if !number.is_empty() && (c == 'h' || c == 'm') {
            let value: i64 = number.parse().ok()?;
            total += if c == 'h' { value * 60 } else { value };
            number.clear();
            found_unit = true;
        } else if !number.is_empty() && !c.is_alphabetic() && c != ' ' {
            break;
        }
    }
    if found_unit {
        return Some(total);
    }
    // No unit at all (`"148"`) or an unknown one (`"148 minutes"` handled above by the `m`).
    number.parse().ok().filter(|v: &i64| *v > 0).or_else(|| lower.split_whitespace().next().and_then(|n| n.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_skips_incomplete_and_duplicate_entries() {
        let payload = br#"{"metas":[
            {"id":"tt1","type":"movie","name":"A","poster":"p","posterShape":"landscape","releaseInfo":2020},
            {"id":"tt1","type":"movie","name":"A again"},
            {"id":"tt2","type":"movie"},
            {"id":"tt3","type":"series","name":"C","landscapePoster":"l"}
        ]}"#;
        let page = parse_catalog(payload, None).unwrap();
        assert_eq!(page.raw_count, 4);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].shape, PosterShape::Landscape);
        assert_eq!(page.items[0].release_info.as_deref(), Some("2020"));
        assert_eq!(page.items[1].background.as_deref(), Some("l"));
    }

    #[test]
    fn catalog_honours_the_item_limit() {
        let payload = br#"{"metas":[{"id":"1","type":"m","name":"a"},{"id":"2","type":"m","name":"b"},{"id":"3","type":"m","name":"c"}]}"#;
        assert_eq!(parse_catalog(payload, Some(2)).unwrap().items.len(), 2);
    }

    #[test]
    fn meta_reads_wrapped_and_bare_objects() {
        let bare = br#"{"id":"tt1","type":"movie","name":"A"}"#;
        let wrapped = br#"{"data":{"meta":{"id":"tt1","type":"movie","name":"A"}}}"#;
        assert_eq!(parse_meta(bare).unwrap().name, "A");
        assert_eq!(parse_meta(wrapped).unwrap().name, "A");
        assert!(matches!(parse_meta(br#"{"meta":{"id":"x"}}"#), Err(MetaError::NoMeta)));
    }

    #[test]
    fn meta_merges_people_from_every_source() {
        let payload = br#"{"meta":{"id":"tt1","type":"movie","name":"A",
            "director":"Jane Roe, John Doe","cast":["Ann Lee","Bo Kim"],
            "links":[{"name":"Ann Lee","category":"Cast","url":"x"},{"name":"Jane Roe","category":"Director","url":"y"}],
            "app_extras":{"cast":[{"name":"ann lee","character":"Maria","photo":"ph"}]}}}"#;
        let meta = parse_meta(payload).unwrap();
        assert_eq!(meta.directors, vec!["Jane Roe", "John Doe"]);
        assert_eq!(meta.cast.len(), 2);
        assert_eq!(meta.cast[0].role.as_deref(), Some("Maria"));
        assert_eq!(meta.cast[0].photo.as_deref(), Some("ph"));
    }

    #[test]
    fn series_seasons_put_specials_last() {
        let payload = br#"{"meta":{"id":"tt1","type":"series","name":"S","videos":[
            {"id":"a","title":"x","season":0,"episode":1},
            {"id":"b","title":"y","season":2,"episode":2},
            {"id":"c","title":"z","season":1,"episode":1},
            {"id":"d","title":"w","season":2,"episode":1}]}}"#;
        let meta = parse_meta(payload).unwrap();
        assert!(meta.is_series());
        assert_eq!(meta.seasons(), vec![1, 2, 0]);
        let ids: Vec<&str> = meta.episodes_of(2).iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["d", "b"]);
    }

    #[test]
    fn runtimes() {
        assert_eq!(parse_runtime_minutes("148 min"), Some(148));
        assert_eq!(parse_runtime_minutes("2h 28m"), Some(148));
        assert_eq!(parse_runtime_minutes("PT2H28M"), Some(148));
        assert_eq!(parse_runtime_minutes("95"), Some(95));
        assert_eq!(parse_runtime_minutes(""), None);
    }
}
