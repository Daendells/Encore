//! User settings and per-song lyric offsets, two small JSON files in one folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const FONT_PX_RANGE: std::ops::RangeInclusive<u32> = 12..=48;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font_px: u32,
    pub visible: bool,
    pub hide_from_capture: bool,
    pub position: Option<(i32, i32)>,
    /// Fixed lyric delay per player, keyed by the SMTC app id.
    pub app_offsets_ms: BTreeMap<String, i32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { font_px: 22, visible: true, hide_from_capture: true, position: None, app_offsets_ms: BTreeMap::new() }
    }
}

/// Best effort like the lyric cache: a file that cannot be read or written is the same as no file.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Defaults when the file is missing or corrupt. `font_px` is clamped to 12..=48.
    pub fn load(&self) -> Settings {
        let mut s: Settings = self.read("settings.json").unwrap_or_default();
        s.font_px = s.font_px.clamp(*FONT_PX_RANGE.start(), *FONT_PX_RANGE.end());
        s
    }

    pub fn save(&self, s: &Settings) {
        self.write("settings.json", s);
    }

    pub fn song_offset_ms(&self, cache_key: &str) -> i32 {
        self.offsets().get(cache_key).copied().unwrap_or(0)
    }

    /// 0 removes the entry.
    pub fn set_song_offset_ms(&self, cache_key: &str, ms: i32) {
        let mut offsets = self.offsets();
        if ms == 0 {
            offsets.remove(cache_key);
        } else {
            offsets.insert(cache_key.to_string(), ms);
        }
        self.write("offsets.json", &offsets);
    }

    fn offsets(&self) -> BTreeMap<String, i32> {
        self.read("offsets.json").unwrap_or_default()
    }

    fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> Option<T> {
        serde_json::from_slice(&fs::read(self.dir.join(name)).ok()?).ok()
    }

    fn write<T: serde::Serialize>(&self, name: &str, value: &T) {
        let path = self.dir.join(name);
        let temp = path.with_extension("tmp");
        let _ = fs::create_dir_all(&self.dir);
        // Written beside the file and renamed over it, so a crash never leaves half a file.
        if serde_json::to_vec_pretty(value).ok().is_some_and(|json| fs::write(&temp, json).is_ok()) {
            let _ = fs::rename(&temp, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in its own folder under the system temp folder, removed when the test ends.
    struct Temp(Store);

    impl Temp {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("larik-settings-test-{}-{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Temp(Store::new(dir))
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.dir);
        }
    }

    #[test]
    fn load_missing_is_default() {
        let s = Temp::new("missing").0.load();
        assert_eq!(s, Settings::default());
        assert_eq!((s.font_px, s.visible, s.hide_from_capture, s.position), (22, true, true, None));
    }

    #[test]
    fn load_corrupt_is_default() {
        let t = Temp::new("corrupt");
        fs::create_dir_all(&t.0.dir).unwrap();
        fs::write(t.0.dir.join("settings.json"), "{ not json").unwrap();
        assert_eq!(t.0.load(), Settings::default());
        fs::write(t.0.dir.join("settings.json"), r#"{"font_px":"big"}"#).unwrap();
        assert_eq!(t.0.load(), Settings::default());
    }

    #[test]
    fn missing_fields_take_defaults_and_save_round_trips() {
        let t = Temp::new("round-trip");
        fs::create_dir_all(&t.0.dir).unwrap();
        fs::write(t.0.dir.join("settings.json"), r#"{"visible":false}"#).unwrap();
        let loaded = t.0.load();
        assert_eq!(loaded, Settings { visible: false, ..Settings::default() });
        let mut s = Settings { font_px: 30, position: Some((-5, 700)), ..loaded };
        s.app_offsets_ms.insert("Spotify.exe".to_string(), 200);
        t.0.save(&s);
        assert_eq!(t.0.load(), s);
    }

    #[test]
    fn font_px_is_clamped() {
        let t = Temp::new("clamp");
        for (stored, loaded) in [(2, 12), (12, 12), (48, 48), (500, 48)] {
            t.0.save(&Settings { font_px: stored, ..Settings::default() });
            assert_eq!(t.0.load().font_px, loaded);
        }
    }

    #[test]
    fn song_offset_round_trip_and_zero_removes() {
        let t = Temp::new("offsets");
        assert_eq!(t.0.song_offset_ms("a"), 0);
        t.0.set_song_offset_ms("a", 250);
        t.0.set_song_offset_ms("b", -500);
        assert_eq!((t.0.song_offset_ms("a"), t.0.song_offset_ms("b")), (250, -500));
        t.0.set_song_offset_ms("a", 0);
        assert_eq!((t.0.song_offset_ms("a"), t.0.song_offset_ms("b")), (0, -500));
        let json = fs::read_to_string(t.0.dir.join("offsets.json")).unwrap();
        assert!(!json.contains("\"a\""), "{json}");
    }
}
