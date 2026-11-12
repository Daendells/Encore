use std::io::Read;
use std::time::Duration;

use crate::lyrics::{Fetch, FetchError};
use crate::matcher::Candidate;

const BASE: &str = "https://lrclib.net";
const USER_AGENT: &str = concat!("Larik/", env!("CARGO_PKG_VERSION"), " (https://github.com/Daendells/larik)");
const TIMEOUT: Duration = Duration::from_secs(10);
/// A longer response is refused. A search for a song with long lyrics is about 1.3 MB: each of its 20 results
/// repeats the lyrics three times.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Longer synced lyrics are dropped: each timestamp makes the parser copy its line.
const MAX_SYNCED_BYTES: usize = 200_000;
const MAX_SEARCH_RESULTS: usize = 20;

pub struct LrcLib {
    agent: ureq::Agent,
}

impl LrcLib {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            // A status is judged here, and a redirect is not followed: the app talks to lrclib.net only.
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .https_only(true)
            .build();
        Self { agent: config.into() }
    }

    /// The status and, for 200 only, the body.
    fn call(&self, url: &str) -> Result<(u16, String), FetchError> {
        let mut response = self.agent.get(url).call().map_err(|e| FetchError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let body = if status == 200 { read_body(response.body_mut().as_reader())? } else { String::new() };
        Ok((status, body))
    }
}

impl Default for LrcLib {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for LrcLib {
    fn get(&self, title: &str, artist: &str, album: &str, duration_s: u32) -> Result<Option<Candidate>, FetchError> {
        match self.call(&get_url(title, artist, album, duration_s))? {
            (200, body) => parse_get(&body),
            (404, _) => Ok(None),
            (status, _) => Err(FetchError::BadResponse(format!("HTTP {status}"))),
        }
    }

    fn search(&self, title: &str, artist: &str) -> Result<Vec<Candidate>, FetchError> {
        match self.call(&search_url(title, artist))? {
            (200, body) => parse_search(&body),
            (status, _) => Err(FetchError::BadResponse(format!("HTTP {status}"))),
        }
    }
}

/// What is read of an LRCLIB record. Fields that LRCLIB sends as null are optional here. The other fields,
/// such as the large `plainLyrics` and `lyricsfile`, are skipped while parsing and never stored.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: u64,
    track_name: String,
    artist_name: String,
    duration: Option<f64>,
    instrumental: Option<bool>,
    synced_lyrics: Option<String>,
}

impl Record {
    /// None without a duration: such a record cannot be checked against the playing song.
    fn candidate(self) -> Option<Candidate> {
        Some(Candidate {
            id: self.id,
            track_name: self.track_name,
            artist_name: self.artist_name,
            duration: self.duration?,
            instrumental: self.instrumental.unwrap_or(false),
            synced_lyrics: self.synced_lyrics.filter(|lyrics| lyrics.len() <= MAX_SYNCED_BYTES),
        })
    }
}

fn get_url(title: &str, artist: &str, album: &str, duration_s: u32) -> String {
    let (title, artist, album) = (encode(title), encode(artist), encode(album));
    format!("{BASE}/api/get?track_name={title}&artist_name={artist}&album_name={album}&duration={duration_s}")
}

fn search_url(title: &str, artist: &str) -> String {
    format!("{BASE}/api/search?track_name={}&artist_name={}", encode(title), encode(artist))
}

/// Percent-encodes every byte that is not an unreserved URL character.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn read_body(reader: impl Read) -> Result<String, FetchError> {
    let mut bytes = Vec::new();
    // One byte past the limit shows that there was more.
    let read = reader.take(MAX_BODY_BYTES as u64 + 1).read_to_end(&mut bytes);
    read.map_err(|e| FetchError::Network(e.to_string()))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(FetchError::BadResponse(format!("body is over {MAX_BODY_BYTES} bytes")));
    }
    String::from_utf8(bytes).map_err(|_| FetchError::BadResponse("body is not UTF-8".to_string()))
}

fn parse_get(body: &str) -> Result<Option<Candidate>, FetchError> {
    let record: Record = serde_json::from_str(body).map_err(bad_json)?;
    Ok(record.candidate())
}

/// A result that is not a usable record is left out; the others are kept.
fn parse_search(body: &str) -> Result<Vec<Candidate>, FetchError> {
    // Each result stays a slice of the body until it is read as a record, so a bad one fails alone.
    let results: Vec<&serde_json::value::RawValue> = serde_json::from_str(body).map_err(bad_json)?;
    let records = results.iter().take(MAX_SEARCH_RESULTS).filter_map(|r| serde_json::from_str::<Record>(r.get()).ok());
    Ok(records.filter_map(Record::candidate).collect())
}

