use crate::lrc::{fill_gaps, parse, Line};
use crate::matcher::{best, score, Candidate, Track};
use crate::normalize::{candidates, key};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Lyrics {
    Synced(Vec<Line>),
    Instrumental,
    NotFound,
}

#[derive(Debug)]
pub enum FetchError {
    Network(String),
    BadResponse(String),
}

pub trait Fetch {
    fn get(&self, title: &str, artist: &str, album: &str, duration_s: u32) -> Result<Option<Candidate>, FetchError>;
    fn search(&self, title: &str, artist: &str) -> Result<Vec<Candidate>, FetchError>;
}

/// Returns the lyrics and the accepted candidate (None when not found), which the eval runner prints.
///
/// Stops at the first accepted candidate. An error is returned only when every call failed: one answer
/// is enough to say that nothing was found.
pub fn resolve(track: &Track, fetch: &dyn Fetch) -> Result<(Lyrics, Option<Candidate>), FetchError> {
    // The matcher accepts nothing for a pair with an empty key, so such a pair is not looked up.
    let pairs: Vec<(String, String)> = candidates(&track.title, &track.artist)
        .into_iter()
        .filter(|(title, artist)| !key(title).is_empty() && !key(artist).is_empty())
        .collect();
    let duration_s = track.duration_ms / 1000 + u32::from(track.duration_ms % 1000 >= 500);
    let mut answered = false;
    let mut failed = None;

    for (i, (title, artist)) in pairs.iter().enumerate() {
        // LRCLIB matches the album exactly, so only the most likely pair is narrowed by it.
        let album = if i == 0 { track.album.as_str() } else { "" };
        match fetch.get(title, artist, album, duration_s) {
            Ok(Some(c)) if score(title, artist, track.duration_ms, &c).is_some() => return Ok(accept(c)),
            Ok(_) => answered = true,
            Err(e) => failed = Some(e),
        }
    }
    for (title, artist) in &pairs {
        match fetch.search(title, artist) {
            Ok(found) => {
                if let Some(c) = best(track, &found) {
                    return Ok(accept(c.clone()));
                }
                answered = true;
            }
            Err(e) => failed = Some(e),
        }
    }
    match failed {
        Some(e) if !answered => Err(e),
        _ => Ok((Lyrics::NotFound, None)),
    }
}

