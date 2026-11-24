# What is ported, and what is not

The reference is the desktop build of Nuvio (`NuvioMedia/NuvioMobile`, branch `cmp-rewrite`, 0.6.2, with the Windows
target from the Kotlin port). Mobile-only features (gestures, picture in picture, haptics, foreground services) are left
out on purpose. This table says what the Rust build does today. "Partly" always has the missing part written next to it.

| Area | State | Notes |
| --- | --- | --- |
| Addons | done | Install by link (`stremio://` and bare addresses are fixed up), enable, disable, reorder, remove, import from an installed Nuvio for Windows (Kotlin) profile. Cinemeta is installed on the first run. |
| Home | done | Rotating hero with wordmark, Continue Watching, one shelf per browsable catalog with paging arrows and endless loading. Shelves of the last session are shown from the disk cache before the network answers. |
| Search | done | Every addon catalog that supports `search`, results arrive row by row. |
| Library | done | Local only. Right click on a poster or a Continue Watching card: resume, go to the show or season, add to or remove from the library, remove from Continue Watching. |
| Details | partly | Facts, description, genres, seasons and episodes with watch progress, cast, credits, trailer (opens YouTube in the browser). No TMDB enrichment (it needs a key), no ratings from MDBList, no comments, no collections. |
| Streams | done | All addons at once, filters for resolution, addon, service (RD, TB, ...), HDR and language, card layout like the Kotlin build, the server is warmed while the pointer rests on a card. |
| Player | done | libmpv drawing into the window through OpenGL. Controls show on pointer movement and hide with the cursor; tooltips, volume slider and on-screen volume and seek feedback, audio and subtitle tracks, subtitles from addons (downloaded by the app and handed to mpv as files, because mpv opens a subtitle URL on the thread that plays; the menus scroll when the list is long), speed, picture size, playback info, previous and next episode and autoplay of the next episode, both without leaving the player (the stream with the same binge group, else the same addon and resolution; the volume, audio track, subtitles, speed and picture size carry over; the stream list opens only when nothing matches), the next episode offered when the credits begin and started after five seconds, resume position, reopening a stream that was cut off, Dolby Vision profile 5. A seek outside what is buffered lands on a keyframe (an exact seek reads up to a whole group of pictures over the network first), and after a seek or a track change playback resumes after one second of data instead of five. The render context is always freed before the player it belongs to: freeing it afterwards crashed inside libmpv when the player was left. While a stream opens the artwork of the title is shown with its wordmark (or name) breathing slowly and what is happening underneath, as in the Kotlin build's `OpeningOverlay`. |
| Torrents | done | librqbit: magnet or info hash from the stream, the file the addon names (else the episode matching `S01E02`, else the biggest video), served to mpv from a password protected loopback port. |
| Debrid direct streams | not yet | A stream that only carries `clientResolve` (no `url`, no `infoHash`) cannot be played. Addons that resolve debrid links themselves (AIOStreams, Torrentio with a debrid key) return a plain `url` and work. |
| Plugins (QuickJS scrapers) | not yet | |
| Accounts and sync | partly | Sign in, sign up and sign out against the official account server (Settings → Account, or the round button at the top right); the session is kept in `account.json` and renewed before it expires. The server's address and publishable key are read from its public discovery document (`https://api.nuvio.tv/.well-known/nuvio`), so no key is built in; `NUVIO_SUPABASE_URL` and `NUVIO_SUPABASE_ANON_KEY`, or a `server.json` (`{"url", "key", "fallbackUrl"}`) next to the settings, point it somewhere else. Checked against the live server with a login it refuses; a successful login has not been tried here. Not ported: syncing the library, progress and addons through the account, profiles, sign-in by link or QR code, account deletion. Everything stays usable without an account. |
| Profiles | not yet | One profile. |
| Trakt, Simkl, MDBList, TMDB | not yet | |
| Downloads | not yet | |
| Notifications, tray | not yet | |
| External players | not yet | |
| Skip intro and credits | done | Intro, recap and end credits from IntroDB (by IMDb id, season and episode), announced with a five second countdown and skipped by themselves (Settings → Playback turns that off). Episodes nobody has marked have none; the credits then begin 40 seconds before the end. |
| Subtitle styling | not yet | |
| Themes | partly | The twelve palettes and a pure black switch; no custom palette editor. |
| Auto update | not applicable | There is no update feed. |
| Deep links (`nuvio://`) | not applicable | The installer does not claim the scheme, so it does not fight the Kotlin build for it. |

## Where behaviour differs on purpose

* **Data folder.** `%APPDATA%\NuvioRust` and `%LOCALAPPDATA%\NuvioRust`, so the Kotlin and Rust builds can be installed
  together without sharing state. Addons can be imported from the Kotlin profile with one click.
* **Dolby Vision profile 5.** Same method as the Kotlin build (libplacebo filter, NVDEC copy-back decoder), because the
  OpenGL render API of libmpv only offers the classic renderer, which does not reshape profile 5. The result is checked,
  and the file is reopened without the filter when mpv dropped it.
* **Images.** Posters are decoded off the UI thread and shrunk to the size they are drawn at with a Lanczos filter; they
  are cached on disk (400 MB cap).
