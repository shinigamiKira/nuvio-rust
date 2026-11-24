//! The Nuvio account: sign in, sign up, keep the session fresh, sign out.
//!
//! The account server is a Supabase project, so this speaks its auth API (`/auth/v1/...`). Its address and its publishable
//! key are not built into this program: the official server publishes both in a discovery document
//! (`/.well-known/nuvio`, the same one a self-hosted server serves), and a different server can be named with
//! `NUVIO_SUPABASE_URL` and `NUVIO_SUPABASE_ANON_KEY` or a `server.json` next to the settings. The Kotlin build reads the
//! same file in the same shape.
//!
//! Tokens never reach a log or an error message.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::{Http, HttpError, Reply};
use crate::paths;
use crate::store::unix_seconds;

pub const OFFICIAL_SERVER: &str = "https://api.nuvio.tv";
const DISCOVERY_PATH: &str = "/.well-known/nuvio";
const TIMEOUT: Duration = Duration::from_secs(20);
/// A session is renewed this long before its access token runs out.
pub const REFRESH_MARGIN_SECS: u64 = 120;

/// A server that answers like this is down or being redeployed, which is when the fallback address is tried.
fn origin_failed(status: u16) -> bool {
    matches!(status, 502..=504 | 520..=526)
}

/// Where an account lives: the server's address, the key that lets a client talk to it, and a second address to try when
/// the first one is down.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
    pub key: String,
    pub fallback_url: Option<String>,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server").field("url", &self.url).finish_non_exhaustive()
    }
}

impl Server {
    /// The address to show: `api.nuvio.tv`.
    pub fn host(&self) -> String {
        self.url.split("://").nth(1).unwrap_or(&self.url).split('/').next().unwrap_or_default().to_owned()
    }

    /// Where to sign in: the environment, then `server.json`, then the official server's own discovery document.
    pub async fn resolve(http: &Http) -> Result<Server, AuthError> {
        if let Some(server) = Self::from_environment().or_else(Self::from_file) {
            return Ok(server);
        }
        Self::discover(http, OFFICIAL_SERVER).await
    }

    fn from_environment() -> Option<Server> {
        let url = std::env::var("NUVIO_SUPABASE_URL").ok()?;
        let key = std::env::var("NUVIO_SUPABASE_ANON_KEY").ok()?;
        Self::configured(&url, &key, std::env::var("NUVIO_SUPABASE_FALLBACK_URL").ok().as_deref())
    }

    /// `{"url": "...", "key": "...", "fallbackUrl": "..."}`.
    fn from_file() -> Option<Server> {
        let bytes = std::fs::read(paths::roaming_data_dir().join("server.json")).ok()?;
        Self::from_json(&bytes)
    }

    fn from_json(bytes: &[u8]) -> Option<Server> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let text = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
        Self::configured(&text("url")?, &text("key")?, text("fallbackUrl").as_deref())
    }

    fn configured(url: &str, key: &str, fallback: Option<&str>) -> Option<Server> {
        let (url, key) = (url.trim().trim_end_matches('/'), key.trim());
        if url.is_empty() || key.is_empty() {
            return None;
        }
        let fallback_url = fallback.map(|f| f.trim().trim_end_matches('/')).filter(|f| !f.is_empty()).map(str::to_owned);
        Some(Server { url: url.to_owned(), key: key.to_owned(), fallback_url })
    }

    /// Reads the discovery document of the server at `origin`.
    pub async fn discover(http: &Http, origin: &str) -> Result<Server, AuthError> {
        let url = format!("{}{DISCOVERY_PATH}", origin.trim_end_matches('/'));
        let body = http.get(&url, TIMEOUT).await.map_err(|e| AuthError::Unavailable(describe(&e)))?;
        Self::from_discovery(&body)
    }

    fn from_discovery(body: &[u8]) -> Result<Server, AuthError> {
        #[derive(Deserialize)]
        struct Document {
            version: i64,
            service: String,
            backend_url: String,
            publishable_key: String,
            #[serde(default)]
            capabilities: Capabilities,
        }
        #[derive(Deserialize, Default)]
        struct Capabilities {
            #[serde(default)]
            email_password_auth: bool,
        }
        let document: Document = serde_json::from_slice(body).map_err(|_| AuthError::Unexpected)?;
        if document.version != 1 || !document.service.eq_ignore_ascii_case("nuvio") {
            return Err(AuthError::Unexpected);
        }
        if !document.capabilities.email_password_auth {
            return Err(AuthError::Rejected { status: 0, message: "This server does not offer sign-in with an email address.".into() });
        }
        let secure = document.backend_url.starts_with("https://");
        let local = document.backend_url.starts_with("http://localhost") || document.backend_url.starts_with("http://127.");
        if !secure && !local {
            return Err(AuthError::Unexpected);
        }
        Self::configured(&document.backend_url, &document.publishable_key, None).ok_or(AuthError::Unexpected)
    }
}