/// Says only where: serde's own message can quote the body, and the body holds lyrics.
fn bad_json(e: serde_json::Error) -> FetchError {
    FetchError::BadResponse(format!("unexpected JSON at line {} column {}", e.line(), e.column()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GET /api/get?track_name=Yellow&artist_name=Coldplay&album_name=Parachutes&duration=267 on 2026-11-12,
    /// with the three lyric fields cut short.
    const REAL_GET: &str = r#"{"id":16233,"name":"Yellow","trackName":"Yellow","artistName":"Coldplay","albumName":"Parachutes","duration":267.0,"instrumental":false,"hasWordSync":true,"plainLyrics":"Look at the stars\nLook how they shine for you\n","syncedLyrics":"[00:33.80] Look at the stars\n[00:36.23] Look how they shine for you\n","lyricsfile":"version: '1.0'\nmetadata:\n  title: Yellow\n  artist: Coldplay\n  album: Parachutes\n  duration_ms: 267000\n  instrumental: false\n  language: en\nlines: []\n"}"#;

    /// GET /api/search?track_name=Yellow&artist_name=Coldplay on 2026-11-12: results 1, 2 and 12 of 20,
    /// with the three lyric fields cut short.
    const REAL_SEARCH: &str = r#"[{"id":7242265,"name":"Yellow","trackName":"Yellow","artistName":"Coldplay","albumName":"Q Awards: The Album","duration":271.0,"instrumental":false,"hasWordSync":false,"plainLyrics":"Look at the stars\nLook how they shine for you\n","syncedLyrics":"[00:33.44] Look at the stars\n[00:36.18] Look how they shine for you\n","lyricsfile":"version: '1.0'\nmetadata:\n  title: Yellow\n  artist: Coldplay\n  album: 'Q Awards: The Album'\n  duration_ms: 271000\n  instrumental: false\nlines: []\n"},
{"id":7831633,"name":"Yellow","trackName":"Yellow","artistName":"Coldplay","albumName":"2000's (Hi-Res Masters)","duration":267.0,"instrumental":false,"hasWordSync":true,"plainLyrics":"Look at the stars\nLook how they shine for you\n","syncedLyrics":"[00:33.44] Look at the stars\n[00:36.19] Look how they shine for you\n","lyricsfile":"version: '1.0'\nmetadata:\n  title: Yellow\n  artist: Coldplay\n  album: 2000's (Hi-Res Masters)\n  duration_ms: 267000\n  instrumental: false\n  language: en\nlines: []\n"},
{"id":8238421,"name":"Yellow","trackName":"Yellow","artistName":"Coldplay","albumName":"null","duration":267.0,"instrumental":false,"hasWordSync":true,"plainLyrics":"Look at the stars\nLook how they shine for you\n","syncedLyrics":"[00:33.42] Look at the stars\n[00:36.27] Look how they shine for you\n","lyricsfile":"version: '1.0'\nmetadata:\n  title: Yellow\n  artist: Coldplay\n  album: 'null'\n  duration_ms: 267000\n  instrumental: false\n  language: en\nlines: []\n"}]"#;

    fn record(id: u64, rest: &str) -> String {
        format!(r#"{{"id":{id},"trackName":"Song","artistName":"Artist"{rest}}}"#)
    }

    #[test]
    fn builds_get_url_with_encoding() {
        assert_eq!(
            get_url("Don't Stop & Go?", "AC/DC", "夜に駆ける", 215),
            "https://lrclib.net/api/get?track_name=Don%27t%20Stop%20%26%20Go%3F&artist_name=AC%2FDC\
             &album_name=%E5%A4%9C%E3%81%AB%E9%A7%86%E3%81%91%E3%82%8B&duration=215"
        );
        assert_eq!(
            get_url("a-b_c.d~e", "A", "", 1),
            "https://lrclib.net/api/get?track_name=a-b_c.d~e&artist_name=A&album_name=&duration=1"
        );
    }

    #[test]
    fn builds_search_url_with_encoding() {
        assert_eq!(
            search_url("100% #1 + more", "Beyoncé"),
            "https://lrclib.net/api/search?track_name=100%25%20%231%20%2B%20more&artist_name=Beyonc%C3%A9"
        );
    }

    #[test]
    fn user_agent_names_the_app_and_its_version() {
        assert_eq!(USER_AGENT, "Larik/0.1.0 (https://github.com/Daendells/larik)");
    }

    #[test]
    fn malformed_json_is_an_error() {
        for body in ["", "<html>502 Bad Gateway</html>", "[]", "null", r#"{"id":1}"#, r#"{"id":"x","trackName":"a","artistName":"b"}"#] {
            assert!(matches!(parse_get(body), Err(FetchError::BadResponse(_))), "get {body}");
        }
        for body in ["", "<html>502 Bad Gateway</html>", "{}", "null", r#"[{"id":1,"#] {
            assert!(matches!(parse_search(body), Err(FetchError::BadResponse(_))), "search {body}");
        }
    }

    #[test]
    fn json_error_does_not_quote_the_body() {
        let body = r#"{"id":"secret lyric line","trackName":"a","artistName":"b"}"#;
        let Err(FetchError::BadResponse(message)) = parse_get(body) else { panic!("accepted") };
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn parses_real_get_sample() {
        let c = parse_get(REAL_GET).unwrap().unwrap();
        let lyrics = "[00:33.80] Look at the stars\n[00:36.23] Look how they shine for you\n";
        assert_eq!(
            c,
            Candidate {
                id: 16233,
                track_name: "Yellow".to_string(),
                artist_name: "Coldplay".to_string(),
                duration: 267.0,
                instrumental: false,
                synced_lyrics: Some(lyrics.to_string()),
            }
        );
    }

    #[test]
    fn parses_real_search_sample() {
        let found = parse_search(REAL_SEARCH).unwrap();
        assert_eq!(found.iter().map(|c| c.id).collect::<Vec<_>>(), [7242265, 7831633, 8238421]);
        assert_eq!(found.iter().map(|c| c.duration).collect::<Vec<_>>(), [271.0, 267.0, 267.0]);
        assert!(found.iter().all(|c| c.track_name == "Yellow" && c.artist_name == "Coldplay" && !c.instrumental));
        assert!(found[2].synced_lyrics.as_deref().unwrap().starts_with("[00:33.42] Look"));
    }

    #[test]
    fn null_and_missing_fields_are_tolerated() {
        let nulls = record(1, r#","albumName":null,"duration":200,"instrumental":null,"syncedLyrics":null,"new":[1]"#);
        let c = parse_get(&nulls).unwrap().unwrap();
        assert_eq!((c.duration, c.instrumental, c.synced_lyrics), (200.0, false, None));
        let bare = parse_get(&record(2, r#","duration":200.5"#)).unwrap().unwrap();
        assert_eq!((bare.duration, bare.instrumental, bare.synced_lyrics), (200.5, false, None));
        let flagged = parse_get(&record(3, r#","duration":200,"instrumental":true"#)).unwrap().unwrap();
        assert!(flagged.instrumental);
    }

    #[test]
    fn lyric_fields_that_are_not_shown_are_not_read() {
        // Of any type and any size: they are skipped, never stored.
        let rest = r#","duration":200,"plainLyrics":7,"lyricsfile":{"lines":[1,2]},"syncedLyrics":"[00:01.00] la""#;
        let c = parse_get(&record(1, rest)).unwrap().unwrap();
        assert_eq!(c.synced_lyrics.as_deref(), Some("[00:01.00] la"));
        assert_eq!(parse_search(&format!("[{}]", record(1, rest))).unwrap(), [c]);
    }

    #[test]
    fn record_without_a_duration_is_no_candidate() {
        assert_eq!(parse_get(&record(1, r#","duration":null,"syncedLyrics":"[00:01.00] la""#)).unwrap(), None);
        assert_eq!(parse_get(&record(1, "")).unwrap(), None);
    }

    #[test]
    fn malformed_search_element_is_dropped_alone() {
        let body = format!(
            r#"[{},{{"id":"x"}},7,null,{},{}]"#,
            record(1, r#","duration":200"#),
            record(2, r#","duration":null"#),
            record(3, r#","duration":201,"instrumental":null"#)
        );
        let found = parse_search(&body).unwrap();
        assert_eq!(found.iter().map(|c| c.id).collect::<Vec<_>>(), [1, 3]);
    }

    #[test]
    fn search_uses_the_first_20_results() {
        let items: Vec<String> = (1..=25).map(|id| record(id, r#","duration":200"#)).collect();
        let found = parse_search(&format!("[{}]", items.join(","))).unwrap();
        assert_eq!(found.iter().map(|c| c.id).collect::<Vec<_>>(), (1..=20).collect::<Vec<u64>>());
    }

    #[test]
    fn oversized_synced_lyrics_are_dropped() {
        let line = "[00:01.00] la\n";
        let fits = line.repeat(MAX_SYNCED_BYTES / line.len());
        let over = format!("{fits}{line}");
        assert!(fits.len() <= MAX_SYNCED_BYTES && over.len() > MAX_SYNCED_BYTES);
        let json =
            |lyrics: &str| record(1, &format!(r#","duration":200,"syncedLyrics":{}"#, serde_json::json!(lyrics)));
        assert_eq!(parse_get(&json(&fits)).unwrap().unwrap().synced_lyrics, Some(fits.clone()));
        assert_eq!(parse_get(&json(&over)).unwrap().unwrap().synced_lyrics, None);
        let found = parse_search(&format!("[{}]", json(&over))).unwrap();
        assert_eq!(found[0].synced_lyrics, None);
    }

    #[test]
    fn body_over_the_limit_is_refused() {
        let body = |len: usize| read_body(std::io::repeat(b'a').take(len as u64));
        assert_eq!(body(MAX_BODY_BYTES).unwrap().len(), MAX_BODY_BYTES);
        assert!(matches!(body(MAX_BODY_BYTES + 1), Err(FetchError::BadResponse(_))));
        // The size of a real search for a song with long lyrics (Rap God, 2026-11-12).
        assert!(body(1_276_485).is_ok());
        // Reading stops at the limit however much is sent.
        assert!(matches!(read_body(std::io::repeat(b'a')), Err(FetchError::BadResponse(_))));
    }

    #[test]
    fn body_that_is_not_utf8_is_refused() {
        assert!(matches!(read_body(&[0xff, 0xfe][..]), Err(FetchError::BadResponse(_))));
        assert_eq!(read_body("夜".as_bytes()).unwrap(), "夜");
    }
}
