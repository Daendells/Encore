//! SMTC watcher: follows one Windows media session and reads it as a `Snapshot`.
//!
//! Everything WinRT lives in `Watcher`, which belongs to the thread that created it (the engine
//! thread). Events only call `on_change`; the engine then calls `read`.

use std::sync::{mpsc, Arc};
use std::time::Duration;

use windows::core::{Result, RuntimeType, HSTRING};
use windows_future::AsyncOperationCompletedHandler;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Media::Control::{
    CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Props, MediaPropertiesChangedEventArgs,
    PlaybackInfoChangedEventArgs, SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::clock::Report;

/// How long `read` waits for the title, artist and album before giving up on them.
const MEDIA_TIMEOUT: Duration = Duration::from_millis(500);
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Closed,
    Opened,
    Changing,
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// False when the title, artist and album are the last good read, because this one failed or timed out.
    pub media_fresh: bool,
    pub status: Status,
    pub report: Report,
}

/// WinRT `DateTime` ticks (100 ns since 1601-01-01 UTC) to Unix epoch ms. Zero means "never set".
pub fn datetime_to_unix_ms(universal_time: i64) -> Option<i64> {
    if universal_time == 0 {
        return None;
    }
    universal_time.checked_sub(UNIX_EPOCH_TICKS).map(|t| t.div_euclid(10_000))
}

/// Index of the session to follow: a Playing one (the current one first), else the current one,
/// else the first.
pub fn pick(sessions: &[(Status, bool)]) -> Option<usize> {
    let playing = |s: &(Status, bool)| s.0 == Status::Playing;
    sessions
        .iter()
        .position(|s| playing(s) && s.1)
        .or_else(|| sessions.iter().position(playing))
        .or_else(|| sessions.iter().position(|s| s.1))
        .or(if sessions.is_empty() { None } else { Some(0) })
}

fn status_from(value: i32) -> Status {
    match value {
        1 => Status::Opened,
        2 => Status::Changing,
        3 => Status::Stopped,
        4 => Status::Playing,
        5 => Status::Paused,
        _ => Status::Closed,
    }
}

fn app_id(s: &Session) -> Option<String> {
    s.SourceAppUserModelId().ok().map(|a| a.to_string_lossy())
}

fn status_of(s: &Session) -> Option<Status> {
    let status = s.GetPlaybackInfo().ok()?.PlaybackStatus().ok()?;
    Some(status_from(status.0))
}

fn text(r: Result<HSTRING>) -> String {
    r.map(|h| h.to_string_lossy()).unwrap_or_default()
}

/// Title, artist and album. None when the call fails or takes longer than `MEDIA_TIMEOUT`.
fn read_media(s: &Session) -> Option<[String; 3]> {
    let (tx, rx) = mpsc::channel();
    let started = s.TryGetMediaPropertiesAsync().and_then(|op| {
        op.SetCompleted(&AsyncOperationCompletedHandler::new(move |op, _status| {
            let texts = op
                .ok()
                .and_then(|op| op.GetResults())
                .map(|m: Props| [text(m.Title()), text(m.Artist()), text(m.AlbumTitle())])
                .ok();
            // The receiver is gone after a timeout; nobody is waiting for this any more.
            let _ = tx.send(texts);
            Ok(())
        }))
    });
    if started.is_err() {
        return None;
    }
    rx.recv_timeout(MEDIA_TIMEOUT).ok().flatten()
}

type Media = [String; 3];

/// The fresh read, else the last good one of the same session (a slow read must not blank the title),
/// else empty strings. A fresh read becomes the last one.
fn media_or_last<S: PartialEq + Clone>(fresh: Option<Media>, session: &S, last: &mut Option<(S, Media)>) -> Media {
    match fresh {
        Some(media) => {
            *last = Some((session.clone(), media.clone()));
            media
        }
        None => last.as_ref().filter(|(s, _)| s == session).map(|(_, media)| media.clone()).unwrap_or_default(),
    }
}

fn ticks_to_ms(ticks: i64) -> f64 {
    ticks as f64 / 10_000.0
}

