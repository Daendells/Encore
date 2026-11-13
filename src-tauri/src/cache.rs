use std::fs;
use std::path::PathBuf;

use crate::lyrics::Lyrics;
use crate::matcher::Track;
use crate::normalize::{clean_artist, clean_title, key};

pub const NOT_FOUND_TTL_S: u64 = 7 * 24 * 3600;

/// One file per song. Best effort: a file that cannot be read or written is the same as no entry.
pub struct Cache {
    dir: PathBuf,
}

#[derive(serde::Deserialize)]
struct Entry {
    saved_unix_s: u64,
    lyrics: Lyrics,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// 16 hex chars, stable across runs.
    pub fn key(track: &Track) -> String {
        let seconds = track.duration_ms / 1000 + u32::from(track.duration_ms % 1000 >= 500);
        let name = format!("{}|{}|{seconds}", key(&clean_artist(&track.artist)), key(&clean_title(&track.title)));
        // FNV-1a, 64 bit. The standard library's hasher may change between Rust versions.
        let hash =
            name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        format!("{hash:016x}")
    }

    pub fn get(&self, track: &Track, now_unix_s: u64) -> Option<Lyrics> {
        let entry: Entry = serde_json::from_slice(&fs::read(self.path(track)).ok()?).ok()?;
        let expired =
            entry.lyrics == Lyrics::NotFound && now_unix_s.saturating_sub(entry.saved_unix_s) > NOT_FOUND_TTL_S;
        (!expired).then_some(entry.lyrics)
    }

    pub fn put(&self, track: &Track, lyrics: &Lyrics, now_unix_s: u64) {
        let path = self.path(track);
        let temp = path.with_extension("tmp");
        let entry = serde_json::json!({ "saved_unix_s": now_unix_s, "lyrics": lyrics });
        let _ = fs::create_dir_all(&self.dir);
        // Written beside the entry and renamed over it, so a crash never leaves half an entry.
        if fs::write(&temp, entry.to_string()).is_ok() {
            let _ = fs::rename(&temp, &path);
        }
    }

