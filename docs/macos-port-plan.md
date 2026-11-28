# Nuvio (Rust) on macOS: port, UI fixes, tests, release

## Status

Ported and packaged for one Mac mini (Apple M6, macOS 27.0). Decisions taken: no developer account (ad hoc signature,
Gatekeeper overridden by hand), Apple Silicon only, the Windows UI kept. This replaces D1-D3 and D6 below, and the
signing, notarization and release steps that depend on them.

| Phase | State |
| --- | --- |
| 0 CI | not done |
| 1 Build, data folders | done |
| 2 Playback | done for H.264 and HEVC 10-bit, file and HTTPS. Not tried: torrents, Dolby Vision, AV1, HDR, audible output |
| 3 macOS integration | done, except import from NuvioDesktop and Keychain |
| 4 UI bugs | U1-U16 changed. Reproduced before and checked after on screen or at the server: U1, U2, U3, U5, U7, U8, U10, U11, U13. Unit or libmpv tests only: U4, U6. Not checked at run time: U9, U12, U15, U16. U14 checked on macOS with `pmset` |
| 5 Packaging | done (`scripts/build-macos.sh`) |
| 6 Release | partly: the port is on the `macos` branch and the DMG is in `releases/`. No CI, tag or GitHub release |

Also found and fixed during the port: a stream went unattended while the window was minimized or covered; the packaged
app refused every HTTPS stream without Homebrew's certificate store; mpv handed failed stream URLs to `yt-dlp`.

The sections below are the plan as written before the port.

Target: **v0.2.0**, a signed and notarized Apple Silicon DMG, released together with a Windows build from the same tag.

## 1. Baseline

Checked on macOS 27.0, arm64, rustc 1.100.1, at commit `7b59a23`.

| Check | Result |
| --- | --- |
| `cargo test -p nuvio-core -p nuvio-mpv` | 60 passed, 1 ignored. The libmpv tests skip when the library is missing and still count as passed. |
| `cargo test -p nuvio` | 20 passed |
| `cargo check -p nuvio --all-targets` | passes |
| `cargo build -p nuvio` | **fails at link**, 8 undefined Win32 symbols: `AttachConsole`, `DwmFlush`, `GetSystemMetrics`, `MessageBoxW`, `SetPriorityClass`, `SetProcessInformation`, `timeBeginPeriod`, `timeEndPeriod` |
| `cargo clippy --workspace --all-targets` | warnings only, mostly collapsible `if` |
| CI | none (no `.github` folder) |
| Release | `v0.1.0`, one Windows `.exe` asset |
| Local signing identities | none |

`cargo check` passing is misleading: several things compile and then behave wrongly on macOS (section 3, Phase 1).

## 2. Decisions to make

| # | Decision | Recommendation | Why |
| --- | --- | --- | --- |
| D1 | Architectures | arm64 for v0.2.0; universal later | Avoids building two sets of native libraries now |
| D2 | Minimum macOS | 12.0, set with `MACOSX_DEPLOYMENT_TARGET` | The linker currently defaults to 11.0. Confirm after the libmpv build; only claim versions that were tested |
| D3 | Signing | Apple Developer ID plus notarization (paid account) | Without it every user must override Gatekeeper by hand |
| D4 | libmpv | Homebrew for development; a pinned source build in CI for releases | Homebrew `mpv` pulls in VapourSynth, Python and yt-dlp, and its bottles set their own minimum macOS |
| D5 | Video path | Keep libmpv's OpenGL render API | Reuses `player.rs` and `glass.rs`. OpenGL is deprecated on macOS; a Metal rewrite is a separate project |
| D6 | UI reference | The current Windows build, with NuvioDesktop for macOS conventions | NuvioDesktop is Kotlin/Compose, so no code carries over. `README.md` and `Cargo.toml` point at NuvioMobile `cmp-rewrite`; decide which is the reference |

## 3. Implementation

