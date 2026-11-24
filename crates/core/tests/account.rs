//! Sign-in against a small local account server: what is sent, what comes back, and the fallback address.

use std::sync::{Arc, Mutex};
use std::thread;

use nuvio_core::account::{self, AuthError, Server, SignUp};
use nuvio_core::http::Http;
use tiny_http::{Header, Response, Server as Listener};

/// What the server saw: the path, the `apikey` header, the bearer token and the body.
type Seen = Arc<Mutex<Vec<(String, Option<String>, Option<String>, String)>>>;

fn json(status: u16, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body).with_status_code(status).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn header(request: &tiny_http::Request, name: &str) -> Option<String> {
    request.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.to_string())
}

/// Accepts `me@example.test` / `right-password`; everything else is refused the way GoTrue does.
fn start() -> (String, Seen) {
    let listener = Listener::http("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.server_addr().to_ip().unwrap().port());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    thread::spawn(move || {
        for mut request in listener.incoming_requests() {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let path = request.url().to_owned();
            log.lock().unwrap().push((path.clone(), header(&request, "apikey"), header(&request, "Authorization"), body.clone()));
            let response = match path.as_str() {
                "/auth/v1/token?grant_type=password" if body.contains("right-password") => json(
                    200,
                    r#"{"access_token":"access-1","token_type":"bearer","expires_in":3600,"refresh_token":"refresh-1","user":{"id":"user-1","email":"me@example.test"}}"#,
                ),
                "/auth/v1/token?grant_type=password" => json(400, r#"{"error":"invalid_grant","error_description":"Invalid login credentials"}"#),
                "/auth/v1/token?grant_type=refresh_token" if body.contains("refresh-1") => json(
                    200,
                    r#"{"access_token":"access-2","expires_in":3600,"refresh_token":"refresh-2","user":{"id":"user-1","email":"me@example.test"}}"#,
                ),
                "/auth/v1/token?grant_type=refresh_token" => json(400, r#"{"error":"invalid_grant","error_description":"Invalid Refresh Token: Already Used"}"#),
                "/auth/v1/signup" => json(200, r#"{"id":"user-2","email":"new@example.test","confirmation_sent_at":"2026-10-21T00:00:00Z"}"#),
                "/auth/v1/logout" => Response::from_string("").with_status_code(204),
                _ => json(404, "{}"),
            };
            let _ = request.respond(response);
        }
    });
    (base, seen)
}

fn server(base: &str) -> Server {
    Server { url: base.to_owned(), key: "publishable-key".into(), fallback_url: None }
}

#[tokio::test]
async fn signs_in_refreshes_and_signs_out() {
    let (base, seen) = start();
    let http = Http::new().unwrap();
    let session = account::sign_in(&http, &server(&base), "me@example.test", "right-password").await.unwrap();
    assert_eq!((session.user_id.as_str(), session.email.as_deref()), ("user-1", Some("me@example.test")));

    let renewed = account::refresh(&http, &session).await.unwrap();
    assert_eq!((renewed.access_token.as_str(), renewed.refresh_token.as_str()), ("access-2", "refresh-2"));

    account::sign_out(&http, &renewed).await;

    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|(_, key, ..)| key.as_deref() == Some("publishable-key")), "every call carries the key");
    assert!(seen[0].3.contains("me@example.test"), "the sign-in sends the address");
    let logout = seen.last().unwrap();
    assert_eq!(logout.0, "/auth/v1/logout");
    assert_eq!(logout.2.as_deref(), Some("Bearer access-2"));
}

#[tokio::test]
async fn a_wrong_password_comes_back_in_the_servers_words() {
    let (base, _) = start();
    let error = account::sign_in(&Http::new().unwrap(), &server(&base), "me@example.test", "wrong").await.unwrap_err();
    assert_eq!(error.to_string(), "Invalid login credentials");
    assert!(error.ends_session(), "a refused request is final, unlike a network error");
}

#[tokio::test]
async fn a_used_refresh_token_ends_the_session() {
    let (base, _) = start();
    let http = Http::new().unwrap();
    let mut session = account::sign_in(&http, &server(&base), "me@example.test", "right-password").await.unwrap();
    session.refresh_token = "refresh-0".into();
    assert!(account::refresh(&http, &session).await.unwrap_err().ends_session());
}

#[tokio::test]
async fn a_sign_up_that_needs_the_mailed_link_is_not_a_session() {
    let (base, _) = start();
    let outcome = account::sign_up(&Http::new().unwrap(), &server(&base), "new@example.test", "a-password").await.unwrap();
    assert!(matches!(outcome, SignUp::ConfirmEmail));
}

#[tokio::test]
async fn falls_back_to_the_second_address_when_the_first_is_down() {
    let (fallback, seen) = start();
    // Nothing listens on the first address.
    let dead = Listener::http("127.0.0.1:0").unwrap();
    let dead_url = format!("http://127.0.0.1:{}", dead.server_addr().to_ip().unwrap().port());
    drop(dead);
    let server = Server { url: dead_url, key: "publishable-key".into(), fallback_url: Some(fallback) };
    let session = account::sign_in(&Http::new().unwrap(), &server, "me@example.test", "right-password").await.unwrap();
    assert_eq!(session.user_id, "user-1");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unreachable_server_is_unavailable_not_refused() {
    let dead = Listener::http("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", dead.server_addr().to_ip().unwrap().port());
    drop(dead);
    let error = account::sign_in(&Http::new().unwrap(), &Server { url, key: "k".into(), fallback_url: None }, "a@b.c", "x").await.unwrap_err();
    assert!(matches!(error, AuthError::Unavailable(_)));
    assert!(!error.ends_session());
}

/// Against the real server, for a person to run by hand: `cargo test -p nuvio-core --test account -- --ignored`. It signs
/// in with an address that does not exist, which is refused without creating anything.
#[tokio::test]
#[ignore = "needs the network"]
async fn the_official_server_discovers_and_refuses_a_stranger() {
    let http = Http::new().unwrap();
    let server = Server::discover(&http, account::OFFICIAL_SERVER).await.unwrap();
    assert_eq!(server.host(), "api.nuvio.tv");
    let error = account::sign_in(&http, &server, "nobody-at-all@example.invalid", "not-a-real-password").await.unwrap_err();
    println!("the server said: {error}");
    assert!(matches!(error, AuthError::Rejected { .. }), "{error:?}");
}