    fn path(&self, track: &Track) -> PathBuf {
        self.dir.join(format!("{}.json", Self::key(track)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lrc::parse;

    /// A cache in its own folder under the system temp folder, removed when the test ends.
    struct Temp(Cache);

    impl Temp {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("encore-cache-test-{}-{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Temp(Cache::new(dir))
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.dir);
        }
    }

    fn track(title: &str, artist: &str, duration_ms: u32) -> Track {
        Track { title: title.to_string(), artist: artist.to_string(), album: String::new(), duration_ms }
    }

    fn synced(lrc: &str) -> Lyrics {
        Lyrics::Synced(parse(lrc))
    }

    #[test]
    fn round_trip_synced() {
        let temp = Temp::new("round-trip");
        let song = track("Yellow", "Coldplay", 266_773);
        let lyrics = synced("[00:01.00] satu\n[00:03.00] <00:03.00> dua <00:03.50> tiga\n[00:09.00]");
        assert_eq!(temp.0.get(&song, 1_000), None);
        temp.0.put(&song, &lyrics, 1_000);
        assert_eq!(temp.0.get(&song, 1_000), Some(lyrics.clone()));
        // Found lyrics do not expire.
        assert_eq!(temp.0.get(&song, 1_000 + 10 * NOT_FOUND_TTL_S), Some(lyrics));
        assert_eq!(temp.0.get(&track("Clocks", "Coldplay", 266_773), 1_000), None);
        temp.0.put(&song, &Lyrics::Instrumental, 1_000);
        assert_eq!(temp.0.get(&song, 1_000 + 10 * NOT_FOUND_TTL_S), Some(Lyrics::Instrumental));
    }

    #[test]
    fn entry_is_one_json_file_named_by_the_key() {
        let temp = Temp::new("file");
        let song = track("Yellow", "Coldplay", 266_773);
        temp.0.put(&song, &Lyrics::NotFound, 1_234);
        let names: Vec<_> = fs::read_dir(&temp.0.dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, [format!("{}.json", Cache::key(&song)).as_str()]);
        let text = fs::read_to_string(temp.0.dir.join(&names[0])).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json, serde_json::json!({ "saved_unix_s": 1_234, "lyrics": "NotFound" }));
    }

    #[test]
    fn key_ignores_decorations_and_case() {
        let key = Cache::key(&track("Song - Remastered 2011", "Artist", 200_000));
        assert_eq!(key, Cache::key(&track("SONG", "artist feat. Someone", 200_400)));
        assert_ne!(key, Cache::key(&track("Song", "Artist", 201_000)));
        assert_ne!(key, Cache::key(&track("Song", "Other", 200_000)));
        assert_ne!(key, Cache::key(&track("Song (Japanese Version)", "Artist", 200_000)));
        assert_ne!(Cache::key(&track("a", "b c", 1_000)), Cache::key(&track("c a", "b", 1_000)));
    }

    #[test]
    fn key_is_the_fnv1a_hash_of_the_cleaned_names_and_seconds() {
        // FNV-1a 64-bit of "coldplay|yellow|267", computed outside this program.
        assert_eq!(Cache::key(&track("Yellow", "Coldplay", 266_773)), "4fd22e87b2e3962e");
        assert_eq!(Cache::key(&track("Yellow", "Coldplay", u32::MAX)).len(), 16);
    }

    #[test]
    fn not_found_expires_after_ttl() {
        let temp = Temp::new("ttl");
        let song = track("Yellow", "Coldplay", 266_773);
        temp.0.put(&song, &Lyrics::NotFound, 1_000);
        assert_eq!(temp.0.get(&song, 1_000 + NOT_FOUND_TTL_S), Some(Lyrics::NotFound));
        assert_eq!(temp.0.get(&song, 1_000 + NOT_FOUND_TTL_S + 1), None);
        // A clock that was set back does not expire the entry.
        assert_eq!(temp.0.get(&song, 0), Some(Lyrics::NotFound));
    }

    #[test]
    fn corrupt_file_reads_as_none() {
        let temp = Temp::new("corrupt");
        let song = track("Yellow", "Coldplay", 266_773);
        temp.0.put(&song, &Lyrics::Instrumental, 1_000);
        let path = temp.0.dir.join(format!("{}.json", Cache::key(&song)));
        for junk in ["", "{\"saved_unix_s\": 1", "{\"saved_unix_s\":1,\"lyrics\":\"Other\"}", "\u{0}\u{1}"] {
            fs::write(&path, junk).unwrap();
            assert_eq!(temp.0.get(&song, 1_000), None, "{junk}");
        }
        temp.0.put(&song, &Lyrics::Instrumental, 2_000);
        assert_eq!(temp.0.get(&song, 2_000), Some(Lyrics::Instrumental));
    }

    #[test]
    fn put_then_get_overwrites() {
        let temp = Temp::new("overwrite");
        let song = track("Yellow", "Coldplay", 266_773);
        temp.0.put(&song, &Lyrics::NotFound, 1_000);
        temp.0.put(&song, &synced("[00:01.00] baru"), 2_000);
        assert_eq!(temp.0.get(&song, 2_000 + 2 * NOT_FOUND_TTL_S), Some(synced("[00:01.00] baru")));
    }

    #[test]
    fn folder_that_cannot_be_made_is_ignored() {
        let temp = Temp::new("blocked");
        fs::write(&temp.0.dir, "a file where the folder should be").unwrap();
        let song = track("Yellow", "Coldplay", 266_773);
        temp.0.put(&song, &Lyrics::Instrumental, 1_000);
        assert_eq!(temp.0.get(&song, 1_000), None);
        fs::remove_file(&temp.0.dir).unwrap();
    }
}