Order: 0, 1, 2, then 3 and 4 in parallel, then 5 and 6. Estimates are rough working days for one person.

### Phase 0: groundwork (1 day)

* Add `rust-toolchain.toml` (pin the version).
* Add CI on `windows-latest` and an arm64 macOS runner: fmt check if the project adopts rustfmt settings, clippy, tests.
* Add `NUVIO_REQUIRE_LIBMPV=1`: CI fails instead of skipping `crates/mpv/tests/playback.rs`.
* Make Windows CI green first. It is the regression guard for every later change.

### Phase 1: build, launch, keep data safely (2-3 days)

Create `crates/app/src/platform/{mod,windows,macos}.rs` with one interface; move `platform.rs`, `compositor.rs` and the OS calls behind it. Move `windows-sys` to `[target.'cfg(windows)'.dependencies]`.

| Area | Today | macOS work |
| --- | --- | --- |
| **Data paths** (`core/src/paths.rs`) | Reads `APPDATA`; unset on macOS, so **library, progress and account go to the temp folder**, which the OS purges | `~/Library/Application Support/NuvioRust`, `~/Library/Caches/NuvioRust`, `~/Library/Logs/NuvioRust`. Fix before anyone runs a build |
| Window placement (`window.rs`) | `GetSystemMetrics` | Use monitor information from winit/AppKit, in points |
| Frame pacing (`compositor.rs`) | `DwmFlush` | No-op; rely on vsync. Check that a hidden or covered window does not spin |
| Power (`platform.rs`) | EcoQoS, timer resolution, priority | `NSProcessInfo` activity during playback |
| Console (`logging.rs`) | `AttachConsole` | Nothing needed |
| Crash dialog (`main.rs`) | `MessageBoxW` | Dialog through `osascript` (panics can come from any thread) |
| OpenGL lookup (`mpv/src/gl.rs`) | `opengl32.dll`, `wglGetProcAddress` | `dlsym` on `OpenGL.framework` |
| libmpv lookup (`mpv/src/sys.rs`) | DLL names | `libmpv.2.dylib` in `NUVIO_LIBMPV`, `../Frameworks`, `libs/macos-arm64`; Homebrew paths in debug builds only |
| Open links (`app.rs`) | `rundll32` | `open`, **http and https only**, since the URL comes from an addon |
| Open folder / edit file (`settings.rs`) | `explorer`, `notepad` | `open`, `open -t` |

Exit: the app launches, shows Home from Cinemeta, and writes only under `~/Library`.

### Phase 2: playback (3-5 days, highest risk)

Start with a one-day spike: Homebrew libmpv, one local file, picture and sound in the window.

