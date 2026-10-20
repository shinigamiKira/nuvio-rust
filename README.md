# Nuvio for Windows, in Rust

A native Windows port of the Nuvio desktop app, written in Rust. No JVM and no web view: the interface is drawn with
[egui](https://github.com/emilk/egui) on OpenGL, and video is played by [libmpv](https://mpv.io) into the same window, so the
player controls are ordinary widgets on top of the picture.

It is an unofficial port of [Nuvio](https://github.com/NuvioMedia/NuvioMobile) (0.6.2, the `cmp-rewrite` branch) and keeps its
look: the floating glass navigation pill, the rotating hero, shelves of posters, the stream list with its filters. It does not
share code with the Kotlin build; the two can be installed side by side (`%APPDATA%\NuvioRust` against `%APPDATA%\Nuvio`).

What is ported and what is not is in [PORTING.md](PORTING.md).

## Lighter and faster, and where it is not

Measured on the machine the Kotlin port was tuned on (Windows 11, i5-11300H, RTX 3060 Laptop, 1080p display at 100 %, on mains
power), release builds, the installed app. The Kotlin start-up and memory figures are from its own notes
(`desktopApp/PERF.md`, taken on this laptop on an earlier day); the playback rows were measured side by side on the same clips,
the Kotlin build through its player harness (the same libmpv, its Direct3D 11 renderer, zero-copy D3D11VA decoding).

| | Kotlin build | This build |
| --- | --- | --- |
| Installer | 167.5 MB | 42.2 MB |
| Installed | 357 MB | 135 MB (libmpv is 122 of them) |
| Window with content, warm cache | first frame 1.8-1.9 s, first image 3.2 s | first frame 0.23 s, the shelves of the last session in the same frame |
| Window with content, empty cache | | first frame 0.25 s, first shelf 0.7-1.7 s (the network) |
| Working set on Home, idle | 428-439 MB | 126-185 MB |
| CPU on Home, idle | | 0.08 % of a core: no frames are drawn |
| CPU, 2160p HEVC 10-bit 30 fps | 0.29 cores (median) | 0.11 cores (0.08-0.15 over four runs) |
| CPU, 2160p H.264 30 fps, 41 Mbit/s | 0.35 cores | 0.07 cores |
| CPU, 1080p H.264 30 fps | 0.27 cores | 0.06 cores |
| Two minutes of 4K playback | | 792 to 800 MB working set, handle and thread counts flat |

The Kotlin playback figures are its player harness (no application window around it), so the comparison favours it if anything.
Both play through the same libmpv; the difference is where the frames go. This build draws one window frame per video frame, and
NVDEC hands its frames to OpenGL without a copy. Without NVIDIA the decoder falls back to copy-back decoding, which measured about 90 % of
a core on the 2160p HEVC clip here (the Kotlin build's D3D11 renderer can keep D3D11VA zero-copy on any GPU; through OpenGL only NVDEC can).
Memory is the working set; with NVDEC the process also holds about 2 GB of address space for CUDA, which is not physical memory.

Where the start-up and memory difference comes from, none of it clever:

* a single exe has no runtime to warm up, and the last session's shelves are read from the disk cache and shown before the network
  answers;
* posters are decoded off the UI thread and shrunk to the size they are drawn at (a Lanczos resize), then cached;
* nothing repaints unless something changed, and egui's `request_repaint()` draws two frames per call, so everything that wakes the
  window (video frames, finished downloads, animations) asks for one frame after a short delay instead; that alone took a 2160p
  file from 36 % to 12 % of a core;
* NVDEC hands decoded frames to OpenGL without a copy.

## Build

Needs Windows 10/11 x64 and a Rust toolchain (stable). Either host works: `x86_64-pc-windows-msvc`, or `x86_64-pc-windows-gnu` with
a MinGW-w64 GCC on `PATH` (that is what this repository was built with; it needs no administrator rights and no Visual Studio).

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-libmpv.ps1    # libmpv-2.dll into libs\windows-x64 (needs 7-Zip)
cargo run -p nuvio --profile fast                                    # release code without the link time cost
cargo test --workspace
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1 # dist\Nuvio-Rust-Setup-<version>.exe (needs Inno Setup 6)
```

The repository sits in a OneDrive folder on the machine it was written on, so `target` is redirected with a per-user
`~/.cargo/config.toml` (`[build] target-dir = "C:/nuvio-build/cargo-target"`). Keep build output out of synced folders.

## Layout

```
crates/core   addon protocol (manifests, catalogs, meta, streams, subtitles), the HTTP client, the stores, facet filtering
crates/mpv    libmpv loaded at run time, events, and an OpenGL render context
crates/app    the egui application: screens, player, torrent engine, the glass blur, the installer resources
installer     Inno Setup script
scripts       fetch-libmpv.ps1, build-installer.ps1
assets        JetBrains Sans and the Material Icons font
```

Things a reader should know before changing the code:

* mpv's getters wait for the player core, and the core waits for the video output, which only starts when the thread that renders
  calls `render`. Anything the UI needs from mpv arrives as an observed property; the getters are for other threads and tests.
* The file may only be loaded after the render context exists. The context can only be created while the window's GL context is
  current, so the first paint creates it and the load follows.
* Dolby Vision profile 5 is converted with a libplacebo filter (`gpu-next` is not available through libmpv's render API), which
  needs the copy-back NVDEC decoder; the result is checked and the file reopened without the filter when mpv dropped it.
* Addon URLs and stream URLs carry the user's token. Errors never include them (`HttpError`), and everything that reaches the
  log passes through `nuvio_core::redact`.

## Testing and the hooks the tests use

`cargo test --workspace` runs the parsers, the facet rules, the stores, the HTTP client against a local mock addon, the torrent file
choice, the image resizing, and a real libmpv through its null outputs (skipped when the DLL is not staged).

The app has command line options for driving a screen without anyone at the mouse, used to take the pictures this was checked with:

```
nuvio --data-dir <dir> --open <home | search:<text> | library | settings[:page] | detail:<type>:<id> | streams:<type>:<video id>>
      --select <n>            play the n-th stream of the list
      --play <url or magnet>  (or the NUVIO_PLAY_URL environment variable)
      --screenshot <png> --screenshot-after <seconds>
      --mouse x,y             a pointer that keeps moving by a pixel, so hover states and the player controls show
      --key <seconds>:<Key>   --click <seconds>:<x>,<y>   --scale <factor>   --size <w>x<h>   --fullscreen
```

`NUVIO_LOG` takes a log filter, `NUVIO_MPV_LOG=v` turns on mpv's own log, `NUVIO_PERF=1` logs the frames drawn per second,
`NUVIO_MPV_HWDEC` forces a decoder, and `NUVIO_TEST_ADDON_URL` adds an addon for that run only (it is never written to disk).

## Licenses

Nuvio is GPL-3.0 and so is this port (`LICENSE`). Bundled: libmpv with FFmpeg and libass (GPL build), JetBrains Sans (OFL),
Material Icons (Apache-2.0), and the Rust crates listed in `Cargo.lock` (egui, librqbit, reqwest, tokio and others, under MIT or
Apache-2.0).
