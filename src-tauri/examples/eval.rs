//! Looks up every song of a test set on the live LRCLIB and reports how many matches were right.
//! Usage: cargo run --release --example eval -- ../eval/songs.json [out.json]
//! Prints names, ids, durations and outcomes only, never lyrics.

use std::process::ExitCode;
use std::time::Duration;

use larik_lib::lrclib::LrcLib;
use larik_lib::lyrics::resolve;
use larik_lib::matcher::{Candidate, Track};
use larik_lib::normalize::{clean_artist, clean_title, key};

const PAUSE_BETWEEN_SONGS: Duration = Duration::from_millis(300);
const OUTCOMES: [&str; 4] = ["correct", "missed", "wrong", "error"];

#[derive(serde::Deserialize)]
struct Song {
    id: String,
    smtc_title: String,
    smtc_artist: String,
    smtc_album: Option<String>,
    duration_ms: u32,
    expect: Expect,
    expect_title: Option<String>,
    expect_artist: Option<String>,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Expect {
    Match,
    None,
}

/// "correct", "missed" or "wrong" for a lookup that ended without an error.
fn judge(song: &Song, accepted: Option<&Candidate>) -> &'static str {
    match (song.expect, accepted) {
        (Expect::None, None) => "correct",
        (Expect::Match, None) => "missed",
        (Expect::Match, Some(c)) if is_expected(song, c) => "correct",
        (_, Some(_)) => "wrong",
    }
}

/// The same title, as written or without decorations, and at least one artist in common.
fn is_expected(song: &Song, c: &Candidate) -> bool {
    let want = song.expect_title.as_deref().unwrap_or_default();
    let same_title =
        key(&c.track_name) == key(want) || key(&clean_title(&c.track_name)) == key(&clean_title(want));
    let theirs = artist_names(&c.artist_name);
    same_title && artist_names(song.expect_artist.as_deref().unwrap_or_default()).iter().any(|n| theirs.contains(n))
}

