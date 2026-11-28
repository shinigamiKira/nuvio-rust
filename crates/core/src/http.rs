//! The HTTP client every request goes through.
//!
//! Two things matter on real networks, both measured on the Kotlin build: addon hosts that publish an IPv6 address
//! nobody can reach (a connection attempt that takes 20 s to fail), and a system resolver that sometimes needs a second
//! or two per lookup. The client therefore sorts IPv4 first, remembers lookups for five minutes and gives up on a
//! connect after eight seconds. The system proxy settings are honoured.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

const DNS_TTL: Duration = Duration::from_secs(300);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(target_os = "macos")]
const USER_AGENT: &str = concat!("Nuvio/", env!("CARGO_PKG_VERSION"), " (macOS)");
#[cfg(not(target_os = "macos"))]
const USER_AGENT: &str = concat!("Nuvio/", env!("CARGO_PKG_VERSION"), " (Windows)");

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("the server answered {0}")]
    Status(u16),
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Request(String),
}

impl HttpError {
    /// Worth asking again: the failure says nothing about the request itself.
    pub fn is_transient(&self) -> bool {
        matches!(self, HttpError::Timeout | HttpError::Request(_)) || matches!(self, HttpError::Status(s) if *s >= 500 || *s == 429)
    }
}

#[derive(Clone, Default)]
struct CachingResolver {
    entries: Arc<Mutex<HashMap<String, (Instant, Vec<SocketAddr>)>>>,
}

impl Resolve for CachingResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let entries = self.entries.clone();
        Box::pin(async move {
            let remembered = entries
                .lock()
                .ok()
                .and_then(|map| map.get(&host).filter(|(at, _)| at.elapsed() < DNS_TTL).map(|(_, addrs)| addrs.clone()));
            if let Some(addrs) = remembered {
                return Ok(Box::new(addrs.into_iter()) as Addrs);
            }
            let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            // IPv4 first, so an unreachable IPv6 route is only ever tried as the fallback.
            addrs.sort_by_key(|a| a.is_ipv6());
            if let Ok(mut map) = entries.lock() {
                map.insert(host, (Instant::now(), addrs.clone()));
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// An answer whatever its status, for servers that explain a refusal in the body.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> Result<Self, HttpError> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(30))
            .http2_adaptive_window(true)
            .dns_resolver(Arc::new(CachingResolver::default()))
            .build()
            .map_err(|e| HttpError::Request(e.without_url().to_string()))?;
        Ok(Self { client })
    }

    /// GET `url` and return the body. Errors never carry the URL: addon URLs hold the user's token.
    pub async fn get(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, HttpError> {
        let response = self.client.get(url).timeout(timeout).send().await.map_err(map_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        response.bytes().await.map(|b| b.to_vec()).map_err(map_error)
    }

    /// POST `body` (JSON) to `url`. Unlike `get`, a 4xx or 5xx is a `Reply`, not an error: the account server says why it
    /// said no in the body. Errors never carry the URL.
    pub async fn post_json(&self, url: &str, headers: &[(&str, &str)], body: Vec<u8>, timeout: Duration) -> Result<Reply, HttpError> {
        let mut request = self.client.post(url).header("Content-Type", "application/json").timeout(timeout).body(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await.map_err(map_error)?;
        let status = response.status().as_u16();
        let body = response.bytes().await.map(|b| b.to_vec()).map_err(map_error)?;
        Ok(Reply { status, body })
    }

    /// Asks for the first byte of `url` and throws the answer away. Addons that resolve a debrid link when the stream is
    /// first requested spend seconds on it; doing that while the pointer rests on the stream means it is done by the time
    /// the player opens it.
    pub async fn warm(&self, url: &str) {
        let request = self.client.get(url).header("Range", "bytes=0-0").timeout(Duration::from_secs(20));
        if let Err(error) = request.send().await {
            tracing::debug!(error = %error.without_url(), "warm-up request failed");
        }
    }

    /// Like `get`, with one more try after a short pause when the failure looks temporary.
    pub async fn get_with_retry(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, HttpError> {
        match self.get(url, timeout).await {
            Err(error) if error.is_transient() => {
                tracing::debug!(%error, "request failed, retrying once");
                tokio::time::sleep(Duration::from_millis(400)).await;
                self.get(url, timeout).await
            }
            other => other,
        }
    }
}

fn map_error(error: reqwest::Error) -> HttpError {
    if error.is_timeout() {
        return HttpError::Timeout;
    }
    HttpError::Request(error.without_url().to_string())
}
