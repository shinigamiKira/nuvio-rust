//! Drives a real libmpv without a window: a synthetic video through the null outputs. Needs libmpv where the app looks for
//! it (Windows: scripts/fetch-libmpv.ps1; macOS: `brew install mpv`, or `NUVIO_LIBMPV`). Without it the tests say so and
//! pass, so a fresh checkout still has a green `cargo test`; set `NUVIO_REQUIRE_LIBMPV=1` to make that a failure.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nuvio_mpv::{Event, Mpv, Observe, Value};

/// Windows keeps the DLL in the checkout, where a test binary does not look on its own.
fn point_at_the_checkout() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../libs/windows-x64/libmpv-2.dll");
    if cfg!(windows) && path.exists() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        // SAFETY: set once, before any test of this binary loads the library.
        ONCE.call_once(|| unsafe { std::env::set_var("NUVIO_LIBMPV", &path) });
    }
}

fn player(wakeups: Arc<AtomicUsize>) -> Option<Mpv> {
    point_at_the_checkout();
    if let Err(error) = nuvio_mpv::available() {
        assert!(std::env::var_os("NUVIO_REQUIRE_LIBMPV").is_none(), "{error}");
        eprintln!("libmpv not available, skipping: {error}");
        return None;
    }
    let options = [("vo", "null"), ("ao", "null"), ("idle", "yes"), ("keep-open", "yes"), ("config", "no"), ("terminal", "no")];
    Some(Mpv::new(&options, move || {
        wakeups.fetch_add(1, Ordering::SeqCst);
    })
    .expect("mpv starts"))
}

fn wait_for(mpv: &Mpv, what: &str, mut done: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let mut batch = Vec::new();
        mpv.poll_events(&mut batch);
        let found = batch.iter().any(&mut done);
        seen.extend(batch);
        if found {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

#[test]
fn plays_a_generated_video_and_reports_its_state() {
    let wakeups = Arc::new(AtomicUsize::new(0));
    let Some(mpv) = player(wakeups.clone()) else { return };
    assert!(mpv.version().starts_with("2."), "client API version {}", mpv.version());
    mpv.observe("time-pos", Observe::Double);
    mpv.observe("duration", Observe::Double);
    mpv.observe("pause", Observe::Flag);

    mpv.command(&["loadfile", "av://lavfi:testsrc=duration=3:size=320x240:rate=25", "replace"]).unwrap();
    wait_for(&mpv, "the file to load", |e| matches!(e, Event::FileLoaded));
    assert!(wakeups.load(Ordering::SeqCst) > 0, "the wakeup callback never ran");

    let events = wait_for(&mpv, "time-pos to pass half a second", |e| matches!(e, Event::Property { name, value } if name == "time-pos" && value.as_f64().is_some_and(|t| t > 0.5)));
    assert!(events.iter().any(|e| matches!(e, Event::Property { name, value: Value::Double(_) } if name == "duration")) || mpv.get_f64("duration").is_some());

    mpv.set_bool("pause", true).unwrap();
    wait_for(&mpv, "the pause flag", |e| matches!(e, Event::Property { name, value: Value::Flag(true) } if name == "pause"));
    assert_eq!(mpv.get_bool("pause"), Some(true));

    // Structured properties come back as JSON the app can parse.
    let tracks = mpv.get_str("track-list").expect("a track list");
    assert!(tracks.starts_with('[') && tracks.contains("\"video\""), "{tracks}");
}

#[test]
fn a_file_that_does_not_exist_ends_with_an_error() {
    let wakeups = Arc::new(AtomicUsize::new(0));
    let Some(mpv) = player(wakeups) else { return };
    mpv.command(&["loadfile", "/this/file/does/not/exist.mkv", "replace"]).unwrap();
    let events = wait_for(&mpv, "the end of the failed file", |e| matches!(e, Event::EndFile { .. }));
    assert!(events.iter().any(|e| matches!(e, Event::EndFile { error: Some(_), .. })), "{events:?}");
}

#[test]
fn a_header_with_a_comma_reaches_the_server_whole() {
    use std::io::{BufRead, BufReader, Write};
    let Some(mpv) = player(Arc::new(AtomicUsize::new(0))) else { return };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/video.mkv", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut lines = Vec::new();
        for line in BufReader::new(connection.try_clone().unwrap()).lines() {
            let line = line.unwrap();
            if line.is_empty() {
                break;
            }
            lines.push(line);
        }
        let _ = connection.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        lines
    });
    mpv.set_list("http-header-fields", &["User-Agent: Mozilla/5.0 (X, like Gecko)".into(), "X-Second: yes".into()]).unwrap();
    mpv.command(&["loadfile", &url, "replace"]).unwrap();
    wait_for(&mpv, "the request to be answered", |e| matches!(e, Event::EndFile { .. }));
    let lines = server.join().unwrap();
    assert!(lines.iter().any(|l| l == "User-Agent: Mozilla/5.0 (X, like Gecko)"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "X-Second: yes"), "{lines:?}");
}

/// Why the app keeps its own pause flag across a reload, and lifts the pause when it reopens a stream that was cut off.
#[test]
fn the_pause_at_the_end_of_a_file_carries_over_to_the_next_load() {
    let Some(mpv) = player(Arc::new(AtomicUsize::new(0))) else { return };
    mpv.observe("eof-reached", Observe::Flag);
    mpv.command(&["loadfile", "av://lavfi:testsrc=duration=0.4:size=64x64:rate=25", "replace"]).unwrap();
    wait_for(&mpv, "the end of the file", |e| matches!(e, Event::Property { name, value: Value::Flag(true) } if name == "eof-reached"));
    assert_eq!(mpv.get_bool("pause"), Some(true));
    mpv.command(&["loadfile", "av://lavfi:testsrc=duration=3:size=64x64:rate=25", "replace"]).unwrap();
    wait_for(&mpv, "the next file", |e| matches!(e, Event::FileLoaded));
    assert_eq!(mpv.get_bool("pause"), Some(true));
}

/// A build of FFmpeg without a filter the app asks for (libplacebo, for Dolby Vision) does not refuse the file: the option
/// is rejected and the file plays without it. The player finds that out from the colour matrix of the picture.
#[test]
fn a_filter_the_library_does_not_have_is_left_out() {
    let Some(mpv) = player(Arc::new(AtomicUsize::new(0))) else { return };
    mpv.command(&["loadfile", "av://lavfi:testsrc=duration=1:size=64x64:rate=25", "replace", "-1", "vf=nuvio-no-such-filter"]).unwrap();
    let events = wait_for(&mpv, "the file to start", |e| matches!(e, Event::PlaybackRestart));
    assert!(events.iter().any(|e| matches!(e, Event::FileLoaded)), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Log { text, .. } if text.contains("nuvio-no-such-filter"))), "{events:?}");
}
