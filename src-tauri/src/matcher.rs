use crate::normalize::{candidates, clean_artist, clean_title, key, too_long};

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: u64,
    pub track_name: String,
    pub artist_name: String,
    pub duration: f64,
    #[serde(default)]
    pub instrumental: bool,
    pub synced_lyrics: Option<String>,
}

pub const MAX_DURATION_DIFF_S: f64 = 2.0;
pub const TITLE_MIN: f64 = 0.85;
pub const ARTIST_MIN: f64 = 0.80;
/// A title that is not equal passes only when the shorter key has at least this many characters ...
pub const TYPO_MIN_LEN: usize = 12;
/// ... and the keys are at most this many edits apart.
pub const TYPO_MAX_EDITS: usize = 2;

/// None = rejected. Some(score) in 0..=1, higher is better.
pub fn score(title: &str, artist: &str, duration_ms: u32, c: &Candidate) -> Option<f64> {
    let playing = side(title, artist)?;
    let (theirs, diff) = admit(duration_ms, c)?;
    compare(&playing, &theirs, diff).map(|(_, score)| score)
}

/// Compared left to right: same cleaned title, same title as it was sent, has synced lyrics, score.
type Rank = (bool, bool, bool, f64);

pub fn best<'a>(track: &Track, cs: &'a [Candidate]) -> Option<&'a Candidate> {
    let pairs = candidates(&track.title, &track.artist);
    let sides: Vec<Side> = pairs.iter().filter_map(|(title, artist)| side(title, artist)).collect();
    let raw = key(&track.title);
    let mut best: Option<(Rank, &Candidate)> = None;
    for c in cs {
        let Some((theirs, diff)) = admit(track.duration_ms, c) else { continue };
        let same_raw = key(&c.track_name) == raw;
        for playing in &sides {
            let Some((exact, score)) = compare(playing, &theirs, diff) else { continue };
            let rank: Rank = (exact, same_raw, has_synced(c), score);
            // Strictly greater: of equal ranks the first candidate wins.
            if best.is_none_or(|(top, _)| rank > top) {
                best = Some((rank, c));
            }
        }
    }
    best.map(|(_, c)| c)
}

/// A title and artist, cleaned and keyed.
struct Side {
    title: String,
    artist: String,
    names: Vec<String>,
}

/// None when a string is too long or a key is empty.
fn side(title: &str, artist: &str) -> Option<Side> {
    if too_long(title) || too_long(artist) {
        return None;
    }
    let artist = clean_artist(artist);
    let side = Side { title: key(&clean_title(title)), artist: key(&artist), names: artist_names(&artist) };
    (!side.title.is_empty() && !side.artist.is_empty()).then_some(side)
}

fn has_synced(c: &Candidate) -> bool {
    c.synced_lyrics.as_deref().is_some_and(|l| !l.trim().is_empty())
}

/// The cleaned candidate and its duration difference, when it has the right length and something to show.
fn admit(duration_ms: u32, c: &Candidate) -> Option<(Side, f64)> {
    let diff = (c.duration - f64::from(duration_ms) / 1000.0).abs();
    // Written as "within" so that a NaN duration is rejected too.
    let usable = diff <= MAX_DURATION_DIFF_S && (has_synced(c) || c.instrumental);
    if !usable {
        return None;
    }
    Some((side(&c.track_name, &c.artist_name)?, diff))
}

/// (the titles are equal, score). None = not the same song.
fn compare(playing: &Side, theirs: &Side, diff: f64) -> Option<(bool, f64)> {
    let title_sim = title_similarity(&playing.title, &theirs.title)?;
    let artist_sim = if theirs.names.iter().any(|n| playing.names.contains(n)) {
        1.0
    } else {
        strsim::normalized_levenshtein(&playing.artist, &theirs.artist)
    };
    (artist_sim >= ARTIST_MIN).then(|| {
        let score = 0.6 * title_sim + 0.3 * artist_sim + 0.1 * (1.0 - diff / MAX_DURATION_DIFF_S);
        (playing.title == theirs.title, score)
    })
}

