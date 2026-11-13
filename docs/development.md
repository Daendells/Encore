# Development guide

Encore is a Tauri 2 tray app for Windows. A Rust engine reads the playing song from the Windows media controls (SMTC), finds synced lyrics on LRCLIB, and tells a small WebView2 window what to draw. For what the app does for users, see [fitur.md](fitur.md).

## Prerequisites

- Windows 10 version 2004 or newer.
- Rust stable, MSVC toolchain.
- Visual Studio 2022 Build Tools with "Desktop development with C++" and a Windows SDK.
- Node 20 (only for the Tauri CLI).
- WebView2 Runtime (already on Windows 11 and current Windows 10).

## Setup and commands

```powershell
npm install                 # once; installs the Tauri CLI
npm run tauri dev           # run a debug build
npm run tauri build         # release build + NSIS installer

cd src-tauri
cargo test                  # unit tests and engine tests
cargo clippy --all-targets -- -D warnings
```

The installer lands in `src-tauri\target\release\bundle\nsis\`. Debug builds also read `ENCORE_LANG=id|en` to force the UI language.

## Architecture

```
src-tauri/src
  engine.rs     one thread owns all state; reacts to Msg, talks to the world through the Host trait
  lrc.rs        parse LRC and enhanced LRC, fill gaps, note rules          (pure)
  normalize.rs  clean titles and artists, build lookup candidates          (pure)
  matcher.rs    score candidates: title, artist, duration                  (pure)
  clock.rs      interpolated playback position, seek detection             (pure)
  schedule.rs   current line index, next start                             (pure)
  lyrics.rs     resolve(): get, then search, behind the Fetch trait
  lrclib.rs     the only network code (ureq, rustls, 10 s timeout)
  cache.rs      one JSON file per song; "not found" expires after 7 days
  settings.rs   settings.json and offsets.json through Store
  smtc.rs       Watcher: follows one media session, returns a Snapshot
  overlay.rs    strip window: geometry, click-through, Ctrl drag, capture exclusion, crash restart
  shell.rs      tray, global shortcuts, settings window and its commands
  i18n.rs       Indonesian and English strings for tray and settings
  timing_log.rs optional CSV log of line timing
  lib.rs        wiring: TauriHost, WEBVIEW_ARGS, setup
