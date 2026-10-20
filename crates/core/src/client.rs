//! Talks to addons: manifests, catalogs, meta, streams. Every successful response is also written to the disk cache, and
//! the `cached_*` readers let a screen show what was there last time before the network answers.

use std::time::Duration;

use crate::addon::{self, CatalogDef, Manifest};
use crate::cache::DiskCache;
use crate::http::{Http, HttpError};
use crate::meta::{self, CatalogPage, MetaDetails};
use crate::stream::{self, AddonSubtitle, Stream};

const MANIFEST_TIMEOUT: Duration = Duration::from_secs(12);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(15);
const META_TIMEOUT: Duration = Duration::from_secs(15);
/// Addons that resolve debrid links on request are slow the first time (several seconds is normal).
const STREAM_TIMEOUT: Duration = Duration::from_secs(40);
const SUBTITLE_TIMEOUT: Duration = Duration::from_secs(12);

/// How many entries of a catalog the home shelves keep.
pub const HOME_PREVIEW_ITEMS: usize = 40;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Clone)]
pub struct AddonClient {
    http: Http,
    cache: DiskCache,
}

/// What a request for a title's streams needs to know about the addon it goes to.
#[derive(Debug, Clone)]
pub struct StreamSource {
    pub manifest_url: String,
    pub name: String,
    pub id: String,
    pub logo: Option<String>,
}

impl AddonClient {
    pub fn new(http: Http, cache: DiskCache) -> Self {
        Self { http, cache }
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    async fn fetch(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, FetchError> {
        let body = self.http.get_with_retry(url, timeout).await?;
        self.cache.put(url, &body);
        Ok(body)
    }

    // -- manifests -------------------------------------------------------------------------------------------------

    pub async fn manifest(&self, manifest_url: &str) -> Result<Manifest, FetchError> {
        let body = self.fetch(manifest_url, MANIFEST_TIMEOUT).await?;
        addon::parse_manifest(&body).map_err(|e| FetchError::Invalid(e.to_string()))
    }

    pub fn cached_manifest(&self, manifest_url: &str) -> Option<Manifest> {
        addon::parse_manifest(&self.cache.get(manifest_url)?.bytes).ok()
    }

    // -- catalogs --------------------------------------------------------------------------------------------------

    fn catalog_url(manifest_url: &str, def: &CatalogDef, skip: usize, query: Option<&str>) -> String {
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if let Some(q) = query {
            pairs.push(("search", q.to_owned()));
        }
        if skip > 0 {
            pairs.push(("skip", skip.to_string()));
        }
        let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(n, v)| (*n, v.as_str())).collect();
        let extra = (!borrowed.is_empty()).then(|| addon::extra_segment(&borrowed));
        addon::resource_url(manifest_url, "catalog", &def.kind, &def.id, extra.as_deref())
    }

    pub async fn catalog(
        &self,
        manifest_url: &str,
        def: &CatalogDef,
        skip: usize,
        max_items: Option<usize>,
    ) -> Result<CatalogPage, FetchError> {
        let url = Self::catalog_url(manifest_url, def, skip, None);
        let body = self.fetch(&url, CATALOG_TIMEOUT).await?;
        meta::parse_catalog(&body, max_items).map_err(|e| FetchError::Invalid(e.to_string()))
    }

    /// The first page of a catalog as stored on disk, with how old it is.
    pub fn cached_catalog(&self, manifest_url: &str, def: &CatalogDef, max_items: Option<usize>) -> Option<(CatalogPage, Duration)> {
        let hit = self.cache.get(&Self::catalog_url(manifest_url, def, 0, None))?;
        Some((meta::parse_catalog(&hit.bytes, max_items).ok()?, hit.age))
    }

    pub async fn search(&self, manifest_url: &str, def: &CatalogDef, query: &str) -> Result<CatalogPage, FetchError> {
        let url = Self::catalog_url(manifest_url, def, 0, Some(query));
        let body = self.http.get_with_retry(&url, CATALOG_TIMEOUT).await?;
        meta::parse_catalog(&body, None).map_err(|e| FetchError::Invalid(e.to_string()))
    }

    // -- meta ------------------------------------------------------------------------------------------------------

    pub async fn meta(&self, manifest_url: &str, kind: &str, id: &str) -> Result<MetaDetails, FetchError> {
        let url = addon::resource_url(manifest_url, "meta", kind, id, None);
        let body = self.fetch(&url, META_TIMEOUT).await?;
        meta::parse_meta(&body).map_err(|e| FetchError::Invalid(e.to_string()))
    }

    pub fn cached_meta(&self, manifest_url: &str, kind: &str, id: &str) -> Option<(MetaDetails, Duration)> {
        let hit = self.cache.get(&addon::resource_url(manifest_url, "meta", kind, id, None))?;
        Some((meta::parse_meta(&hit.bytes).ok()?, hit.age))
    }

    // -- subtitles -------------------------------------------------------------------------------------------------

    /// Subtitle files an addon offers for a video. `filename` helps addons that match on the release name.
    pub async fn subtitles(&self, source: &StreamSource, kind: &str, id: &str, filename: Option<&str>) -> Result<Vec<AddonSubtitle>, FetchError> {
        let extra = filename.map(|f| addon::extra_segment(&[("filename", f)]));
        let url = addon::resource_url(&source.manifest_url, "subtitles", kind, id, extra.as_deref());
        let body = self.http.get_with_retry(&url, SUBTITLE_TIMEOUT).await?;
        Ok(stream::parse_subtitles(&body, &source.name))
    }

    // -- streams ---------------------------------------------------------------------------------------------------

    /// Streams are not written to the disk cache: their URLs are tokenised and short lived.
    pub async fn streams(&self, source: &StreamSource, kind: &str, id: &str) -> Result<Vec<Stream>, FetchError> {
        let url = addon::resource_url(&source.manifest_url, "stream", kind, id, None);
        let body = self.http.get_with_retry(&url, STREAM_TIMEOUT).await?;
        Ok(stream::parse_streams(&body, &source.name, &source.id, source.logo.as_deref()))
    }
}
