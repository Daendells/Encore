//! The engine: the one thread that owns all state. It ties SMTC snapshots, the lyric lookup, the
//! interpolating clock and the single line timer together, and tells the webview what to draw.
//!
//! Time is passed in through `Host`, so everything here is tested without a real clock.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::cache::Cache;
use crate::clock::{self, Clock};
use crate::lrc::{Line, LONG_GAP_MS, NOTE_AFTER_MS};
use crate::lyrics::Lyrics;
use crate::matcher::Track;
use crate::schedule::{index_at, next_start_after};
use crate::settings::{Settings, Store};
use crate::smtc::{Snapshot, Status};
use crate::timing_log::TimingLog;

/// SMTC fires several events for one change; they are read once after this long.
const COALESCE_MS: f64 = 60.0;
/// How long the "no lyrics" card stays.
const CARD_MS: f64 = 3_000.0;
/// A new line deadline closer than this to the old one is not worth replacing.
const KEEP_DEADLINE_MS: f64 = 15.0;
/// A track whose length differs by more than this from the one we looked up is another track.
const DURATION_TOLERANCE_MS: u32 = 2_000;
/// After a timeline that does not fit the lyrics, the next read (or a title event) decides what the track is.
const CONFIRM_MS: f64 = 600.0;
const LAST_WORD_MS: f64 = 500.0;
const NOTE: &str = "♪";