fn accept(c: Candidate) -> (Lyrics, Option<Candidate>) {
    if c.instrumental {
        return (Lyrics::Instrumental, Some(c));
    }
    let lines = fill_gaps(parse(c.synced_lyrics.as_deref().unwrap_or_default()));
    // Lines without any text are only breaks: there is nothing to sing along to.
    if lines.iter().all(|line| line.text.is_empty()) {
        (Lyrics::NotFound, None)
    } else {
        (Lyrics::Synced(lines), Some(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lrc::{fill_gaps, parse};
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const LRC: &str = "[00:01.00] satu\n[00:03.00] dua";

    /// Answers from its scripts in order and records every call. A call past the script finds nothing.
    #[derive(Default)]
    struct Fake {
        gets: RefCell<VecDeque<Result<Option<Candidate>, FetchError>>>,
        searches: RefCell<VecDeque<Result<Vec<Candidate>, FetchError>>>,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(
            gets: Vec<Result<Option<Candidate>, FetchError>>,
            searches: Vec<Result<Vec<Candidate>, FetchError>>,
        ) -> Self {
            Fake { gets: RefCell::new(gets.into()), searches: RefCell::new(searches.into()), ..Fake::default() }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Fetch for Fake {
        fn get(
            &self,
            title: &str,
            artist: &str,
            album: &str,
            duration_s: u32,
        ) -> Result<Option<Candidate>, FetchError> {
            self.calls.borrow_mut().push(format!("get {title}|{artist}|{album}|{duration_s}"));
            self.gets.borrow_mut().pop_front().unwrap_or(Ok(None))
        }

        fn search(&self, title: &str, artist: &str) -> Result<Vec<Candidate>, FetchError> {
            self.calls.borrow_mut().push(format!("search {title}|{artist}"));
            self.searches.borrow_mut().pop_front().unwrap_or(Ok(Vec::new()))
        }
    }

    fn cand(id: u64, title: &str, artist: &str, duration: f64) -> Candidate {
        Candidate {
            id,
            track_name: title.to_string(),
            artist_name: artist.to_string(),
            duration,
            instrumental: false,
            synced_lyrics: Some(LRC.to_string()),
        }
    }

    fn track(title: &str, artist: &str, album: &str, duration_ms: u32) -> Track {
        Track { title: title.to_string(), artist: artist.to_string(), album: album.to_string(), duration_ms }
    }

    fn yellow() -> Track {
        track("Yellow", "Coldplay", "Parachutes", 266_773)
    }

    fn down() -> FetchError {
        FetchError::Network("offline".to_string())
    }

    #[test]
    fn get_hit_is_used_and_search_not_called() {
        let fake = Fake::new(vec![Ok(Some(cand(1, "Yellow", "Coldplay", 267.0)))], vec![]);
        let (lyrics, accepted) = resolve(&yellow(), &fake).unwrap();
        assert_eq!(lyrics, Lyrics::Synced(fill_gaps(parse(LRC))));
        assert_eq!(accepted.map(|c| c.id), Some(1));
        assert_eq!(fake.calls(), ["get Yellow|Coldplay|Parachutes|267"]);
    }

    #[test]
    fn get_hit_rejected_by_score_falls_back_to_search() {
        let longer = cand(1, "Yellow", "Coldplay", 277.0);
        let fake = Fake::new(vec![Ok(Some(longer))], vec![Ok(vec![cand(2, "Yellow", "Coldplay", 267.0)])]);
        let (lyrics, accepted) = resolve(&yellow(), &fake).unwrap();
        assert!(matches!(lyrics, Lyrics::Synced(_)));
        assert_eq!(accepted.map(|c| c.id), Some(2));
        assert_eq!(fake.calls(), ["get Yellow|Coldplay|Parachutes|267", "search Yellow|Coldplay"]);
    }

    #[test]
    fn search_picks_best_and_ignores_wrong_song() {
        let found = vec![
            cand(1, "Clocks", "Coldplay", 267.0),
            cand(2, "Yellow", "Someone Else", 267.0),
            cand(3, "Yellow", "Coldplay", 268.5),
            cand(4, "Yellow", "Coldplay", 267.0),
        ];
        let fake = Fake::new(vec![], vec![Ok(found)]);
        let (_, accepted) = resolve(&yellow(), &fake).unwrap();
        assert_eq!(accepted.map(|c| c.id), Some(4));
    }

    #[test]
    fn nothing_matches_is_not_found() {
        let found = vec![cand(1, "Clocks", "Coldplay", 267.0), cand(2, "Yellow", "Someone Else", 267.0)];
        let fake = Fake::new(vec![], vec![Ok(found)]);
        assert_eq!(resolve(&yellow(), &fake).unwrap(), (Lyrics::NotFound, None));
        assert_eq!(fake.calls(), ["get Yellow|Coldplay|Parachutes|267", "search Yellow|Coldplay"]);
    }

    #[test]
    fn instrumental_candidate() {
        let mut flagged = cand(1, "Yellow", "Coldplay", 267.0);
        flagged.instrumental = true;
        flagged.synced_lyrics = None;
        let fake = Fake::new(vec![Ok(Some(flagged))], vec![]);
        let (lyrics, accepted) = resolve(&yellow(), &fake).unwrap();
        assert_eq!(lyrics, Lyrics::Instrumental);
        assert_eq!(accepted.map(|c| c.id), Some(1));
    }

    #[test]
    fn synced_lyrics_that_parse_to_nothing_is_not_found() {
        for unusable in ["words without a timestamp", "[00:01.00]\n[00:05.00]"] {
            let mut c = cand(1, "Yellow", "Coldplay", 267.0);
            c.synced_lyrics = Some(unusable.to_string());
            let fake = Fake::new(vec![Ok(Some(c))], vec![]);
            assert_eq!(resolve(&yellow(), &fake).unwrap(), (Lyrics::NotFound, None), "{unusable}");
            assert_eq!(fake.calls().len(), 1, "{unusable}");
        }
    }

    #[test]
    fn network_error_everywhere_is_err() {
        let fake = Fake::new(vec![Err(down())], vec![Err(down())]);
        assert!(matches!(resolve(&yellow(), &fake), Err(FetchError::Network(_))));
        assert_eq!(fake.calls().len(), 2);
    }

    #[test]
    fn error_on_get_but_search_matches_is_ok() {
        let fake = Fake::new(vec![Err(down())], vec![Ok(vec![cand(2, "Yellow", "Coldplay", 267.0)])]);
        let (lyrics, accepted) = resolve(&yellow(), &fake).unwrap();
        assert!(matches!(lyrics, Lyrics::Synced(_)));
        assert_eq!(accepted.map(|c| c.id), Some(2));
    }

    #[test]
    fn error_on_one_call_without_a_match_is_not_found() {
        let fake = Fake::new(vec![Ok(None)], vec![Err(down())]);
        assert_eq!(resolve(&yellow(), &fake).unwrap(), (Lyrics::NotFound, None));
    }

    #[test]
    fn youtube_title_pair_is_tried() {
        let title = "Keane - Somewhere Only We Know (Official Music Video)";
        let playing = track(title, "Some Channel", "Uploads", 237_400);
        let fake = Fake::new(vec![Ok(None), Ok(Some(cand(1, "Somewhere Only We Know", "Keane", 237.0)))], vec![]);
        let (_, accepted) = resolve(&playing, &fake).unwrap();
        assert_eq!(accepted.map(|c| c.id), Some(1));
        // The album goes only with the first pair.
        assert_eq!(
            fake.calls(),
            ["get Keane - Somewhere Only We Know|Some Channel|Uploads|237", "get Somewhere Only We Know|Keane||237"]
        );
    }

    #[test]
    fn search_stops_at_the_first_pair_that_matches() {
        let playing = track("Keane - Somewhere Only We Know (Official Music Video)", "Some Channel", "", 237_400);
        let fake = Fake::new(vec![], vec![Ok(vec![cand(1, "Somewhere Only We Know", "Keane", 237.0)])]);
        let (_, accepted) = resolve(&playing, &fake).unwrap();
        assert_eq!(accepted.map(|c| c.id), Some(1));
        assert_eq!(
            fake.calls(),
            [
                "get Keane - Somewhere Only We Know|Some Channel||237",
                "get Somewhere Only We Know|Keane||237",
                "search Keane - Somewhere Only We Know|Some Channel"
            ]
        );
    }

    #[test]
    fn track_that_cannot_match_is_not_looked_up() {
        let long = "a".repeat(301);
        for (title, artist) in [("My video", ""), ("!!!", "Artist"), (long.as_str(), "Artist")] {
            let playing = track(title, artist, "", 60_000);
            let fake = Fake::new(vec![], vec![]);
            assert_eq!(resolve(&playing, &fake).unwrap(), (Lyrics::NotFound, None));
            assert_eq!(fake.calls(), [] as [&str; 0]);
        }
    }
}
