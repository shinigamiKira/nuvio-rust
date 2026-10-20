//! Logging to `%LOCALAPPDATA%\NuvioRust\logs\nuvio.log`, and to the console when the app was started from one.
//! `NUVIO_LOG` takes the usual filter syntax (`debug`, `nuvio_core=trace,info`).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

/// A log file bigger than this is started over, so it never grows without bound.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

pub fn log_dir() -> PathBuf {
    nuvio_core::paths::local_data_dir().join("logs")
}

#[derive(Clone)]
struct Sink {
    file: Option<Arc<Mutex<File>>>,
    console: bool,
}

struct SinkWriter(Sink);

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0.console {
            let _ = io::stderr().write_all(buf);
        }
        if let Some(file) = &self.0.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.write_all(buf);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(Ok(mut file)) = self.0.file.as_ref().map(|f| f.lock()) {
            let _ = file.flush();
        }
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Sink {
    type Writer = SinkWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SinkWriter(self.clone())
    }
}

pub fn init() {
    let console = attach_parent_console();
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("nuvio.log");
    let too_big = std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(!too_big)
        .truncate(too_big)
        .open(&path)
        .ok()
        .map(|f| Arc::new(Mutex::new(f)));
    let filter = EnvFilter::try_from_env("NUVIO_LOG").unwrap_or_else(|_| EnvFilter::new("info,wgpu=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true)
        .with_writer(Sink { file, console })
        .init();
}

/// The exe is a GUI program, so Windows gives it no console. When it was started from a terminal, borrow that one.
fn attach_parent_console() -> bool {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: no pointers involved; failure just means there is no parent console.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}