#[derive(Debug)]
pub enum Msg {
    SmtcChanged,
    Lyrics { key: String, result: Result<Lyrics, String> },
    ToggleVisible,
    SetVisible(bool),
    Nudge(i32),
    SettingsChanged,
    Shown { seq: u64 },
    Quit,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ui {
    Lines {
        seq: u64,
        prev: Option<String>,
        cur: String,
        next: Option<String>,
        words: Option<Vec<UiWord>>,
        animate: bool,
        lit: bool,
    },
    Info { title: String, artist: String },
    Toast { text: String },
    /// The plate is emptied, so that the next time the window is shown it holds nothing of the old song.
    Clear,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UiWord {
    pub text: String,
    pub delay_ms: i32,
    pub dur_ms: u32,
}

pub trait Host {
    fn emit(&self, ui: &Ui);
    fn set_overlay_visible(&self, visible: bool);
    /// The settings were (re)loaded: size, position and capture exclusion of the window follow them.
    fn settings_changed(&self, _settings: &Settings) {}
    /// Starts a lookup that ends in `Msg::Lyrics`. A second request for a key still running is ignored.
    fn fetch_lyrics(&self, key: String, track: Track);
    fn read_smtc(&mut self) -> Option<Snapshot>;
    fn now_mono_ms(&self) -> f64;
    fn now_wall_ms(&self) -> i64;
}

enum Content {
    /// Lyrics requested, not here yet.
    Loading,
    Lines(Vec<Line>),
    /// Not found or failed: the title card, shown until `card_at`.
    Card,
}

struct Current {
    key: String,
    track: Track,
    content: Content,
    song_offset_ms: i32,
}

/// What the overlay was last told.
struct Told {
    idx: Option<usize>,
    playing: bool,
}

#[derive(Clone, Copy)]
enum How {
    /// A snapshot: tell the overlay only when the line or the play state changed.
    Auto,
    /// Tell the overlay whatever changed (seek, first show, nudge, shown again).
    Force,
    /// The line timer fired: the one case that animates.
    Timer,
}

pub struct Engine {
    settings: Settings,
    store: Store,
    cache: Cache,
    log: Option<TimingLog>,
    clock: Clock,
    rate: f64,
    duration_ms: f64,
    app: String,
    cur: Option<Current>,
    told: Option<Told>,
    /// Whether the overlay window is shown right now.
    shown: bool,
    seq: u64,
    /// `(seq, index, line start)` of the latest `Ui::Lines`, for the `shown` rows of the timing log.
    last_emit: Option<(u64, Option<usize>, u32, bool)>,
    read_at: Option<f64>,
    line_at: Option<f64>,
    card_at: Option<f64>,
    /// While set the strip stays empty, and this is when the confirming read happens.
    hold_until: Option<f64>,
}

impl Engine {
    pub fn new(settings: Settings, store: Store, cache: Cache) -> Self {
        Engine {
            settings,
            store,
            cache,
            log: None,
            clock: Clock::default(),
            rate: 1.0,
            duration_ms: 0.0,
            app: String::new(),
            cur: None,
            told: None,
            shown: false,
            seq: 0,
            last_emit: None,
            read_at: None,
            line_at: None,
            card_at: None,
            hold_until: None,
        }
    }

    pub fn with_timing_log(mut self, log: Option<TimingLog>) -> Self {
        self.log = log;
        self
    }

    /// Handles one message, or a timeout when `msg` is None. Returns the mono deadline of the next wake-up, if any.
    pub fn step(&mut self, msg: Option<Msg>, host: &mut dyn Host) -> Option<f64> {
        if let Some(msg) = msg {
            self.handle(msg, host);
        }
        self.fire_due(host);
        [self.read_at, self.line_at, self.card_at, self.hold_until].into_iter().flatten().reduce(f64::min)
    }

    fn handle(&mut self, msg: Msg, host: &mut dyn Host) {
        match msg {
            // An open window is not extended, so a burst of events costs one read.
            Msg::SmtcChanged => _ = self.read_at.get_or_insert(host.now_mono_ms() + COALESCE_MS),
            Msg::Lyrics { key, result } => self.lyrics_arrived(&key, result, host),
            Msg::ToggleVisible => self.set_visible(!self.settings.visible, host),
            Msg::SetVisible(visible) => self.set_visible(visible, host),
            Msg::Nudge(delta) => self.nudge(delta, host),
            Msg::SettingsChanged => self.reload_settings(host),
            Msg::Shown { seq } => {
                let (idx, start, timer) = match self.last_emit {
                    Some((last, idx, start, timer)) if last == seq => (idx, Some(start), timer),
                    _ => (None, None, false),
                };
                self.log_row("shown", host, idx, start, &emit_extra(seq, timer));
            }
            Msg::Quit => {}
        }
    }

    /// Runs whatever has come due: the SMTC read, the end of the card, the line change.
    fn fire_due(&mut self, host: &mut dyn Host) {
        let due = |at: Option<f64>, host: &dyn Host| at.is_some_and(|t| host.now_mono_ms() >= t);
        if due(self.read_at, host) || due(self.hold_until, host) {
            self.read_at = None;
            self.read(host);
        }
        if due(self.card_at, host) {
            self.card_at = None;
            self.apply_visibility(host);
        }
        if due(self.line_at, host) {
            self.line_at = None;
            self.render(host, How::Timer);
        }
    }

    /// Reads the session once and brings the engine in line with it.
    fn read(&mut self, host: &mut dyn Host) {
        let snapshot = host.read_smtc();
        let (now, wall) = (host.now_mono_ms(), host.now_wall_ms());
        let Some(s) = snapshot.filter(|s| !matches!(s.status, Status::Closed | Status::Stopped)) else {
            return self.forget(host);
        };
        let duration_ms = s.report.duration_ms.round().clamp(0.0, f64::from(u32::MAX)) as u32;
        // Spotify sends the new timeline and the new title in separate events, in either order. Until both
        // are there, the old lyrics must not run against the new timeline.
        if s.title.trim().is_empty() || duration_ms == 0 {
            return self.forget(host);
        }
        let same_song = self.cur.as_ref().is_some_and(|c| c.track.title == s.title && c.track.artist == s.artist);
        let same = same_song
            && self.cur.as_ref().is_some_and(|c| c.track.duration_ms.abs_diff(duration_ms) <= DURATION_TOLERANCE_MS);
        let confirming = self.hold_until.take_if(|t| now >= *t).is_some();
        // The title stayed but the length changed: Spotify sends the next timeline before the next title. Wait for
        // the title, and take it for a new track only when it still has not come.
        if same_song && !same && !confirming {
            return self.hold(now, host);
        }
        if !same {
            self.forget(host);
        }
        let sync = self.clock.sync(&s.report, now, wall);
        self.app = s.app;
        self.rate = s.report.rate.filter(|r| r.is_finite() && *r > 0.0).unwrap_or(1.0);
        self.duration_ms = s.report.duration_ms;
        let fresh = (!same).then(|| {
            let track = Track { title: s.title, artist: s.artist, album: s.album, duration_ms };
            (Cache::key(&track), track)
        });
        if let Some((key, track)) = &fresh {
            let song_offset_ms = self.store.song_offset_ms(key);
            self.cur = Some(Current { key: key.clone(), track: track.clone(), content: Content::Loading, song_offset_ms });
        }
        let extra = match sync {
            clock::Sync::First => "first".to_string(),
            clock::Sync::Seek { .. } => "seek".to_string(),
            clock::Sync::Drift { error_ms } => (error_ms.round() as i64).to_string(),
        };
        let idx = self.index(now);
        let start = idx.map(|i| self.lines()[i].start_ms);
        self.log_row("sync", host, idx, start, &extra);
        // A jump to the very start is also what the next song looks like when its title is still the old one.
        let to_start = matches!(sync, clock::Sync::Seek { .. }) && self.clock.position_ms(now) < clock::SEEK_THRESHOLD_MS;
        if same && to_start && !confirming && self.hold_until.is_none() {
            return self.hold(now, host);
        }

        match fresh {
            Some((key, track)) => match self.cache.get(&track, wall_secs(wall)) {
                Some(lyrics) => self.accept(lyrics, host),
                None => host.fetch_lyrics(key, track),
            },
            None => self.render(host, if matches!(sync, clock::Sync::Drift { .. }) { How::Auto } else { How::Force }),
        }
    }

    /// Empties the strip until `CONFIRM_MS` have passed and the track has been read again.
    fn hold(&mut self, now: f64, host: &dyn Host) {
        self.hold_until = Some(now + CONFIRM_MS);
        self.told = None;
        self.line_at = None;
        self.card_at = None;
        self.apply_visibility(host);
    }

    /// No track (yet): drop the lyrics and the timers and hide the overlay.
    fn forget(&mut self, host: &dyn Host) {
        self.cur = None;
        self.hold_until = None;
        self.clock.reset();
        self.told = None;
        self.line_at = None;
        self.card_at = None;
        self.apply_visibility(host);
    }

    fn lyrics_arrived(&mut self, key: &str, result: Result<Lyrics, String>, host: &dyn Host) {
        // A result for another track, or one we no longer wait for, is dropped.
        let Some(c) = self.cur.as_ref().filter(|c| c.key == key && matches!(c.content, Content::Loading)) else {
            return;
        };
        match result {
            Ok(lyrics) => {
                self.cache.put(&c.track, &lyrics, wall_secs(host.now_wall_ms()));
                self.accept(lyrics, host);
            }
            Err(e) => {
                eprintln!("larik: lyrics lookup failed: {e}");
                self.show_card(host);
            }
        }
    }

    fn accept(&mut self, lyrics: Lyrics, host: &dyn Host) {
        let lines = match lyrics {
            Lyrics::Synced(mut lines) if !lines.is_empty() => {
                // Like a long gap inside the song: a last line that the track outlasts gets a break after it.
                let duration_ms = self.cur.as_ref().map_or(0, |c| c.track.duration_ms);
                if let Some(last) = lines.last().filter(|l| !l.text.is_empty() && duration_ms.saturating_sub(l.start_ms) > LONG_GAP_MS) {
                    lines.push(Line { start_ms: last.start_ms + NOTE_AFTER_MS, text: String::new(), words: vec![] });
                }
                lines
            }
            Lyrics::Instrumental => vec![Line { start_ms: 0, text: String::new(), words: vec![] }],
            _ => return self.show_card(host),
        };
        if let Some(c) = &mut self.cur {
            c.content = Content::Lines(lines);
        }
        self.render(host, How::Force);
    }

    /// The title and artist for 3 seconds, when there are no lyrics to show.
    fn show_card(&mut self, host: &dyn Host) {
        let Some(c) = &mut self.cur else { return };
        c.content = Content::Card;
        if self.settings.visible {
            host.emit(&Ui::Info { title: c.track.title.clone(), artist: c.track.artist.clone() });
            self.card_at = Some(host.now_mono_ms() + CARD_MS);
            self.apply_visibility(host);
        }
    }

    fn set_visible(&mut self, visible: bool, host: &dyn Host) {
        // Saved on top of the file, so a change made by the settings window in the meantime is kept.
        self.store.update(|s| s.visible = visible);
        self.reload_settings(host);
    }

    fn reload_settings(&mut self, host: &dyn Host) {
        self.settings = self.store.load();
        host.settings_changed(&self.settings);
        if let Some(c) = &mut self.cur {
            c.song_offset_ms = self.store.song_offset_ms(&c.key);
        }
        if !self.settings.visible {
            self.card_at = None;
        }
        self.render(host, How::Force);
        self.apply_visibility(host);
    }

    fn nudge(&mut self, delta: i32, host: &dyn Host) {
        let Some(c) = &mut self.cur else { return };
        c.song_offset_ms = c.song_offset_ms.saturating_add(delta);
        self.store.set_song_offset_ms(&c.key, c.song_offset_ms);
        if self.shown {
            let total = c.song_offset_ms;
            host.emit(&Ui::Toast { text: if total == 0 { "0 ms".to_string() } else { format!("{total:+} ms") } });
        }
        self.render(host, How::Force);
    }

    /// Shows or hides the window so that it matches what there is to show.
    fn apply_visibility(&mut self, host: &dyn Host) {
        let content = match self.cur.as_ref().map(|c| &c.content) {
            Some(Content::Lines(_)) => true,
            Some(Content::Card) => self.card_at.is_some(),
            _ => false,
        };
        let want = self.settings.visible && content && self.hold_until.is_none();
        if want != self.shown {
            self.shown = want;
            if !want {
                // Before the window goes: its next show must not flash the last line of this song.
                host.emit(&Ui::Clear);
            }
            host.set_overlay_visible(want);
        }
    }

    fn lines(&self) -> &[Line] {
        match self.cur.as_ref().map(|c| &c.content) {
            Some(Content::Lines(lines)) => lines,
            _ => &[],
        }
    }

    /// App offset plus song offset: the lyric time is the playback position plus this.
    fn offset_ms(&self) -> f64 {
        let song = self.cur.as_ref().map_or(0, |c| c.song_offset_ms);
        f64::from(self.settings.app_offsets_ms.get(&self.app).copied().unwrap_or(0)) + f64::from(song)
    }

    fn lyric_pos(&self, now: f64) -> f64 {
        self.clock.position_ms(now) + self.offset_ms()
    }

    fn index(&self, now: f64) -> Option<usize> {
        index_at(self.lines(), self.lyric_pos(now))
    }

    /// Tells the overlay what to draw, when that has changed or `how` forces it, then arms the line timer.
    fn render(&mut self, host: &dyn Host, how: How) {
        if self.lines().is_empty() || !self.settings.visible || self.hold_until.is_some() {
            // Nothing to draw, or the user hid the strip: say nothing now, and everything when it is back.
            self.told = None;
            self.line_at = None;
            return;
        }
        let now = host.now_mono_ms();
        let (idx, playing) = (self.index(now), self.clock.is_playing());
        let before = self.told.as_ref().map(|t| t.idx);
        let changed = self.told.as_ref().is_none_or(|t| t.idx != idx || t.playing != playing);
        if changed || matches!(how, How::Force) {
            // One step forward by the timer is the only change that slides.
            let animate = matches!(how, How::Timer) && matches!((before, idx), (Some(Some(a)), Some(b)) if b == a + 1);
            self.seq += 1;
            let ui = self.lines_ui(idx, now, animate);
            host.emit(&ui);
            let start = idx.map(|i| self.lines()[i].start_ms);
            self.last_emit = Some((self.seq, idx, start.unwrap_or(0), animate));
            self.told = Some(Told { idx, playing });
            self.log_row("emit", host, idx, start, &emit_extra(self.seq, animate));
        }
        self.apply_visibility(host);
        self.arm_line_timer(now);
    }

    fn lines_ui(&self, idx: Option<usize>, now: f64, animate: bool) -> Ui {
        let lines = self.lines();
        let (prev, cur, next) = match idx {
            None => (None, &lines[0], lines.get(1)),
            Some(i) => (i.checked_sub(1).map(|p| &lines[p]), &lines[i], lines.get(i + 1)),
        };
        let text = |l: &Line| if l.text.is_empty() { NOTE.to_string() } else { l.text.clone() };
        let pos = self.lyric_pos(now);
        let words = (self.clock.is_playing() && !cur.words.is_empty()).then(|| {
            let ends = cur.words.iter().skip(1).map(|w| w.start_ms).chain(next.map(|l| l.start_ms));
            cur.words
                .iter()
                .zip(ends.map(Some).chain(std::iter::repeat(None)))
                .map(|(w, end)| {
                    let start = f64::from(w.start_ms);
                    let end = end.map_or(start + LAST_WORD_MS, f64::from);
                    UiWord {
                        text: w.text.clone(),
                        delay_ms: ((start - pos) / self.rate).round() as i32,
                        dur_ms: ((end - start).max(0.0) / self.rate).round() as u32,
                    }
                })
                .collect()
        });
        Ui::Lines {
            seq: self.seq,
            prev: prev.map(text),
            cur: text(cur),
            next: next.map(text),
            words,
            animate,
            lit: idx.is_some(),
        }
    }

    /// The single timer: wakes just after the next line starts. Keeps the old deadline when the new one is
    /// within 15 ms of it, so small drift corrections do not move the timer.
    fn arm_line_timer(&mut self, now: f64) {
        let new = self.next_line_mono(now);
        self.line_at = match (self.line_at, new) {
            (Some(old), Some(new)) if old > now && (old - new).abs() < KEEP_DEADLINE_MS => Some(old),
            _ => new,
        };
    }

    fn next_line_mono(&self, now: f64) -> Option<f64> {
        if !self.settings.visible || !self.clock.is_playing() {
            return None;
        }
        let target = f64::from(next_start_after(self.lines(), self.lyric_pos(now))?) - self.offset_ms();
        // A line after the end of the track is never reached, and waiting for it would wake again and again.
        if target > self.duration_ms {
            return None;
        }
        // The extra millisecond keeps float rounding from waking us just before the line starts.
        self.clock.mono_at(target, now).map(|at| at + 1.0)
    }

    fn log_row(&mut self, kind: &str, host: &dyn Host, idx: Option<usize>, start: Option<u32>, extra: &str) {
        if self.log.is_none() {
            return;
        }
        let pos = self.lyric_pos(host.now_mono_ms()).round() as i64;
        let title = self.cur.as_ref().map_or("", |c| c.track.title.as_str());
        if let Some(log) = &mut self.log {
            log.row(kind, host.now_wall_ms(), &self.app, title, idx, start, pos, extra);
        }
    }
}

/// Timing log marker of an emit and its `shown`: `timer` when the line timer advanced exactly one line, `other` for
/// every emit with a different cause (resume, show, nudge, settings, first line), whose time is not a line start.
fn emit_extra(seq: u64, timer: bool) -> String {
    format!("{seq} {}", if timer { "timer" } else { "other" })
}

fn wall_secs(wall_ms: i64) -> u64 {
    (wall_ms / 1000).max(0) as u64
}

pub fn run(rx: Receiver<Msg>, mut engine: Engine, mut host: impl Host) {
    let mut deadline: Option<f64> = None;
    loop {
        let msg = match deadline {
            None => rx.recv().ok(),
            Some(at) => {
                let wait = Duration::from_secs_f64((at - host.now_mono_ms()).max(0.0) / 1000.0);
                match rx.recv_timeout(wait) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        };
        // `recv` fails only when every sender is gone.
        if matches!(msg, Some(Msg::Quit)) || (deadline.is_none() && msg.is_none()) {
            return;
        }
        deadline = engine.step(msg, &mut host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Report;
    use crate::lrc::Word;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const DUR: f64 = 200_000.0;
    const WALL: i64 = 1_700_000_000_000;

    /// Scripted SMTC, a manual clock, and a record of everything the engine did to it.
    struct Fake {
        mono: f64,
        wall: i64,
        snap: Option<Snapshot>,
        reads: Cell<usize>,
        emitted: RefCell<Vec<Ui>>,
        vis: RefCell<Vec<bool>>,
        fetches: RefCell<Vec<(String, Track)>>,
    }

    impl Host for Fake {
        fn emit(&self, ui: &Ui) {
            self.emitted.borrow_mut().push(ui.clone());
        }
        fn set_overlay_visible(&self, visible: bool) {
            self.vis.borrow_mut().push(visible);
        }
        fn fetch_lyrics(&self, key: String, track: Track) {
            self.fetches.borrow_mut().push((key, track));
        }
        fn read_smtc(&mut self) -> Option<Snapshot> {
            self.reads.set(self.reads.get() + 1);
            self.snap.clone()
        }
        fn now_mono_ms(&self) -> f64 {
            self.mono
        }
        fn now_wall_ms(&self) -> i64 {
            self.wall
        }
    }

    impl Fake {
        /// What was emitted since the last call.
        fn out(&self) -> Vec<Ui> {
            self.emitted.take()
        }
        /// Overlay show (true) and hide (false) calls since the last call.
        fn vis(&self) -> Vec<bool> {
            self.vis.take()
        }
        fn fetched(&self) -> usize {
            self.fetches.borrow().len()
        }
        fn last_key(&self) -> String {
            self.fetches.borrow().last().unwrap().0.clone()
        }
    }

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Rig {
        e: Engine,
        h: Fake,
        dir: PathBuf,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Rig {
        fn new() -> Rig {
            Rig::with(Settings::default(), None)
        }

        fn with(settings: Settings, log: Option<TimingLog>) -> Rig {
            let dir = std::env::temp_dir().join(format!(
                "larik-engine-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let e = Engine::new(settings, Store::new(dir.clone()), Cache::new(dir.join("cache"))).with_timing_log(log);
            let h = Fake {
                mono: 1_000.0,
                wall: WALL,
                snap: None,
                reads: Cell::new(0),
                emitted: Default::default(),
                vis: Default::default(),
                fetches: Default::default(),
            };
            Rig { e, h, dir }
        }

        /// A rig with `lyrics` shown for "Song", playing at `pos` as read at mono 1060. Nothing pending.
        fn showing(lyrics: Vec<Line>, pos: f64) -> Rig {
            let mut r = Rig::new();
            r.poll(play("Song", pos));
            r.lyrics(Ok(Lyrics::Synced(lyrics)));
            r.h.out();
            r.h.vis();
            r
        }

        /// Every deadline must be in the future: a deadline at or before now would spin the run loop.
        fn step(&mut self, msg: Option<Msg>) -> Option<f64> {
            let deadline = self.e.step(msg, &mut self.h);
            assert!(deadline.is_none_or(|d| d > self.h.mono), "deadline {deadline:?} at {}", self.h.mono);
            deadline
        }

        fn advance(&mut self, ms: f64) {
            self.h.mono += ms;
            self.h.wall += ms as i64;
        }

        /// An SMTC event, the 60 ms wait, then the read, which sees `snap`.
        fn poll(&mut self, snap: Option<Snapshot>) -> Option<f64> {
            self.h.snap = snap;
            self.step(Some(Msg::SmtcChanged));
            self.advance(COALESCE_MS);
            self.step(None)
        }

        fn lyrics(&mut self, result: Result<Lyrics, String>) -> Option<f64> {
            let key = self.h.last_key();
            self.step(Some(Msg::Lyrics { key, result }))
        }

        fn secs(&self) -> u64 {
            (self.h.wall / 1000) as u64
        }
    }

    fn snap(title: &str, pos: f64, playing: bool, duration_ms: f64) -> Option<Snapshot> {
        Some(Snapshot {
            app: "Spotify.exe".into(),
            title: title.into(),
            artist: "Artist".into(),
            album: String::new(),
            status: if playing { Status::Playing } else { Status::Paused },
            report: Report { position_ms: pos, last_updated_wall_ms: None, rate: None, playing, duration_ms },
        })
    }

    fn play(title: &str, pos: f64) -> Option<Snapshot> {
        snap(title, pos, true, DUR)
    }

    fn pause(title: &str, pos: f64) -> Option<Snapshot> {
        snap(title, pos, false, DUR)
    }

    fn track(title: &str) -> Track {
        Track { title: title.into(), artist: "Artist".into(), album: String::new(), duration_ms: DUR as u32 }
    }

    fn ln(start_ms: u32, text: &str) -> Line {
        Line { start_ms, text: text.into(), words: vec![] }
    }

    /// Starts at 1, 5, 9 and 13 s.
    fn song() -> Vec<Line> {
        vec![ln(1_000, "one"), ln(5_000, "two"), ln(9_000, "three"), ln(13_000, "four")]
    }

    fn lines(seq: u64, prev: Option<&str>, cur: &str, next: Option<&str>, animate: bool, lit: bool) -> Ui {
        let own = |s: &str| s.to_string();
        Ui::Lines { seq, prev: prev.map(own), cur: own(cur), next: next.map(own), words: None, animate, lit }
    }

    fn info(title: &str) -> Ui {
        Ui::Info { title: title.into(), artist: "Artist".into() }
    }

    fn toast(text: &str) -> Ui {
        Ui::Toast { text: text.into() }
    }

    #[test]
    fn shows_lines_when_lyrics_arrive_and_advances_on_deadline() {
        let mut r = Rig::new();
        assert_eq!(r.poll(play("Song", 2_000.0)), None);
        assert_eq!(r.h.fetched(), 1);
        assert_eq!((r.h.out(), r.h.vis()), (vec![], vec![]));

        // Read at mono 1060 with the position at 2 s: line "two" (5 s) is 3 s away, plus the 1 ms margin.
        assert_eq!(r.lyrics(Ok(Lyrics::Synced(song()))), Some(4_061.0));
        assert_eq!(r.h.out(), [lines(1, None, "one", Some("two"), false, true)]);
        assert_eq!(r.h.vis(), [true]);

        r.advance(3_001.0);
        assert_eq!(r.step(None), Some(8_061.0));
        assert_eq!(r.h.out(), [lines(2, Some("one"), "two", Some("three"), true, true)]);
        assert!(r.h.vis().is_empty());
    }

    #[test]
    fn coalesces_smtc_events_for_60ms() {
        let mut r = Rig::new();
        r.h.snap = play("Song", 0.0);
        assert_eq!(r.step(Some(Msg::SmtcChanged)), Some(1_060.0));
        r.advance(10.0);
        assert_eq!(r.step(Some(Msg::SmtcChanged)), Some(1_060.0));
        r.advance(10.0);
        assert_eq!(r.step(Some(Msg::SmtcChanged)), Some(1_060.0));
        r.advance(39.0);
        assert_eq!(r.step(None), Some(1_060.0));
        assert_eq!(r.h.reads.get(), 0);
        r.advance(1.0);
        r.step(None);
        assert_eq!(r.h.reads.get(), 1);
        // The next event opens a new window.
        assert_eq!(r.step(Some(Msg::SmtcChanged)), Some(1_120.0));
    }

    #[test]
    fn late_lyrics_for_old_track_are_dropped() {
        let mut r = Rig::new();
        r.poll(play("Song A", 2_000.0));
        let key_a = r.h.last_key();
        r.poll(play("Song B", 100.0));
        let key_b = r.h.last_key();
        assert_ne!(key_a, key_b);

        r.step(Some(Msg::Lyrics { key: key_a, result: Ok(Lyrics::Synced(song())) }));
        assert_eq!((r.h.out(), r.h.vis()), (vec![], vec![]));
        r.lyrics(Ok(Lyrics::Synced(vec![ln(0, "b one"), ln(4_000, "b two")])));
        assert_eq!(r.h.out(), [lines(1, None, "b one", Some("b two"), false, true)]);
    }

    #[test]
    fn duration_mismatch_blanks_lyrics() {
        let mut r = Rig::new();
        r.poll(snap("Song", 2_000.0, true, 207_000.0));
        r.lyrics(Ok(Lyrics::Synced(song())));
        let old_key = r.h.last_key();
        assert_eq!((r.h.out().len(), r.h.vis()), (1, vec![true]));

        // The timeline of the next track arrives before its title: the strip empties, and nothing is looked up yet.
        r.advance(1_000.0);
        assert_eq!(r.poll(snap("Song", 100.0, true, 188_000.0)), Some(r.h.mono + CONFIRM_MS));
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![Ui::Clear], vec![false], 1));

        // The old lookup finishing now must not light the new timeline either.
        r.step(Some(Msg::Lyrics { key: old_key, result: Ok(Lyrics::Synced(song())) }));
        r.advance(300.0);
        r.poll(snap("Song", 500.0, true, 188_000.0));
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![], vec![], 1));

        // The title never changed: after the wait it is another track of the same name and is looked up.
        r.advance(CONFIRM_MS);
        assert_eq!(r.step(None), None);
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![], vec![], 2));
        assert_eq!(r.h.fetches.borrow()[1].1.duration_ms, 188_000);
    }

    #[test]
    fn spotify_order_shows_no_line_of_the_old_song_and_fetches_once() {
        let mut r = Rig::showing(song(), 6_000.0);
        r.advance(10_000.0);
        r.step(None);
        r.h.out();
        // The next song's timeline with the old title, then its title 475 ms later.
        r.poll(snap("Song", 0.0, true, 188_000.0));
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![Ui::Clear], vec![false], 1));
        r.advance(415.0);
        r.poll(snap("Next", 475.0, true, 188_000.0));
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![], vec![], 2));
        let (_, fetched) = r.h.fetches.borrow()[1].clone();
        assert_eq!((fetched.title.as_str(), fetched.duration_ms), ("Next", 188_000));

        // The confirming read has nothing left to decide.
        r.advance(CONFIRM_MS);
        r.h.snap = snap("Next", 1_100.0, true, 188_000.0);
        assert_eq!(r.step(None), None);
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![], vec![], 2));
        r.lyrics(Ok(Lyrics::Synced(vec![ln(0, "next one"), ln(4_000, "next two")])));
        assert_eq!(r.h.out(), [lines(3, None, "next one", Some("next two"), false, true)]);
    }

    #[test]
    fn restart_of_the_same_song_shows_lines_after_the_confirming_read() {
        let mut r = Rig::showing(song(), 100_000.0);
        r.advance(1_000.0);
        // A jump to the very start: maybe the next song, maybe a restart. Empty until the next read.
        let deadline = r.poll(play("Song", 0.0));
        assert_eq!(deadline, Some(r.h.mono + CONFIRM_MS));
        assert_eq!((r.h.out(), r.h.vis(), r.h.fetched()), (vec![Ui::Clear], vec![false], 1));

        // Events inside the wait change nothing.
        r.advance(200.0);
        assert_eq!(r.poll(play("Song", 260.0)), deadline);
        assert_eq!((r.h.out(), r.h.vis()), (vec![], vec![]));

        r.advance(CONFIRM_MS);
        r.h.snap = play("Song", 1_500.0);
        r.step(None);
        assert_eq!(r.h.fetched(), 1);
        assert_eq!(r.h.vis(), [true]);
        assert_eq!(r.h.out(), [lines(2, None, "one", Some("two"), false, true)]);
    }

    #[test]
    fn a_seek_away_from_the_start_shows_at_once() {
        let mut r = Rig::showing(song(), 100_000.0);
        r.advance(1_000.0);
        r.poll(play("Song", 6_000.0));
        assert_eq!(r.h.out(), [lines(2, Some("one"), "two", Some("three"), false, true)]);
    }

    #[test]
    fn duration_within_tolerance_keeps_lyrics() {
        let mut r = Rig::new();
        r.poll(snap("Song", 2_000.0, true, 207_000.0));
        r.lyrics(Ok(Lyrics::Synced(song())));
        r.h.out();
        r.poll(snap("Song", 2_200.0, true, 208_900.0));
        assert_eq!(r.h.fetched(), 1);
        assert_eq!(r.h.vis(), [true]);
        // 2.1 s off: another track, once the confirming read agrees.
        r.poll(snap("Song", 2_300.0, true, 209_100.0));
        assert_eq!(r.h.fetched(), 1);
        r.advance(CONFIRM_MS);
        r.step(None);
        assert_eq!(r.h.fetched(), 2);
    }

    #[test]
    fn the_last_line_is_followed_by_a_break_when_the_track_goes_on() {
        let mut r = Rig::new();
        r.poll(play("Song", 14_000.0));
        // "four" starts at 13 s and the track is 200 s long: the break comes 10 s after it, at 23 s.
        assert_eq!(r.lyrics(Ok(Lyrics::Synced(song()))), Some(1_060.0 + 9_000.0 + 1.0));
        assert_eq!(r.h.out(), [lines(1, Some("three"), "four", Some("♪"), false, true)]);

        // The track ends 7 s after its last line: no break.
        let mut r = Rig::new();
        r.poll(snap("Song", 14_000.0, true, 20_000.0));
        assert_eq!(r.lyrics(Ok(Lyrics::Synced(song()))), None);
        assert_eq!(r.h.out(), [lines(1, Some("three"), "four", None, false, true)]);
    }

    #[test]
    fn not_found_shows_info_then_hides_after_3s() {
        let mut r = Rig::new();
        r.poll(play("Song", 2_000.0));
        assert_eq!(r.lyrics(Ok(Lyrics::NotFound)), Some(4_060.0));
        assert_eq!(r.h.out(), [info("Song")]);
        assert_eq!(r.h.vis(), [true]);
        assert_eq!(r.e.cache.get(&track("Song"), r.secs()), Some(Lyrics::NotFound));

        // Events during the card change nothing and keep the deadline.
        r.advance(500.0);
        assert_eq!(r.poll(play("Song", 2_560.0)), Some(4_060.0));
        assert_eq!((r.h.out(), r.h.vis()), (vec![], vec![]));

        r.h.mono = 4_059.0;
        assert_eq!(r.step(None), Some(4_060.0));
        assert!(r.h.vis().is_empty());
        r.h.mono = 4_060.0;
        assert_eq!(r.step(None), None);
        assert_eq!((r.h.out(), r.h.vis()), (vec![Ui::Clear], vec![false]));
    }

    #[test]
    fn fetch_error_shows_info_and_is_not_cached() {
        let mut r = Rig::new();
        r.poll(play("Song", 2_000.0));
        assert_eq!(r.lyrics(Err("offline".into())), Some(4_060.0));
        assert_eq!(r.h.out(), [info("Song")]);
        assert_eq!(r.e.cache.get(&track("Song"), r.secs()), None);
        r.advance(3_000.0);
        r.step(None);
        assert_eq!(r.h.vis(), [true, false]);

        // The same track on the next event is not looked up again, or an LRCLIB outage would be hammered.
        r.poll(play("Song", 8_000.0));
        assert_eq!(r.h.fetched(), 1);
        // A later play of it is.
        r.poll(None);
        r.poll(play("Song", 0.0));
        assert_eq!(r.h.fetched(), 2);
    }

    #[test]
    fn not_found_is_cached() {
        let mut r = Rig::new();
        r.poll(play("Song", 2_000.0));
        r.lyrics(Ok(Lyrics::NotFound));
        r.advance(3_000.0);
        r.step(None);
        r.h.out();
        r.poll(None);
        r.poll(play("Song", 0.0));
        assert_eq!(r.h.fetched(), 1);
        assert_eq!(r.h.out(), [info("Song")]);
    }

    #[test]
    fn cache_hit_does_not_fetch() {
        let mut r = Rig::new();
        r.e.cache.put(&track("Song"), &Lyrics::Synced(song()), r.secs());
        assert_eq!(r.poll(play("Song", 2_000.0)), Some(4_061.0));
        assert_eq!(r.h.fetched(), 0);
        assert_eq!(r.h.out(), [lines(1, None, "one", Some("two"), false, true)]);
        assert_eq!(r.h.vis(), [true]);
    }

    #[test]
    fn found_lyrics_are_cached() {
        let mut r = Rig::new();
        r.poll(play("Song", 2_000.0));
        r.lyrics(Ok(Lyrics::Synced(song())));
        assert_eq!(r.e.cache.get(&track("Song"), r.secs()), Some(Lyrics::Synced(song())));
    }

    #[test]
    fn pause_emits_static_lines_and_no_deadline() {
        let mut r = Rig::showing(song(), 6_000.0);
        r.advance(500.0);
        assert_eq!(r.poll(pause("Song", 6_200.0)), None);
        assert_eq!(r.h.out(), [lines(2, Some("one"), "two", Some("three"), false, true)]);
    }

    #[test]
    fn resume_reschedules() {
        let mut r = Rig::showing(song(), 6_000.0);
        r.poll(pause("Song", 6_200.0));
        r.h.out();
        r.advance(1_000.0);
        let deadline = r.poll(play("Song", 6_200.0));
        // "three" starts at 9 s, 2.8 s from the position that was read.
        assert_eq!(deadline, Some(r.h.mono + 2_800.0 + 1.0));
        assert_eq!(r.h.out(), [lines(3, Some("one"), "two", Some("three"), false, true)]);
    }

    #[test]
    fn seek_emits_without_animation() {
        let mut r = Rig::showing(song(), 2_000.0);
        // Read at 2120: the clock expects 3060. 4800 is a jump of 1.7 s, still inside the first line.
        r.advance(1_000.0);
        r.poll(play("Song", 4_800.0));
        assert_eq!(r.h.out(), [lines(2, None, "one", Some("two"), false, true)]);
        // Read at 2230, before "two" is due: the clock expects 4910. 10 s is a jump into "three".
        r.advance(50.0);
        r.poll(play("Song", 10_000.0));
        assert_eq!(r.h.out(), [lines(3, Some("two"), "three", Some("four"), false, true)]);
    }

    #[test]
    fn small_drift_inside_the_line_emits_nothing() {
        let mut r = Rig::showing(song(), 2_000.0);
        r.advance(1_000.0);
        r.poll(play("Song", 2_900.0));
        assert!(r.h.out().is_empty());
    }

    #[test]
    fn deadline_not_replaced_for_small_drift() {
        let mut r = Rig::showing(song(), 2_000.0);
        // Read at 2120: the clock expects 3060. 10 ms behind moves "two" 10 ms later: kept.
        r.advance(1_000.0);
        assert_eq!(r.poll(play("Song", 3_050.0)), Some(4_061.0));
        // Read at 3180: the clock expects 4110. 50 ms behind moves "two" to 4121: replaced.
        r.advance(1_000.0);
        assert_eq!(r.poll(play("Song", 4_060.0)), Some(4_121.0));
    }

    #[test]
    fn hidden_by_user_emits_nothing_and_has_no_deadline() {
        let mut r = Rig::showing(song(), 2_000.0);
        assert_eq!(r.step(Some(Msg::SetVisible(false))), None);
        assert_eq!((r.h.out(), r.h.vis()), (vec![Ui::Clear], vec![false]));
        assert!(!r.e.store.load().visible);

        r.advance(1_000.0);
        assert_eq!(r.poll(play("Song", 3_000.0)), None);
        // Another track is still followed, so its lyrics are ready when the strip comes back.
        assert_eq!(r.poll(play("Other", 0.0)), None);
        assert_eq!(r.h.fetched(), 2);
        assert_eq!(r.lyrics(Ok(Lyrics::Synced(song()))), None);
        assert_eq!((r.h.out(), r.h.vis()), (vec![], vec![]));
    }

    #[test]
    fn hidden_not_found_card_is_not_shown() {
        let mut r = Rig::new();
        r.step(Some(Msg::SetVisible(false)));
        r.poll(play("Song", 0.0));
        assert_eq!(r.lyrics(Ok(Lyrics::NotFound)), None);
        assert_eq!((r.h.out(), r.h.vis()), (vec![], vec![]));
    }

    #[test]
    fn show_again_emits_current_state() {
        let mut r = Rig::showing(song(), 2_000.0);
        r.step(Some(Msg::ToggleVisible));
        r.advance(1_000.0);
        r.poll(play("Song", 7_000.0));
        assert_eq!(r.h.out(), [Ui::Clear]);

        let deadline = r.step(Some(Msg::ToggleVisible));
        assert!(r.e.store.load().visible);
        assert_eq!(r.h.out(), [lines(2, Some("one"), "two", Some("three"), false, true)]);
        assert_eq!(r.h.vis(), [false, true]);
        // "three" is 2 s away.
        assert_eq!(deadline, Some(r.h.mono + 2_000.0 + 1.0));
    }

    #[test]
    fn nudge_changes_offset_saves_and_toasts() {
        let mut r = Rig::showing(song(), 4_800.0);
        let key = r.h.last_key();
        // +250 puts the lyric time at 5050: "two" is current, and "three" is 3950 ms of playback away.
        assert_eq!(r.step(Some(Msg::Nudge(250))), Some(1_060.0 + 3_950.0 + 1.0));
        assert_eq!(r.h.out(), [toast("+250 ms"), lines(2, Some("one"), "two", Some("three"), false, true)]);
        assert_eq!(r.e.store.song_offset_ms(&key), 250);

        r.step(Some(Msg::Nudge(-500)));
        assert_eq!(r.h.out(), [toast("-250 ms"), lines(3, None, "one", Some("two"), false, true)]);
        assert_eq!(r.e.store.song_offset_ms(&key), -250);

        r.step(Some(Msg::Nudge(250)));
        assert_eq!(r.h.out()[0], toast("0 ms"));
        assert_eq!(r.e.store.song_offset_ms(&key), 0);
    }

    #[test]
    fn nudge_without_a_track_is_ignored() {
        let mut r = Rig::new();
        assert_eq!(r.step(Some(Msg::Nudge(250))), None);
        assert!(r.h.out().is_empty());
        assert!(!r.dir.join("offsets.json").exists());
    }

    #[test]
    fn saved_song_offset_is_used_on_the_next_play() {
        let mut r = Rig::new();
        r.poll(play("Song", 4_800.0));
        let key = r.h.last_key();
        r.e.store.set_song_offset_ms(&key, 250);
        r.poll(None);
        r.poll(play("Song", 4_800.0));
        r.lyrics(Ok(Lyrics::Synced(song())));
        assert_eq!(r.h.out(), [lines(1, Some("one"), "two", Some("three"), false, true)]);
    }

    #[test]
    fn app_offset_is_applied() {
        let mut settings = Settings::default();
        settings.app_offsets_ms.insert("Spotify.exe".into(), 200);
        let mut r = Rig::with(settings.clone(), None);
        r.poll(play("Song", 4_800.0));
        r.lyrics(Ok(Lyrics::Synced(song())));
        assert_eq!(r.h.out(), [lines(1, Some("one"), "two", Some("three"), false, true)]);
        // "three" at 9 s minus the 200 ms offset, 4 s of playback from the read at 1060.
        assert_eq!(r.step(None), Some(1_060.0 + 4_000.0 + 1.0));

        let mut other = Rig::with(settings, None);
        let mut chrome = play("Song", 4_800.0).unwrap();
        chrome.app = "chrome.exe".into();
        other.poll(Some(chrome));
        other.lyrics(Ok(Lyrics::Synced(song())));
        assert_eq!(other.h.out(), [lines(1, None, "one", Some("two"), false, true)]);
    }

    #[test]
    fn closed_session_hides_overlay() {
        let mut r = Rig::showing(song(), 2_000.0);
        let mut closed = play("Song", 2_000.0).unwrap();
        closed.status = Status::Closed;
        assert_eq!(r.poll(Some(closed)), None);
        assert_eq!((r.h.out(), r.h.vis()), (vec![Ui::Clear], vec![false]));

        let mut r = Rig::showing(song(), 2_000.0);
        assert_eq!(r.poll(None), None);
        assert_eq!((r.h.out(), r.h.vis()), (vec![Ui::Clear], vec![false]));
        let mut stopped = play("Song", 2_000.0).unwrap();
        stopped.status = Status::Stopped;
        r.poll(Some(stopped));
        assert!(r.h.vis().is_empty());
    }

    #[test]
    fn instrumental_shows_note() {
        let mut r = Rig::new();
        r.poll(play("Song", 2_000.0));
        assert_eq!(r.lyrics(Ok(Lyrics::Instrumental)), None);
        assert_eq!(r.h.out(), [lines(1, None, "♪", None, false, true)]);
        assert_eq!(r.h.vis(), [true]);
    }

    #[test]
    fn intro_break_shows_note_then_first_line() {
        let mut r = Rig::new();
        r.poll(play("Song", 1_000.0));
        let deadline = r.lyrics(Ok(Lyrics::Synced(vec![ln(0, ""), ln(8_000, "one"), ln(12_000, "two")])));
        assert_eq!(deadline, Some(1_060.0 + 7_000.0 + 1.0));
        assert_eq!(r.h.out(), [lines(1, None, "♪", Some("one"), false, true)]);
        r.advance(7_001.0);
        r.step(None);
        assert_eq!(r.h.out(), [lines(2, Some("♪"), "one", Some("two"), true, true)]);
    }

    #[test]
    fn before_the_first_line_it_is_dim_then_lit() {
        let mut r = Rig::new();
        r.poll(play("Song", 1_000.0));
        assert_eq!(r.lyrics(Ok(Lyrics::Synced(vec![ln(3_000, "one"), ln(6_000, "two")]))), Some(3_061.0));
        assert_eq!(r.h.out(), [lines(1, None, "one", Some("two"), false, false)]);
        r.advance(2_001.0);
        assert_eq!(r.step(None), Some(6_061.0));
        assert_eq!(r.h.out(), [lines(2, None, "one", Some("two"), false, true)]);
    }

    fn word(start_ms: u32, text: &str) -> Word {
        Word { start_ms, text: text.into() }
    }

    fn uw(text: &str, delay_ms: i32, dur_ms: u32) -> UiWord {
        UiWord { text: text.into(), delay_ms, dur_ms }
    }

    #[test]
    fn word_delays_are_relative_to_position_and_rate() {
        let sung = Line {
            start_ms: 4_000,
            text: "hello big world".into(),
            words: vec![word(4_000, "hello "), word(4_500, "big "), word(5_200, "world")],
        };
        let at_double_speed = |pos| {
            let mut s = play("Song", pos).unwrap();
            s.report.rate = Some(2.0);
            Some(s)
        };
        let mut r = Rig::new();
        r.poll(at_double_speed(4_600.0));
        let deadline = r.lyrics(Ok(Lyrics::Synced(vec![ln(1_000, "intro"), sung.clone(), ln(7_000, "outro")])));
        // 2.4 s of lyrics to "outro" pass in 1.2 s at double speed.
        assert_eq!(deadline, Some(1_060.0 + 1_200.0 + 1.0));
        let words = Some(vec![uw("hello ", -300, 250), uw("big ", -50, 350), uw("world", 300, 900)]);
        assert_eq!(
            r.h.out(),
            [Ui::Lines {
                seq: 1,
                prev: Some("intro".into()),
                cur: "hello big world".into(),
                next: Some("outro".into()),
                words,
                animate: false,
                lit: true
            }]
        );

        // Paused: no word timing, the whole line lit.
        let mut paused = at_double_speed(4_600.0).unwrap();
        paused.status = Status::Paused;
        paused.report.playing = false;
        r.poll(Some(paused));
        let out = r.h.out();
        assert!(matches!(&out[..], [Ui::Lines { words: None, lit: true, .. }]), "{out:?}");

        // The last word of the last line, when the track ends soon after it, gets 500 ms of lyric time.
        let mut r = Rig::new();
        let mut short = at_double_speed(4_600.0).unwrap();
        short.report.duration_ms = 10_000.0;
        r.poll(Some(short));
        r.lyrics(Ok(Lyrics::Synced(vec![sung])));
        let Ui::Lines { words: Some(words), .. } = r.h.out().remove(0) else { panic!("no words") };
        assert_eq!(words[2], uw("world", 300, 250));
    }

    #[test]
    fn empty_title_waits() {
        let mut r = Rig::new();
        assert_eq!(r.poll(play("", 0.0)), None);
        assert_eq!(r.poll(snap("Song", 0.0, true, 0.0)), None);
        assert_eq!((r.h.fetched(), r.h.out(), r.h.vis()), (0, vec![], vec![]));
        r.poll(play("Song", 0.0));
        assert_eq!(r.h.fetched(), 1);
    }

    #[test]
    fn a_line_beyond_the_track_end_gets_no_deadline() {
        let mut r = Rig::new();
        r.poll(play("Song", 2_000.0));
        assert_eq!(r.lyrics(Ok(Lyrics::Synced(vec![ln(1_000, "one"), ln(250_000, "late")]))), None);
    }

    #[test]
    fn timing_log_marks_timer_emits_apart_from_pause() {
        #[derive(Clone, Default)]
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let out = Shared::default();
        let mut r = Rig::with(Settings::default(), Some(TimingLog::new(Box::new(out.clone()))));
        r.poll(play("Song", 2_000.0));
        let deadline = r.lyrics(Ok(Lyrics::Synced(song()))).unwrap();
        // The line timer fires: "two" follows "one" (seq 2). A pause then re-emits the same line (seq 3).
        r.h.mono = deadline;
        r.step(None);
        r.step(Some(Msg::Shown { seq: 2 }));
        r.poll(pause("Song", 5_100.0));
        r.step(Some(Msg::Shown { seq: 3 }));
        let log = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        let extras: Vec<&str> = log
            .lines()
            .filter(|l| l.starts_with("emit,") || l.starts_with("shown,"))
            .map(|l| l.rsplit(',').next().unwrap())
            .collect();
        assert_eq!(extras, ["1 other", "2 timer", "2 timer", "3 other", "3 other"]);
    }

    #[test]
    fn timing_log_rows_for_sync_emit_and_shown() {
        #[derive(Clone, Default)]
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let out = Shared::default();
        let mut r = Rig::with(Settings::default(), Some(TimingLog::new(Box::new(out.clone()))));
        r.poll(play("Song", 2_000.0));
        r.lyrics(Ok(Lyrics::Synced(song())));
        r.advance(10.0);
        r.step(Some(Msg::Shown { seq: 1 }));
        // A second sync, 5 ms behind the clock.
        r.advance(990.0);
        r.poll(play("Song", 3_055.0));
        let w = WALL;
        let expected = format!(
            "sync,{},Spotify.exe,Song,,,2000,first\n\
             emit,{},Spotify.exe,Song,0,1000,2000,1 other\n\
             shown,{},Spotify.exe,Song,0,1000,2010,1 other\n\
             sync,{},Spotify.exe,Song,0,1000,3055,-5\n",
            w + 60,
            w + 60,
            w + 70,
            w + 1_120
        );
        assert_eq!(String::from_utf8(out.0.lock().unwrap().clone()).unwrap(), expected);
    }
}