fn read_session(s: &Session, last: &mut Option<(Session, Media)>) -> Option<Snapshot> {
    // The slow read first, so the position below is as fresh as possible when it is returned.
    let fresh = read_media(s);
    let media_fresh = fresh.is_some();
    let [title, artist, album] = media_or_last(fresh, s, last);
    let timeline = s.GetTimelineProperties().ok()?;
    let playback = s.GetPlaybackInfo().ok()?;

    let ticks = |r: Result<TimeSpan>| r.map(|t| t.Duration).unwrap_or_default();
    let start = ticks(timeline.StartTime());
    let end = ticks(timeline.EndTime());
    let status = status_from(playback.PlaybackStatus().map(|v| v.0).unwrap_or_default());
    Some(Snapshot {
        app: app_id(s).unwrap_or_default(),
        title,
        artist,
        album,
        media_fresh,
        status,
        report: Report {
            position_ms: ticks_to_ms(ticks(timeline.Position()).saturating_sub(start)),
            last_updated_wall_ms: timeline
                .LastUpdatedTime()
                .ok()
                .and_then(|d| datetime_to_unix_ms(d.UniversalTime)),
            // A null IReference comes back as Err: the rate is absent.
            rate: playback.PlaybackRate().and_then(|r| r.Value()).ok().filter(|r| r.is_finite()),
            playing: status == Status::Playing,
            duration_ms: ticks_to_ms(end.saturating_sub(start)).max(0.0),
        },
    })
}

type OnChange = Arc<dyn Fn() + Send + Sync>;

fn signal<S: RuntimeType + 'static, A: RuntimeType + 'static>(
    on_change: &OnChange,
) -> TypedEventHandler<S, A> {
    let on_change = on_change.clone();
    TypedEventHandler::new(move |_, _| {
        on_change();
        Ok(())
    })
}

/// A session we are subscribed to, with the tokens to undo it (media, playback, timeline).
struct Tracked {
    session: Session,
    tokens: [Option<i64>; 3],
}

impl Tracked {
    fn new(session: Session, on_change: &OnChange) -> Tracked {
        let tokens = [
            session
                .MediaPropertiesChanged(&signal::<_, MediaPropertiesChangedEventArgs>(on_change))
                .ok(),
            session
                .PlaybackInfoChanged(&signal::<_, PlaybackInfoChangedEventArgs>(on_change))
                .ok(),
            session
                .TimelinePropertiesChanged(&signal::<_, TimelinePropertiesChangedEventArgs>(on_change))
                .ok(),
        ];
        Tracked { session, tokens }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        // Errors ignored: a session that is gone may refuse.
        let [media, playback, timeline] = self.tokens;
        if let Some(t) = media {
            let _ = self.session.RemoveMediaPropertiesChanged(t);
        }
        if let Some(t) = playback {
            let _ = self.session.RemovePlaybackInfoChanged(t);
        }
        if let Some(t) = timeline {
            let _ = self.session.RemoveTimelinePropertiesChanged(t);
        }
    }
}

pub struct Watcher {
    manager: Manager,
    on_change: OnChange,
    manager_tokens: [i64; 2],
    tracked: Vec<Tracked>,
    /// The last title, artist and album read in full, and the session they came from.
    last_media: Option<(Session, Media)>,
}

