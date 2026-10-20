//! Drives a real libmpv without a window: a synthetic video through the null outputs. Needs `libs/windows-x64/libmpv-2.dll`
//! (scripts/fetch-libmpv.ps1); without it the tests say so and pass, so a fresh checkout still has a green `cargo test`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nuvio_mpv::{Event, Mpv, Observe, Value};

fn library() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../libs/windows-x64/libmpv-2.dll");
    path.exists().then_some(path)
}

fn player(wakeups: Arc<AtomicUsize>) -> Option<Mpv> {
    let path = library().or_else(|| {
        eprintln!("libmpv-2.dll not staged, skipping");
        None
    })?;
    // SAFETY: set before any other thread of this test binary reads the environment.
    unsafe { std::env::set_var("NUVIO_LIBMPV", &path) };
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
    mpv.command(&["loadfile", "C:\\this\\file\\does\\not\\exist.mkv", "replace"]).unwrap();
    let events = wait_for(&mpv, "the end of the failed file", |e| matches!(e, Event::EndFile { .. }));
    assert!(events.iter().any(|e| matches!(e, Event::EndFile { error: Some(_), .. })), "{events:?}");
}