/// Equal keys pass. Otherwise only a long ASCII title with at most two typos and the same numbers passes:
/// one changed character is another song in "Symphony No. 5" / "No. 6" and in any CJK title.
fn title_similarity(a: &str, b: &str) -> Option<f64> {
    if a == b {
        return Some(1.0);
    }
    let digit_runs = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_ascii_digit()).filter(|run| !run.is_empty()).map(str::to_owned).collect()
    };
    let sim = strsim::normalized_levenshtein(a, b);
    let typo = a.is_ascii()
        && b.is_ascii()
        && a.len().min(b.len()) >= TYPO_MIN_LEN
        && strsim::levenshtein(a, b) <= TYPO_MAX_EDITS
        && sim >= TITLE_MIN
        && digit_runs(a) == digit_runs(b);
    typo.then_some(sim)
}

/// The keyed names of an artist list such as "Dewa 19, Ari Lasso".
fn artist_names(list: &str) -> Vec<String> {
    let commas = list.replace(" & ", ",").replace(" x ", ",");
    commas.split([',', ';', '、']).map(key).filter(|k| !k.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: u64, title: &str, artist: &str, duration: f64) -> Candidate {
        Candidate {
            id,
            track_name: title.to_string(),
            artist_name: artist.to_string(),
            duration,
            instrumental: false,
            synced_lyrics: Some("[00:01.00] la".to_string()),
        }
    }

    fn track(title: &str, artist: &str, duration_ms: u32) -> Track {
        Track { title: title.to_string(), artist: artist.to_string(), album: String::new(), duration_ms }
    }

    #[test]
    fn score_accepts_exact() {
        let s = score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 266.0)).unwrap();
        assert!((s - 1.0).abs() < 1e-9, "{s}");
    }

    #[test]
    fn score_rejects_duration_off_by_more_than_2s() {
        assert!(score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 268.0)).is_some());
        assert!(score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 264.0)).is_some());
        assert!(score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 268.1)).is_none());
        assert!(score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 263.9)).is_none());
    }

    #[test]
    fn score_rejects_duration_that_is_not_a_number() {
        assert_eq!(score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", f64::NAN)), None);
    }

    #[test]
    fn score_drops_as_duration_drifts() {
        let near = score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 266.5)).unwrap();
        let far = score("Yellow", "Coldplay", 266_000, &cand(1, "Yellow", "Coldplay", 268.0)).unwrap();
        assert!((far - 0.9).abs() < 1e-9, "{far}");
        assert!(near > far);
    }

    #[test]
    fn score_rejects_different_title_same_artist() {
        assert_eq!(score("Yellow", "Coldplay", 266_000, &cand(1, "Clocks", "Coldplay", 266.0)), None);
    }

    #[test]
    fn score_rejects_same_title_different_artist() {
        let cover = cand(1, "Hallelujah", "Leonard Cohen", 413.0);
        assert_eq!(score("Hallelujah", "Jeff Buckley", 413_000, &cover), None);
    }

    #[test]
    fn score_rejects_candidate_without_synced_lyrics() {
        let mut c = cand(1, "Yellow", "Coldplay", 266.0);
        c.synced_lyrics = None;
        assert_eq!(score("Yellow", "Coldplay", 266_000, &c), None);
        c.synced_lyrics = Some("  \n".to_string());
        assert_eq!(score("Yellow", "Coldplay", 266_000, &c), None);
    }

    #[test]
    fn score_accepts_instrumental_without_lyrics() {
        let mut c = cand(1, "Yellow", "Coldplay", 266.0);
        c.synced_lyrics = None;
        c.instrumental = true;
        assert!(score("Yellow", "Coldplay", 266_000, &c).is_some());
    }

    #[test]
    fn score_accepts_artist_list_overlap() {
        let c = cand(1, "Kangen", "Dewa 19", 300.0);
        assert!(score("Kangen", "Dewa 19, Ari Lasso", 300_000, &c).is_some());
        let c = cand(1, "Kangen", "Ari Lasso & Dewa 19", 300.0);
        assert!(score("Kangen", "Dewa 19", 300_000, &c).is_some());
    }

    #[test]
    fn score_rejects_empty_key() {
        assert_eq!(score("!!!", "Coldplay", 266_000, &cand(1, "!!!", "Coldplay", 266.0)), None);
        assert_eq!(score("Yellow", "", 266_000, &cand(1, "Yellow", "", 266.0)), None);
        assert_eq!(score("Yellow", "", 266_000, &cand(1, "Yellow", "Coldplay", 266.0)), None);
        assert_eq!(score("Yellow", "?, !", 266_000, &cand(1, "Yellow", "?, !", 266.0)), None);
    }

    #[test]
    fn score_ignores_decorations_on_both_sides() {
        let c = cand(1, "Bohemian Rhapsody (Remastered 2011)", "Queen feat. Nobody", 354.0);
        let s = score("Bohemian Rhapsody - 2011 Remaster", "Queen", 354_000, &c).unwrap();
        assert!((s - 1.0).abs() < 1e-9, "{s}");
    }

    #[test]
    fn score_ignores_case_accents_and_punctuation() {
        let c = cand(1, "DÉJÀ VU", "beyonce", 240.0);
        assert!(score("Déjà Vu", "Beyoncé", 240_000, &c).is_some());
    }

    #[test]
    fn score_rejects_different_japanese_title() {
        let c = cand(1, "夜に駆ける", "YOASOBI", 261.0);
        assert!(score("夜に駆ける", "YOASOBI", 261_000, &c).is_some());
        assert_eq!(score("群青", "YOASOBI", 261_000, &c), None);
    }

    #[test]
    fn best_prefers_closer_duration() {
        let cs = [
            cand(1, "Yellow", "Coldplay", 267.5),
            cand(2, "Yellow", "Coldplay", 266.2),
            cand(3, "Yellow", "Coldplay", 264.5),
        ];
        assert_eq!(best(&track("Yellow", "Coldplay", 266_000), &cs).map(|c| c.id), Some(2));
    }

    #[test]
    fn best_none_when_all_rejected() {
        let cs = [
            cand(1, "Clocks", "Coldplay", 266.0),
            cand(2, "Yellow", "Coldplay", 300.0),
            cand(3, "Yellow", "Someone Else", 266.0),
        ];
        assert_eq!(best(&track("Yellow", "Coldplay", 266_000), &cs), None);
        assert_eq!(best(&track("Yellow", "Coldplay", 266_000), &[]), None);
    }

    #[test]
    fn best_keeps_the_first_of_equal_scores() {
        let cs = [cand(1, "Yellow", "Coldplay", 266.0), cand(2, "Yellow", "Coldplay", 266.0)];
        assert_eq!(best(&track("Yellow", "Coldplay", 266_000), &cs).map(|c| c.id), Some(1));
    }

    #[test]
    fn score_rejects_language_version_against_plain() {
        for (playing, theirs) in [
            ("Song (Japanese Version)", "Song"),
            ("Song", "Song (Japanese Version)"),
            ("Song (Japanese Version)", "Song (English Version)"),
            ("Song (Instrumental Version)", "Song"),
            ("Cupid - Twin Version", "Cupid"),
            ("Song -Japanese ver.-", "Song"),
        ] {
            let c = cand(1, theirs, "Artist", 200.0);
            assert_eq!(score(playing, "Artist", 200_000, &c), None, "{playing} / {theirs}");
        }
        let same_qualifier = cand(1, "Cupid (Twin Version)", "Artist", 200.0);
        assert!(score("Cupid - Twin Version", "Artist", 200_000, &same_qualifier).is_some());
    }

    #[test]
    fn score_rejects_numbered_sibling() {
        for (playing, theirs) in [
            ("Pigs on the Wing 1", "Pigs on the Wing 2"),
            ("Symphony No. 5", "Symphony No. 6"),
            ("Another Brick in the Wall, Pt. 1", "Another Brick in the Wall, Pt. 2"),
            ("Song Pt. 1", "Song Pt. 2"),
        ] {
            assert_eq!(score(playing, "Artist", 200_000, &cand(1, theirs, "Artist", 200.0)), None, "{playing}");
        }
    }

    #[test]
    fn score_rejects_short_near_miss() {
        assert_eq!(score("Stronger", "Artist", 200_000, &cand(1, "Stranger", "Artist", 200.0)), None);
    }

    #[test]
    fn score_rejects_cjk_one_char_difference() {
        let c = cand(1, "僕の知らない物語", "supercell", 340.0);
        assert_eq!(score("君の知らない物語", "supercell", 340_000, &c), None);
    }

    #[test]
    fn score_accepts_long_ascii_typo() {
        let c = cand(1, "Don't Stop Believing", "Journey", 250.0);
        let s = score("Don't Stop Believin'", "Journey", 250_000, &c).unwrap();
        assert!(s > 0.9 && s < 1.0, "{s}");
    }

    #[test]
    fn score_rejects_typo_with_three_edits() {
        // 0.90 similar, which TITLE_MIN alone would accept.
        let c = cand(1, "The Nights We Meet Again Todays", "Artist", 250.0);
        assert_eq!(score("The Night We Met Again Today", "Artist", 250_000, &c), None);
    }

    #[test]
    fn best_prefers_exact_title_over_closer_duration() {
        let cs = [cand(1, "Don't Stop Believing", "Journey", 250.0), cand(2, "Don't Stop Believin'", "Journey", 251.5)];
        assert_eq!(best(&track("Don't Stop Believin'", "Journey", 250_000), &cs).map(|c| c.id), Some(2));
    }

    #[test]
    fn best_prefers_same_raw_title() {
        let cs = [cand(1, "Yellow", "Coldplay", 266.0), cand(2, "Yellow (Live)", "Coldplay", 267.5)];
        assert_eq!(best(&track("Yellow - Live", "Coldplay", 266_000), &cs).map(|c| c.id), Some(2));
        assert_eq!(best(&track("Yellow", "Coldplay", 267_500), &cs).map(|c| c.id), Some(1));
    }

    #[test]
    fn best_prefers_synced_over_instrumental() {
        let mut flagged = cand(1, "Yellow", "Coldplay", 266.0);
        flagged.synced_lyrics = None;
        flagged.instrumental = true;
        let cs = [flagged, cand(2, "Yellow", "Coldplay", 267.0)];
        assert_eq!(best(&track("Yellow", "Coldplay", 266_000), &cs).map(|c| c.id), Some(2));
    }

    #[test]
    fn overlong_input_is_rejected() {
        let long = "a".repeat(301);
        let fits = &long[..300];
        assert!(score(fits, "Artist", 200_000, &cand(1, fits, "Artist", 200.0)).is_some());
        assert_eq!(score(&long, "Artist", 200_000, &cand(1, &long, "Artist", 200.0)), None);
        assert_eq!(score("Song", &long, 200_000, &cand(1, "Song", &long, 200.0)), None);
        assert_eq!(score(fits, "Artist", 200_000, &cand(1, &long, "Artist", 200.0)), None);
        assert_eq!(best(&track(&long, "Artist", 200_000), &[cand(1, &long, "Artist", 200.0)]), None);
    }

    #[test]
    fn best_matches_youtube_style_title() {
        let cs = [cand(1, "Everybody's Changing", "Keane", 237.0), cand(2, "Somewhere Only We Know", "Keane", 237.0)];
        let playing = track("Keane - Somewhere Only We Know (Official Music Video)", "KeaneVEVO", 237_000);
        assert_eq!(best(&playing, &cs).map(|c| c.id), Some(2));
    }

    #[test]
    fn candidate_reads_lrclib_json() {
        let json = r#"{"id":7,"name":"Yellow","trackName":"Yellow","artistName":"Coldplay","albumName":"Parachutes",
            "duration":266,"plainLyrics":"la","syncedLyrics":null}"#;
        let c: Candidate = serde_json::from_str(json).unwrap();
        assert_eq!(c, Candidate { synced_lyrics: None, ..cand(7, "Yellow", "Coldplay", 266.0) });
    }
}