/// A signed-in user. Kept in `account.json`, so the next start is already signed in.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    pub user_id: String,
    pub email: Option<String>,
    pub server: Server,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("user_id", &self.user_id).field("email", &self.email).field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

impl Session {
    pub fn needs_refresh(&self, now: u64) -> bool {
        now + REFRESH_MARGIN_SECS >= self.expires_at
    }

    /// Seconds until a refresh is due, 0 when it is overdue.
    pub fn refresh_in(&self, now: u64) -> u64 {
        self.expires_at.saturating_sub(REFRESH_MARGIN_SECS).saturating_sub(now)
    }

    fn from_token_body(body: &[u8], server: &Server, now: u64) -> Result<Session, AuthError> {
        #[derive(Deserialize)]
        struct Tokens {
            access_token: String,
            refresh_token: String,
            expires_in: Option<u64>,
            expires_at: Option<u64>,
            user: Option<User>,
        }
        #[derive(Deserialize)]
        struct User {
            id: String,
            email: Option<String>,
        }
        let tokens: Tokens = serde_json::from_slice(body).map_err(|_| AuthError::Unexpected)?;
        let user = tokens.user.ok_or(AuthError::Unexpected)?;
        let expires_at = tokens.expires_at.or(tokens.expires_in.map(|s| now + s)).unwrap_or(now + 3600);
        Ok(Session {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at,
            user_id: user.id,
            email: user.email.filter(|e| !e.is_empty()),
            server: server.clone(),
        })
    }
}

/// What is kept about the account between runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub session: Option<Session>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The server said no, in words meant for the person (wrong password, address already registered...).
    #[error("{message}")]
    Rejected { status: u16, message: String },
    /// The server could not be reached, or is having a bad moment.
    #[error("{0}")]
    Unavailable(String),
    #[error("The account server sent an answer this app does not understand.")]
    Unexpected,
}

impl AuthError {
    /// The session is gone for good (a refresh token that was revoked or already used), as opposed to a bad network.
    pub fn ends_session(&self) -> bool {
        matches!(self, AuthError::Rejected { status: 400 | 401 | 403, .. })
    }
}

fn describe(error: &HttpError) -> String {
    match error {
        HttpError::Timeout => "The account server did not answer in time.".into(),
        HttpError::Status(status) => format!("The account server answered {status}."),
        HttpError::Request(_) => "Could not reach the account server. Check the connection and try again.".into(),
    }
}

/// The words of a refusal. GoTrue has used `error_description`, `msg` and `message` over its versions.
fn refusal(status: u16, body: &[u8]) -> AuthError {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let message = ["error_description", "msg", "message", "error"]
        .iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|text| !text.is_empty())
        .map(str::to_owned);
    match message {
        Some(message) if status < 500 => AuthError::Rejected { status, message },
        _ if status >= 500 => AuthError::Unavailable(format!("The account server is having trouble ({status}). Try again in a moment.")),
        _ => AuthError::Rejected { status, message: format!("The account server refused the request ({status}).") },
    }
}