* Set `LC_NUMERIC=C` before `mpv_create` (NuvioDesktop's release notes record the same fix).
* Decoder list per OS (`player.rs:28`): `videotoolbox,videotoolbox-copy`.
* Dolby Vision profile 5 (`player.rs:519`): use `videotoolbox-copy`. Verify the bundled FFmpeg has the `libplacebo` filter and that it runs through MoltenVK. If it does not, ship without it and say so in `PORTING.md`.
* `mpv_custom.rs:89`: script list separator is `:` on macOS (`;` on Windows). Accept `.so`/`.dylib`, add Meta (⌘) key forwarding.
* Prevent display sleep while playing and release it on pause and exit (see U14).
* Verify Retina sizing, resize, native full screen, moving between displays, sleep/wake, audio device changes.
* Torrents: raise the open-file limit at start (GUI apps get a low default), check the firewall and local network prompts.
* Add `libs/macos-arm64` fetch script, `scripts/fetch-libmpv.sh`, with a pinned URL and SHA-256, as the PowerShell script does.

Exit: H.264, HEVC 10-bit with hardware decoding, subtitles, HTTP and torrent streams all play.

### Phase 3: macOS integration (2-3 days)

* **Full screen state:** the green button changes full screen without updating `view.fullscreen`. Read it from the viewport every frame. Do not leave a full screen the user entered before opening the player.
* **Window state:** egui-winit only reads `maximized` at startup on macOS, so `track_window` (`app.rs:945`) saves stale values. Do not save during the full-screen animation.
* **Shortcuts:** ⌘[ back, ⌃⌘F full screen (F11 belongs to the system), ⌘F or ⌘K search, ⌘R refresh, ⌘, settings, ⌘Q and ⌘W. Tooltips show the macOS keys.
* **Quit:** eframe calls `on_exit` when the loop exits. Test that ⌘Q saves progress and settings.
* **Text:** replace "Nuvio for Windows" (settings About, import offer, HTTP user agent in `core/src/http.rs:17`).
* **Import from NuvioDesktop:** find where its macOS build keeps addons, then extend `core/src/import.rs`. Hide the offer until then.
* **Secrets:** `account.json` holds tokens in plain text. Create it with mode 0600 now; move to Keychain later.
* **Look:** `.icns` from a 1024 px source (the current PNG is 256 px), do not override the Dock icon at run time on macOS, check emoji in stream descriptions.

### Phase 4: UI bugs (3-4 days)

All were found by reading the code. **None has been reproduced on screen**, because the app does not link on macOS yet. Reproduce each (Windows now, macOS after Phase 2), then fix with a test.

| ID | Severity | Problem | Where | Fix | Test |
| --- | --- | --- | --- | --- | --- |
| U1 | High | "Up next" and auto-skip countdowns keep running while paused, so a paused video jumps | `player_view.rs:1041`, `:1073` | Count only unpaused, non-buffering time | unit on a countdown type |
| U2 | High | A click on empty space in the control bar, stats panel or error card falls through and toggles pause | `player_view.rs:385` | Give panels their own click sense | headless egui |
| U3 | High | Blank screen with no way out when the auto-picked stream opens externally or the player fails to start | `app.rs:785`, `:487` | Pick only in-app streams; fall back to the stream list | unit on selection, UI |
| U4 | High | Reload resets `paused` and `speed` in the UI while mpv keeps them: wrong button, lost speed after next episode. A recovered stream may stay paused (`keep-open`) | `player.rs:472` | Keep both across reload; set `pause=no` on recovery | libmpv integration |
| U5 | High | Details page has no back button while loading or after failure, and the nav bar is hidden | `details.rs:24`, `:32` | Draw the back button in every state | headless egui |
| U6 | High (macOS) | Trackpad scroll over the video uses the wheel acceleration, including momentum, so volume overshoots | `player_view.rs:515` | No streak for point deltas, ignore momentum | unit |
| U7 | Medium | Escape closes a popup and also leaves the page or full screen | `app.rs:1036`, `player_view.rs:594` | Record popup state before drawing; consume Escape | headless egui |
| U8 | Medium | Stream filter popups do not scroll | `streams.rs:214` | Wrap in `ScrollArea` | screenshot at 960×600 |
| U9 | Medium | Continue Watching, seasons and cast have no arrows for mouse users | `home.rs:257`, `details.rs` | Use `paging_row` | screenshot |
| U10 | Medium | Long addon name or error runs under the row's buttons | `settings.rs:253-270` | Truncate to the free width | screenshot |
| U11 | Medium | Long episode titles overlap date, runtime and rating | `details.rs:255` | Measure the metadata first | screenshot |
| U12 | Medium | Storage page scans both caches on the UI thread every frame | `settings.rs:504` | Compute in the background on page open | perf check |
| U13 | Medium | Stream headers containing commas are split, which can break playback | `player.rs:479` | Escape or set as a list | unit and mock server |
| U14 | Medium | Nothing keeps the display awake during playback (confirm on Windows too) | `platform.rs` | Per-OS inhibitor | manual, `pmset -g assertions` |
| U15 | Low | A cached title page ignores the last watched season until fresh data arrives, and always when offline | `state.rs:531` | Apply `default_season` to cached data | unit |
| U16 | Low | ⌘K or `/` on Search does not focus the field | `app.rs:344` | Reset `search_focused` | headless egui |

After the first macOS build, do one visual pass of every screen at 960×600, 1280×800 and full screen, on Retina and non-Retina, against the Windows screenshots. Add findings here.

### Phase 5: packaging (2-3 days)

`scripts/build-macos.sh` does, in order:

1. `cargo build -p nuvio --release --locked --target aarch64-apple-darwin`
2. Assemble `Nuvio.app`: `Contents/{Info.plist,MacOS/nuvio,Frameworks/*.dylib,Resources/{AppIcon.icns,LICENSE,THIRD-PARTY}}`
3. Collect dylib dependencies, rewrite install names to `@rpath`/`@loader_path`; fail if `otool -L` shows `/opt/homebrew` or `/usr/local`
4. Include what MoltenVK needs (its ICD file) if Dolby Vision ships
5. Sign inside out with hardened runtime and timestamp. Add only the entitlements that testing requires (LuaJIT may need JIT; user scripts loading native code need library validation off)
6. Build the DMG with an Applications link, sign it, `notarytool submit --wait`, `stapler staple`
7. Write the SHA-256

`Info.plist`: bundle id different from NuvioDesktop's, version from `Cargo.toml`, `LSMinimumSystemVersion`, `NSHighResolutionCapable`, `NSLocalNetworkUsageDescription`. Do not claim `nuvio://`.

### Phase 6: release (1 day)

See section 5.

## 4. Tests

### Layers

| Layer | Covers | Runs |
| --- | --- | --- |
| Unit | Per-OS paths, library candidates, decoder and script-separator choice, URL scheme filter, header escaping, countdowns, wheel-to-volume, stream auto-pick, import paths | every PR, both OSes |
| libmpv integration | Existing tests with the real dylib; add pause and speed across reload, recovery after a cut-off HTTP stream, headers, subtitle path containing spaces | every PR |
| Headless UI | Drive `egui::Context` with synthetic input for U2, U5, U7, U16 (move decisions out of drawing code where needed) | every PR |
| GUI smoke | The existing `--data-dir --open --click --key --screenshot` options against a local fixture addon | local Mac and pre-release; in CI only if a hosted runner can create the GL context (spike) |
| Package | Bundle checks below | tag |
| Manual | Matrix below | release candidate |

Test support to add:

* A fixture addon server (extend the `tiny_http` mock in `crates/core/tests`) serving manifest, catalog, meta, streams, subtitles and short clips generated by a script.
* Modifiers in the `--key` hook; today it always sends none, so ⌘ shortcuts cannot be driven.
* A script that runs the screenshot set and keeps the images for review. Start with human comparison; pixel diffing is unstable with animation.

### Manual matrix

| Axis | Values |
| --- | --- |
| macOS | minimum supported, previous release, current |
| Display | built-in Retina, external 1×, HDR, move between them |
| Media | H.264, HEVC 10-bit, AV1, HDR10, Dolby Vision 5, 5.1 audio, ASS and SRT subtitles |
| Source | direct HTTPS, debrid URL, torrent (public domain film) |
| Input | trackpad, wheel mouse, keyboard only, Bluetooth media keys |
| Lifecycle | sleep/wake, headphones unplugged, network loss, ⌘Q during playback, covered and minimized window |

### Performance budgets

Measure with `NUVIO_PERF=1` and Activity Monitor on the reference Mac. Proposed starting limits, to be adjusted after the first measurement: first frame under 0.5 s warm; no frames drawn on idle Home; 4K HEVC under 0.5 core with hardware decoding; no memory, thread or file-handle growth over 30 minutes.

### Package checks (automated)

```bash
codesign --verify --deep --strict --verbose=2 Nuvio.app
spctl --assess --type execute -vv Nuvio.app
xcrun stapler validate Nuvio-*.dmg
otool -L Nuvio.app/Contents/MacOS/nuvio Nuvio.app/Contents/Frameworks/*.dylib
lipo -archs Nuvio.app/Contents/MacOS/nuvio
vtool -show-build Nuvio.app/Contents/Frameworks/*.dylib   # minimum macOS
```

### End-to-end release scenario

Run on a Mac or fresh user account **without Homebrew**, using the DMG downloaded in a browser from the draft release.

| # | Step | Pass when |
| --- | --- | --- |
| 1 | Verify SHA-256, open DMG, drag to Applications | checksum matches |
| 2 | First launch | only the standard "downloaded from the internet" prompt; Home fills from Cinemeta |
| 3 | Check folders | data under `~/Library`, nothing in the temp folder; `account.json` is 0600 if present |
| 4 | Add the fixture addon by pasting its link | shelves appear |
| 5 | Search, open a series, open streams, filter | results and filters correct, back works everywhere |
| 6 | Play; pause, seek, volume, audio and subtitle switch, speed, full screen by key and green button | each works, display stays awake |
| 7 | ⌘Q mid-episode, relaunch, resume | position within 10 s, volume kept |
| 8 | Let the episode reach the credits; pause during the countdown, then continue | no jump while paused; next episode keeps tracks and speed |
| 9 | Play the torrent; cut and restore the network | starts, recovers |
| 10 | Sign in with a test account, quit, relaunch, sign out | session kept, then removed |
| 11 | Sleep and wake during playback | playback resumes cleanly |
| 12 | Replace the app with a newer build | data kept |
| 13 | Check the log | no tokens or full stream URLs |
| 14 | Delete the app | no process or sleep assertion left |

**Release gates:** CI green on both OSes; package checks pass; steps 1-14 pass on minimum and current macOS; all High UI bugs fixed or shown not to reproduce; Windows smoke run passes.

## 5. Release runbook (v0.2.0)

1. Add secrets: Developer ID certificate and password, App Store Connect API key for notarization. Protect the release environment.
2. Add `.github/workflows/release.yml`, triggered by `v*` tags: build libmpv (cached, pinned), test, package macOS, build the Windows installer, create a **draft** release.
3. Bump the version in `Cargo.toml` **and** `crates/app/resources/nuvio.rc` (hardcoded `0.1.0`); fail the workflow if tag and versions differ.
4. Update `README.md` (macOS install and build), `PORTING.md`, release notes with known limitations.
5. Tag `v0.2.0-rc.1`, run the scenario, fix, repeat.
6. Tag `v0.2.0`, re-run steps 1-7 on the downloaded assets, publish.
7. Assets: `Nuvio-Rust-0.2.0-macos-arm64.dmg`, `Nuvio-Rust-Setup-0.2.0.exe`, `SHA256SUMS`.
8. GPL: publish the matching source tag and the build recipe and source references for the bundled libmpv and FFmpeg.
9. Rollback: mark the release as pre-release or remove the bad asset, fix, ship a new patch version. Never replace a published file under the same name. There is no auto-update, so say so in the notes.

## 6. Risks

| Risk | Effect | Mitigation |
| --- | --- | --- |
| Building and bundling libmpv | largest unknown | spike first; Homebrew path for development |
| Dolby Vision through libplacebo/MoltenVK | feature may not ship | optional for v0.2.0 |
| Hardware decode through OpenGL | high CPU | verify `hwdec-current` in the spike |
| OpenGL deprecation | future breakage | accept; keep GL code isolated |
| No Developer ID | poor first launch | get it early; otherwise label the build unsigned and document the override |
| Hosted runners lack usable GL | GUI tests not in CI | local pre-release run or a self-hosted Mac |

**Total estimate:** about 14-20 working days.