/// The keyed names of an artist list such as "Dewa 19, Ari Lasso".
fn artist_names(list: &str) -> Vec<String> {
    let commas = clean_artist(list).replace(" & ", ",").replace(" x ", ",");
    commas.split([',', ';', '、']).map(key).filter(|k| !k.is_empty()).collect()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("usage: eval <songs.json> [out.json]");
        return ExitCode::from(2);
    };
    let read = std::fs::read_to_string(input).map_err(|e| e.to_string());
    let songs: Vec<Song> = match read.and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string())) {
        Ok(songs) => songs,
        Err(e) => {
            eprintln!("{input}: {e}");
            return ExitCode::from(2);
        }
    };

    let lrclib = LrcLib::new();
    let mut results = Vec::new();
    let mut totals = [0usize; OUTCOMES.len()];
    println!("{:<10} {:<8} accepted", "id", "outcome");
    for (i, song) in songs.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(PAUSE_BETWEEN_SONGS);
        }
        let track = Track {
            title: song.smtc_title.clone(),
            artist: song.smtc_artist.clone(),
            album: song.smtc_album.clone().unwrap_or_default(),
            duration_ms: song.duration_ms,
        };
        let (outcome, accepted, error) = match resolve(&track, &lrclib) {
            Ok((_, accepted)) => (judge(song, accepted.as_ref()), accepted, None),
            Err(e) => ("error", None, Some(format!("{e:?}"))),
        };
        totals[OUTCOMES.iter().position(|o| *o == outcome).expect("a known outcome")] += 1;
        let shown = match (&accepted, &error) {
            (Some(c), _) => format!("{} / {} (id {}, {} s)", c.track_name, c.artist_name, c.id, c.duration),
            (None, Some(e)) => e.clone(),
            (None, None) => "-".to_string(),
        };
        println!("{:<10} {:<8} {shown}", song.id, outcome);
        results.push(serde_json::json!({
            "id": song.id,
            "expect": match song.expect { Expect::Match => "match", Expect::None => "none" },
            "outcome": outcome,
            "accepted": accepted.map(|c| serde_json::json!({
                "id": c.id, "track_name": c.track_name, "artist_name": c.artist_name, "duration": c.duration,
            })),
            "error": error,
        }));
    }

    let counts: Vec<String> = OUTCOMES.iter().zip(totals).map(|(outcome, n)| format!("{outcome} {n}")).collect();
    println!("\n{} songs: {}", songs.len(), counts.join(", "));

    if let Some(out) = args.get(1) {
        let totals: serde_json::Map<String, serde_json::Value> =
            OUTCOMES.iter().zip(totals).map(|(outcome, n)| (outcome.to_string(), n.into())).collect();
        let report = serde_json::json!({ "songs": songs.len(), "totals": totals, "results": results });
        let text = serde_json::to_string_pretty(&report).expect("plain JSON values");
        if let Err(e) = std::fs::write(out, text) {
            eprintln!("{out}: {e}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(expect: Expect, title: &str, artist: &str) -> Song {
        Song {
            id: "t-01".to_string(),
            smtc_title: String::new(),
            smtc_artist: String::new(),
            smtc_album: None,
            duration_ms: 200_000,
            expect,
            expect_title: Some(title.to_string()),
            expect_artist: Some(artist.to_string()),
        }
    }

    fn cand(title: &str, artist: &str) -> Candidate {
        Candidate {
            id: 1,
            track_name: title.to_string(),
            artist_name: artist.to_string(),
            duration: 200.0,
            instrumental: false,
            synced_lyrics: None,
        }
    }

    #[test]
    fn expected_song_is_correct() {
        let want = song(Expect::Match, "Kangen", "Dewa 19");
        for (title, artist) in [
            ("Kangen", "Dewa 19"),
            ("KANGEN", "dewa 19"),
            ("Kangen - Remastered 2011", "Dewa 19"),
            ("Kangen", "Ari Lasso, Dewa 19"),
            ("Kangen", "Dewa 19 & Ari Lasso"),
            ("Kangen", "Dewa 19 feat. Someone"),
        ] {
            assert_eq!(judge(&want, Some(&cand(title, artist))), "correct", "{title} / {artist}");
        }
        let decorated = song(Expect::Match, "Kangen (Live)", "Dewa 19");
        assert_eq!(judge(&decorated, Some(&cand("Kangen", "Dewa 19"))), "correct");
    }

    #[test]
    fn another_song_is_wrong() {
        let want = song(Expect::Match, "Kangen", "Dewa 19");
        let others = [("Kangen", "Other Band"), ("Pupus", "Dewa 19"), ("Kangen (Japanese Version)", "Dewa 19")];
        for (title, artist) in others {
            assert_eq!(judge(&want, Some(&cand(title, artist))), "wrong", "{title} / {artist}");
        }
    }

    #[test]
    fn nothing_accepted_is_missed_or_correct() {
        assert_eq!(judge(&song(Expect::Match, "Kangen", "Dewa 19"), None), "missed");
        assert_eq!(judge(&song(Expect::None, "", ""), None), "correct");
    }

    #[test]
    fn any_candidate_is_wrong_when_none_is_expected() {
        assert_eq!(judge(&song(Expect::None, "Kangen", "Dewa 19"), Some(&cand("Kangen", "Dewa 19"))), "wrong");
    }

    #[test]
    fn song_file_format_is_read() {
        let json = r#"[
            { "id": "en-01", "smtc_title": "Yellow", "smtc_artist": "Coldplay", "smtc_album": "Parachutes",
              "duration_ms": 266773, "expect": "match", "expect_title": "Yellow", "expect_artist": "Coldplay",
              "note": "x" },
            { "id": "xx-01", "smtc_title": "Nothing", "smtc_artist": "Nobody", "smtc_album": null,
              "duration_ms": 1000, "expect": "none" }
        ]"#;
        let songs: Vec<Song> = serde_json::from_str(json).unwrap();
        assert_eq!(songs.len(), 2);
        assert_eq!(judge(&songs[0], Some(&cand("Yellow", "Coldplay"))), "correct");
        assert_eq!(judge(&songs[1], None), "correct");
    }
}