/// One call to the auth API, with the fallback address tried when the first one looks down.
async fn call(http: &Http, server: &Server, path: &str, bearer: Option<&str>, body: Value) -> Result<Reply, AuthError> {
    let payload = serde_json::to_vec(&body).map_err(|_| AuthError::Unexpected)?;
    let authorization = bearer.map(|token| format!("Bearer {token}"));
    let mut headers: Vec<(&str, &str)> = vec![("apikey", server.key.as_str())];
    if let Some(value) = &authorization {
        headers.push(("Authorization", value));
    }
    let attempt = |base: &str| {
        let url = format!("{base}/auth/v1/{path}");
        let payload = payload.clone();
        let headers = headers.clone();
        async move { http.post_json(&url, &headers, payload, TIMEOUT).await }
    };
    let first = attempt(&server.url).await;
    let down = match &first {
        Ok(reply) => origin_failed(reply.status),
        Err(error) => error.is_transient(),
    };
    let result = match (&server.fallback_url, down) {
        (Some(fallback), true) if *fallback != server.url => attempt(fallback).await.or(first),
        _ => first,
    };
    result.map_err(|e| AuthError::Unavailable(describe(&e)))
}

pub async fn sign_in(http: &Http, server: &Server, email: &str, password: &str) -> Result<Session, AuthError> {
    let reply = call(http, server, "token?grant_type=password", None, json!({ "email": email, "password": password })).await?;
    match reply.status {
        200..=299 => Session::from_token_body(&reply.body, server, unix_seconds()),
        status => Err(refusal(status, &reply.body)),
    }
}

pub enum SignUp {
    SignedIn(Session),
    /// The server wants the address confirmed first: a link was mailed.
    ConfirmEmail,
}

pub async fn sign_up(http: &Http, server: &Server, email: &str, password: &str) -> Result<SignUp, AuthError> {
    let reply = call(http, server, "signup", None, json!({ "email": email, "password": password })).await?;
    match reply.status {
        // With confirmation on, the answer is the new user and no tokens.
        200..=299 => match Session::from_token_body(&reply.body, server, unix_seconds()) {
            Ok(session) => Ok(SignUp::SignedIn(session)),
            Err(_) => Ok(SignUp::ConfirmEmail),
        },
        status => Err(refusal(status, &reply.body)),
    }
}

/// Trades the refresh token for a new access token (and a new refresh token: each one works once).
pub async fn refresh(http: &Http, session: &Session) -> Result<Session, AuthError> {
    let reply = call(http, &session.server, "token?grant_type=refresh_token", None, json!({ "refresh_token": session.refresh_token })).await?;
    match reply.status {
        200..=299 => Session::from_token_body(&reply.body, &session.server, unix_seconds()),
        status => Err(refusal(status, &reply.body)),
    }
}