impl Watcher {
    /// `on_change` is called from WinRT threads on every manager or session event. It must be cheap.
    /// Puts the calling thread in the MTA, so create the watcher and call `read` on the same thread.
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Result<Watcher> {
        // S_FALSE (already initialised) is Ok. A thread already in an STA stays there.
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Err(e) if e.code() != RPC_E_CHANGED_MODE => return Err(e),
            _ => {}
        }
        let on_change: OnChange = Arc::new(on_change);
        let manager = Manager::RequestAsync()?.join()?;
        // Manager events first, so a session that appears during startup is not missed.
        let manager_tokens = [
            manager.SessionsChanged(&signal::<_, SessionsChangedEventArgs>(&on_change))?,
            manager.CurrentSessionChanged(&signal::<_, CurrentSessionChangedEventArgs>(&on_change))?,
        ];
        let mut watcher = Watcher { manager, on_change, manager_tokens, tracked: Vec::new(), last_media: None };
        watcher.sync_sessions();
        Ok(watcher)
    }

    /// Re-enumerates sessions, fixes subscriptions, and reads the followed session (spec B4). None = no session.
    pub fn read(&mut self) -> Option<Snapshot> {
        self.sync_sessions();
        let current = self.manager.GetCurrentSession().ok();
        let infos: Vec<(Status, bool)> = self
            .tracked
            .iter()
            // Two sessions of one app (two browser windows) are told apart by identity, not by app id.
            .map(|t| (status_of(&t.session).unwrap_or(Status::Closed), current.as_ref() == Some(&t.session)))
            .collect();
        read_session(&self.tracked[pick(&infos)?].session, &mut self.last_media)
    }

    /// Subscribes to new sessions and drops (and so unsubscribes from) the ones that are gone.
    fn sync_sessions(&mut self) {
        // On failure keep what we have.
        let Ok(now) = self.manager.GetSessions() else { return };
        let mut old = std::mem::take(&mut self.tracked);
        for s in now {
            // Matched by COM identity.
            let tracked = match old.iter().position(|t| t.session == s) {
                Some(i) => old.swap_remove(i),
                None => Tracked::new(s, &self.on_change),
            };
            self.tracked.push(tracked);
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let [sessions, current] = self.manager_tokens;
        let _ = self.manager.RemoveSessionsChanged(sessions);
        let _ = self.manager.RemoveCurrentSessionChanged(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::{Paused, Playing, Stopped};

    #[test]
    fn datetime_converts_from_the_1601_epoch() {
        assert_eq!(datetime_to_unix_ms(0), None);
        assert_eq!(datetime_to_unix_ms(116_444_736_000_000_000), Some(0));
        assert_eq!(datetime_to_unix_ms(132_223_104_000_000_000), Some(1_577_836_800_000));
        assert_eq!(datetime_to_unix_ms(i64::MIN), None);
    }

    #[test]
    fn status_from_winrt_values() {
        assert_eq!(status_from(4), Playing);
        assert_eq!(status_from(5), Paused);
        assert_eq!(status_from(3), Stopped);
        assert_eq!(status_from(99), Status::Closed);
    }

    #[test]
    fn pick_prefers_current_playing() {
        assert_eq!(pick(&[(Playing, false), (Playing, true)]), Some(1));
    }

    #[test]
    fn pick_prefers_any_playing_over_paused_current() {
        assert_eq!(pick(&[(Paused, true), (Playing, false)]), Some(1));
    }

    #[test]
    fn pick_falls_back_to_current() {
        assert_eq!(pick(&[(Paused, false), (Stopped, true)]), Some(1));
    }

    fn media(title: &str) -> [String; 3] {
        [title.to_string(), "Artist".to_string(), String::new()]
    }

    #[test]
    fn failed_media_read_reuses_the_last_one_of_the_same_session() {
        let mut last = None;
        assert_eq!(media_or_last(Some(media("A")), &1, &mut last), media("A"));
        // The call failed or timed out: the same session keeps its title, another session gets nothing.
        assert_eq!(media_or_last(None, &1, &mut last), media("A"));
        assert_eq!(media_or_last(None, &2, &mut last), <[String; 3]>::default());
        // A fresh read replaces it.
        assert_eq!(media_or_last(Some(media("B")), &2, &mut last), media("B"));
        assert_eq!(media_or_last(None, &2, &mut last), media("B"));
        assert_eq!(media_or_last(None, &1, &mut last), <[String; 3]>::default());
    }

    #[test]
    fn pick_empty_is_none() {
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn pick_no_current_no_playing_is_first() {
        assert_eq!(pick(&[(Paused, false), (Stopped, false)]), Some(0));
    }

    /// Reads whatever is playing now; starts, pauses and controls nothing.
    #[test]
    #[ignore = "needs the Windows media session service"]
    fn live_read_prints_snapshot() {
        let mut watcher = Watcher::start(|| {}).expect("start the watcher");
        println!("{:#?}", watcher.read());
    }
}