ui/             overlay.* and settings.* (plain HTML, CSS and JS, no build step)
```

Flow: SMTC events only send `Msg::SmtcChanged`. The engine waits 60 ms to coalesce them, calls `Host::read_smtc`, syncs the clock, checks the cache, and otherwise starts `Host::fetch_lyrics` on a worker thread. The worker ends in `Msg::Lyrics`. The engine then computes the line, sends `Ui` events to the overlay, and sets one deadline for the next line. The tray, shortcuts and settings window call `Store::update` or send a `Msg`, and the engine reloads settings on `Msg::SettingsChanged`.

Overlay window rules: it is declared with `create: false` and `focusable: false` in `tauri.conf.json` and built in `lib.rs` with the shared `WEBVIEW_ARGS`, so showing it never takes focus from the player. It is a tool window kept out of Alt+Tab. Ctrl is watched with Raw Input on a message-only window (no hook, no polling), and only the key code is read.

## Rules that must not be broken

1. **A miss beats a wrong match.** Never show another song's lyrics. When in doubt, resolve to not found. "Not found" is cached for days, so it is stored only after every LRCLIB call answered. A network error is never cached.
2. **One timer, no polling.** The engine sleeps until the next line, read or card deadline. No fast loop anywhere in Rust.
3. **The webview only draws.** No `requestAnimationFrame` and no `setInterval` in `ui/`. Animations are CSS transitions. The page does no timing.
4. **The network is `lrclib.net` only.** No analytics, CDN, web fonts or update checks. The CSP is `default-src 'self'`.
5. **Capabilities stay least privilege.** The overlay may use only `core:event:default`, window dragging, and the `shown` and `ctrl_released` commands. The settings window gets its own commands and autostart enable, disable and status. The settings commands validate everything again in Rust, because the page is not trusted.
6. **Every window uses the same `WEBVIEW_ARGS`.** They share one WebView2 profile, and WebView2 refuses a window whose flags differ. Each switch has a reason:
   - `msOneAuthWAM,msForceOneAuthWAM`: stops WebView2 signing the Windows account in to Microsoft (a connection to substrate.office.com). Both are needed.
   - `DnsOverHttpsUpgrade`: stops a connection to Google DNS (8.8.8.8 and 8.8.4.4) a few seconds after start when the system DNS is Google.
   - `--disable-background-networking` and `--disable-component-update`: no background traffic.
   - `--disable-gpu`: saves about 60 MB. The strip is three lines of text.
   - `--single-process`: saves about 40 MB. Microsoft does not document it, so a crash takes WebView2 down. `overlay::restart_on_webview_failure` then restarts the app, at most once every 30 s. The restart is recorded before it happens, so it cannot loop.
7. **Settings are written through `Store::update`.** It loads, changes and saves under one lock, so concurrent changes from the engine, tray, settings window and drag all survive. Never read, modify and write the file yourself.
8. **Disk is best effort.** A missing or corrupt cache, settings or offsets file behaves like no file. Writes go to a temp file and are renamed.

## Testing

- **Pure modules (`lrc`, `normalize`, `matcher`, `clock`, `schedule`):** test first. Write the failing case from a real title or LRC sample, then the code. They have no I/O, so tests are fast and exact.
- **Engine:** tests drive `Engine` with a fake `Host` that records emitted `Ui` values and supplies a fake clock. Add a test for every behaviour change, including the order of emitted events.
- **Matching quality:** `src-tauri/examples/eval.rs` runs a song list against live LRCLIB and reports correct, missed, wrong and error. It prints names, ids and outcomes, never lyrics.

  ```powershell
  cd src-tauri
  cargo run --release --example eval -- ..\eval\sample.json out.json
  ```

  `eval/sample.json` is a small example. The 50-song blind set is kept outside the repo, so matching cannot be tuned against it. Run it once per change that touches matching and report the result as it is. Do not copy its songs into the repo.

## Measuring

- **Timing:** set `ENCORE_TIMING_LOG` to a file path, run the release build, play songs, then run `node scripts\timing-report.mjs <file>`. It reports the delay between a line's LRC time and the moment it appears, for lines the timer advanced by exactly one. Other lines are counted as left out. The reference is the position SMTC reports, not the sound.
- **Resources:** with Encore running, `powershell -File scripts\measure-resources.ps1 -Seconds 300` samples CPU, RAM (private bytes and working set) and network destinations. The network sample sees TCP only, about every 450 ms.
- **After a WebView2 update:** repeat the network check, because the `WEBVIEW_ARGS` switches are undocumented and an update can change what they do. Check RAM against the 80 MiB target again.

The targets are in the README table. Report a missed target as missed.

## Commit conventions

- One-line conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `test:`.
- The author is the repo owner. No `Co-Authored-By` trailers and no tool attribution.
- One change per commit, with its tests.

## CI

`.github/workflows/ci.yml` runs on every push and pull request on `windows-latest`: `npm ci`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `npm run tauri build`, then it uploads the NSIS installer as an artifact. Run clippy and the tests locally before pushing.

## Release checklist

1. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. The User-Agent sent to LRCLIB follows `Cargo.toml`, so update its test in `lrclib.rs` and the installer name in the README.
2. `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.
3. `npm run tauri build`. Check the installer size against the 15 MiB target.
4. Install it and run the manual checks: Spotify desktop, YouTube and YouTube Music in Chrome, a song without lyrics, pause and seek, the `Win+Alt` shortcuts, Ctrl drag, tray menu, settings window, capture exclusion, autostart.
5. Measure timing, resources and network, and update the README table with the date, commit and WebView2 version.
6. Tag the commit and publish the installer.
