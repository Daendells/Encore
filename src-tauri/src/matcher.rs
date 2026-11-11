use crate::normalize::{candidates, clean_artist, clean_title, key};

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

/// None = rejected. Some(score) in 0..=1, higher is better.
pub fn score(title: &str, artist: &str, duration_ms: u32, c: &Candidate) -> Option<f64> {
    let diff = (c.duration - f64::from(duration_ms) / 1000.0).abs();
    let has_lyrics = c.synced_lyrics.as_deref().is_some_and(|l| !l.trim().is_empty());
    // Written as "within" so that a NaN duration is rejected too.
    let usable = diff <= MAX_DURATION_DIFF_S && (has_lyrics || c.instrumental);
    if !usable {
        return None;
    }
    let title_sim = similarity(&key(&clean_title(title)), &key(&clean_title(&c.track_name)))?;
    let (artist, theirs) = (clean_artist(artist), clean_artist(&c.artist_name));
    let mut artist_sim = similarity(&key(&artist), &key(&theirs))?;
    let names = artist_names(&artist);
    if artist_names(&theirs).iter().any(|n| names.contains(n)) {
        artist_sim = 1.0;
    }
    (title_sim >= TITLE_MIN && artist_sim >= ARTIST_MIN)
        .then(|| 0.6 * title_sim + 0.3 * artist_sim + 0.1 * (1.0 - diff / MAX_DURATION_DIFF_S))
}

pub fn best<'a>(track: &Track, cs: &'a [Candidate]) -> Option<&'a Candidate> {
    let pairs = candidates(&track.title, &track.artist);
    let mut best: Option<(f64, &Candidate)> = None;
    for c in cs {
        for (title, artist) in &pairs {
            if let Some(s) = score(title, artist, track.duration_ms, c) {
                // Strictly greater: of equal scores the first candidate wins.
                if best.is_none_or(|(top, _)| s > top) {
                    best = Some((s, c));
                }
            }
        }
    }
    best.map(|(_, c)| c)
}

/// None when either key is empty.
fn similarity(a: &str, b: &str) -> Option<f64> {
    (!a.is_empty() && !b.is_empty()).then(|| strsim::normalized_levenshtein(a, b))
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
