//! The client against a small local addon: manifest, catalog paging, meta, streams, the disk cache and the error paths.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use nuvio_core::addon::{CatalogDef, ExtraDef};
use nuvio_core::cache::DiskCache;
use nuvio_core::client::{AddonClient, FetchError, StreamSource};
use nuvio_core::http::{Http, HttpError};
use tiny_http::{Header, Response, Server};

struct TestAddon {
    base: String,
    hits: Arc<AtomicUsize>,
}

fn json_response(body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

/// Serves `/cfg/...` like a real addon would. Unknown paths are 404, `/boom/...` is a 500.
fn start() -> TestAddon {
    let server = Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    thread::spawn(move || {
        for request in server.incoming_requests() {
            counter.fetch_add(1, Ordering::SeqCst);
            let url = request.url().to_owned();
            let response = if url.starts_with("/boom") {
                Response::from_string("no").with_status_code(500)
            } else if url == "/cfg/manifest.json?token=t" {
                json_response(
                    r#"{"id":"test.addon","name":"Test","version":"1.0.0","types":["movie","series"],"idPrefixes":["tt"],
                    "resources":["catalog","meta","stream"],
                    "catalogs":[{"type":"movie","id":"top","name":"Top","extra":[{"name":"skip"}]}]}"#,
                )
            } else if url == "/cfg/catalog/movie/top.json?token=t" {
                json_response(r#"{"metas":[{"id":"tt1","type":"movie","name":"One"},{"id":"tt2","type":"movie","name":"Two"}]}"#)
            } else if url == "/cfg/catalog/movie/top/skip=2.json?token=t" {
                json_response(r#"{"metas":[{"id":"tt3","type":"movie","name":"Three"}]}"#)
            } else if url == "/cfg/catalog/movie/top/search=dune%20part.json?token=t" {
                json_response(r#"{"metas":[{"id":"tt9","type":"movie","name":"Dune"}]}"#)
            } else if url == "/cfg/meta/series/tt5%3A1%3A2.json?token=t" || url == "/cfg/meta/series/tt5.json?token=t" {
                json_response(r#"{"meta":{"id":"tt5","type":"series","name":"Show","videos":[{"id":"tt5:1:2","title":"Two","season":1,"episode":2}]}}"#)
            } else if url == "/cfg/stream/series/tt5%3A1%3A2.json?token=t" {
                json_response(r#"{"streams":[{"name":"2160p HDR","description":"Addon : Test | RD","url":"https://cdn.test/v.mkv"}]}"#)
            } else {
                Response::from_string("{}").with_status_code(404)
            };
            let _ = request.respond(response);
        }
    });
    TestAddon { base, hits }
}

fn client(name: &str) -> AddonClient {
    let dir = std::env::temp_dir().join(format!("nuvio-client-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    AddonClient::new(Http::new().unwrap(), DiskCache::new(dir))
}

fn top_catalog() -> CatalogDef {
    CatalogDef {
        kind: "movie".into(),
        id: "top".into(),
        name: "Top".into(),
        extra: vec![ExtraDef { name: "skip".into(), required: false, options: vec![] }],
    }
}

#[tokio::test]
async fn manifest_catalog_paging_and_search() {
    let addon = start();
    let client = client("paging");
    let manifest_url = format!("{}/cfg/manifest.json?token=t", addon.base);

    let manifest = client.manifest(&manifest_url).await.unwrap();
    assert_eq!(manifest.name, "Test");
    assert!(manifest.supports("stream", "series", "tt5:1:2"));

    let first = client.catalog(&manifest_url, &top_catalog(), 0, None).await.unwrap();
    assert_eq!(first.items.len(), 2);
    let second = client.catalog(&manifest_url, &top_catalog(), 2, None).await.unwrap();
    assert_eq!(second.items[0].name, "Three");

    let found = client.search(&manifest_url, &top_catalog(), "dune part").await.unwrap();
    assert_eq!(found.items[0].id, "tt9");
}

#[tokio::test]
async fn meta_and_streams() {
    let addon = start();
    let client = client("meta");
    let manifest_url = format!("{}/cfg/manifest.json?token=t", addon.base);

    let meta = client.meta(&manifest_url, "series", "tt5").await.unwrap();
    assert_eq!(meta.videos.len(), 1);

    let source = StreamSource { manifest_url: manifest_url.clone(), name: "Test".into(), id: "test.addon".into(), logo: None };
    let streams = client.streams(&source, "series", "tt5:1:2").await.unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].addon_name, "Test");
    assert_eq!(streams[0].direct_url(), Some("https://cdn.test/v.mkv"));
}

#[tokio::test]
async fn responses_are_cached_for_the_next_launch() {
    let addon = start();
    let client = client("cache");
    let manifest_url = format!("{}/cfg/manifest.json?token=t", addon.base);

    assert!(client.cached_manifest(&manifest_url).is_none());
    client.manifest(&manifest_url).await.unwrap();
    client.catalog(&manifest_url, &top_catalog(), 0, None).await.unwrap();
    let hits_after_fetch = addon.hits.load(Ordering::SeqCst);

    assert_eq!(client.cached_manifest(&manifest_url).unwrap().name, "Test");
    let (page, age) = client.cached_catalog(&manifest_url, &top_catalog(), None).unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(age.as_secs() < 5);
    assert_eq!(addon.hits.load(Ordering::SeqCst), hits_after_fetch, "reading the cache must not touch the network");
}

#[tokio::test]
async fn errors_do_not_leak_the_url() {
    let addon = start();
    let client = client("errors");

    let missing = client.manifest(&format!("{}/nowhere/secrettoken/manifest.json", addon.base)).await.unwrap_err();
    assert!(matches!(missing, FetchError::Http(HttpError::Status(404))));
    assert!(!missing.to_string().contains("secrettoken"));

    let server_error = client.manifest(&format!("{}/boom/secrettoken/manifest.json", addon.base)).await.unwrap_err();
    assert!(matches!(server_error, FetchError::Http(HttpError::Status(500))));
    // The 500 is transient, so it was tried twice.
    assert!(addon.hits.load(Ordering::SeqCst) >= 3);

    let refused = client.manifest("http://127.0.0.1:1/secrettoken/manifest.json").await.unwrap_err();
    assert!(!refused.to_string().contains("secrettoken"), "{refused}");
}