/// Ends the session on the server. Whatever the answer, the caller forgets it locally.
pub async fn sign_out(http: &Http, session: &Session) {
    if let Err(error) = call(http, &session.server, "logout", Some(&session.access_token), json!({})).await {
        tracing::debug!(%error, "sign-out was not delivered to the server");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISCOVERY: &str = r#"{"version":1,"service":"nuvio","self_hosted":true,"backend_url":"https://api.nuvio.tv","publishable_key":"anon-key","capabilities":{"email_password_auth":true,"tv_login":true,"watch_party":true}}"#;

    fn server() -> Server {
        Server { url: "https://api.example.test".into(), key: "k".into(), fallback_url: None }
    }

    #[test]
    fn reads_the_discovery_document() {
        let server = Server::from_discovery(DISCOVERY.as_bytes()).unwrap();
        assert_eq!(server.url, "https://api.nuvio.tv");
        assert_eq!(server.key, "anon-key");
        assert_eq!(server.host(), "api.nuvio.tv");
    }

    #[test]
    fn refuses_a_discovery_document_it_cannot_trust() {
        let wrong_service = DISCOVERY.replace("\"nuvio\"", "\"other\"");
        assert!(Server::from_discovery(wrong_service.as_bytes()).is_err());
        let no_password_sign_in = DISCOVERY.replace("\"email_password_auth\":true", "\"email_password_auth\":false");
        assert!(Server::from_discovery(no_password_sign_in.as_bytes()).is_err());
        let plain_http = DISCOVERY.replace("https://api.nuvio.tv", "http://api.nuvio.tv");
        assert!(Server::from_discovery(plain_http.as_bytes()).is_err());
        assert!(Server::from_discovery(b"<html>").is_err());
    }

    #[test]
    fn reads_server_json() {
        let server = Server::from_json(br#"{"url":"https://a.test/","key":" k ","fallbackUrl":"https://b.test/"}"#).unwrap();
        assert_eq!((server.url.as_str(), server.key.as_str(), server.fallback_url.as_deref()), ("https://a.test", "k", Some("https://b.test")));
        assert!(Server::from_json(br#"{"url":"https://a.test"}"#).is_none());
        assert!(Server::from_json(br#"{"url":"","key":"k"}"#).is_none());
    }

    #[test]
    fn reads_a_token_answer() {
        let body = br#"{"access_token":"a","token_type":"bearer","expires_in":3600,"refresh_token":"r","user":{"id":"u1","email":"me@example.test"}}"#;
        let session = Session::from_token_body(body, &server(), 1000).unwrap();
        assert_eq!((session.access_token.as_str(), session.refresh_token.as_str()), ("a", "r"));
        assert_eq!(session.expires_at, 4600);
        assert_eq!(session.email.as_deref(), Some("me@example.test"));
        // An absolute expiry in the answer wins over the relative one.
        let body = br#"{"access_token":"a","expires_in":3600,"expires_at":9000,"refresh_token":"r","user":{"id":"u1"}}"#;
        assert_eq!(Session::from_token_body(body, &server(), 1000).unwrap().expires_at, 9000);
        // A sign-up that waits for the mailed link has a user and no tokens.
        assert!(Session::from_token_body(br#"{"id":"u1","email":"me@example.test"}"#, &server(), 1000).is_err());
    }

    #[test]
    fn says_when_a_refresh_is_due() {
        let mut session = Session::from_token_body(br#"{"access_token":"a","expires_in":3600,"refresh_token":"r","user":{"id":"u"}}"#, &server(), 1000).unwrap();
        assert!(!session.needs_refresh(1000));
        assert_eq!(session.refresh_in(1000), 3600 - REFRESH_MARGIN_SECS);
        assert!(session.needs_refresh(4600 - REFRESH_MARGIN_SECS));
        session.expires_at = 10;
        assert_eq!(session.refresh_in(1000), 0);
    }

    #[test]
    fn explains_a_refusal_in_the_servers_words() {
        let message = |error: AuthError| error.to_string();
        assert_eq!(message(refusal(400, br#"{"error":"invalid_grant","error_description":"Invalid login credentials"}"#)), "Invalid login credentials");
        assert_eq!(message(refusal(422, br#"{"code":422,"error_code":"weak_password","msg":"Password should be at least 6 characters."}"#)), "Password should be at least 6 characters.");
        assert_eq!(message(refusal(429, br#"{"message":"email rate limit exceeded"}"#)), "email rate limit exceeded");
        assert!(message(refusal(400, b"")).contains("400"));
        assert!(matches!(refusal(503, b"upstream down"), AuthError::Unavailable(_)));
    }

    #[test]
    fn only_a_refused_refresh_ends_the_session() {
        assert!(refusal(400, br#"{"error_description":"Invalid Refresh Token: Already Used"}"#).ends_session());
        assert!(!refusal(429, br#"{"message":"slow down"}"#).ends_session());
        assert!(!AuthError::Unavailable("offline".into()).ends_session());
    }

    #[test]
    fn debug_output_has_no_tokens() {
        let session = Session { access_token: "secret-access".into(), refresh_token: "secret-refresh".into(), expires_at: 1, user_id: "u".into(), email: None, server: Server { url: "https://a".into(), key: "secret-key".into(), fallback_url: None } };
        let text = format!("{session:?}");
        assert!(!text.contains("secret"));
    }
}
