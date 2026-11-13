//! User settings and per-song lyric offsets, two small JSON files in one folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

pub const FONT_PX_RANGE: std::ops::RangeInclusive<u32> = 12..=48;
pub const PLATE_OPACITY_RANGE: std::ops::RangeInclusive<u32> = 0..=70;
pub const MAX_WIDTH_RANGE: std::ops::RangeInclusive<u32> = 40..=90;
/// One line (only the current one) or three (previous, current, next).
pub const LINES: [u32; 2] = [1, 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    BottomCenter,
    TopCenter,
    BottomLeft,
    BottomRight,
    /// Set by Ctrl+drag: the strip is where `Settings::position` says.
    Custom,
}

impl Placement {
    /// The places the user can choose; `Custom` only comes from dragging.
    pub const CHOICES: [Placement; 4] = [Self::BottomCenter, Self::TopCenter, Self::BottomLeft, Self::BottomRight];

    pub fn id(self) -> &'static str {
        match self {
            Self::BottomCenter => "bottom_center",
            Self::TopCenter => "top_center",
            Self::BottomLeft => "bottom_left",
            Self::BottomRight => "bottom_right",
            Self::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> Option<Placement> {
        Self::CHOICES.into_iter().find(|p| p.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    White,
    Yellow,
    Green,
    Blue,
}

impl Accent {
    pub const ALL: [Accent; 4] = [Self::White, Self::Yellow, Self::Green, Self::Blue];

    pub fn id(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
        }
    }

    /// Colour of the current line and the lit words. Each is at least 4.5:1 against the black outline (see the test).
    pub fn hex(self) -> &'static str {
        match self {
            Self::White => "#FFFFFF",
            Self::Yellow => "#FFD54A",
            Self::Green => "#9BE59B",
            Self::Blue => "#8FD3FF",
        }
    }
}

/// A named set of display settings. `Custom` is what any other combination is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Default,
    Minimal,
    Karaoke,
    Focus,
    Custom,
}

/// The display values of a preset. A preset places the strip by `placement`, so it also clears the dragged position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Look {
    pub placement: Placement,
    pub lines: u32,
    pub font_px: u32,
    pub plate_opacity: u32,
    pub max_width_pct: u32,
    pub accent: Accent,
}

impl Preset {
    pub const CHOICES: [Preset; 4] = [Self::Default, Self::Minimal, Self::Karaoke, Self::Focus];

    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Minimal => "minimal",
            Self::Karaoke => "karaoke",
            Self::Focus => "focus",
            Self::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> Option<Preset> {
        Self::CHOICES.into_iter().find(|p| p.id() == id)
    }

    /// `None` for `Custom`.
    pub fn look(self) -> Option<Look> {
        use Accent::{Blue, White, Yellow};
        use Placement::{BottomCenter, TopCenter};
        let (placement, lines, font_px, plate_opacity, max_width_pct, accent) = match self {
            Self::Default => (BottomCenter, 3, 22, 35, 60, White),
            Self::Minimal => (BottomCenter, 1, 20, 0, 60, White),
            Self::Karaoke => (BottomCenter, 3, 30, 55, 80, Yellow),
            Self::Focus => (TopCenter, 1, 22, 25, 50, Blue),
            Self::Custom => return None,
        };
        Some(Look { placement, lines, font_px, plate_opacity, max_width_pct, accent })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font_px: u32,
    pub visible: bool,
    pub hide_from_capture: bool,
    /// Where the strip is when `placement` is `Custom` (Ctrl+drag). `None` for every other placement.
    pub position: Option<(i32, i32)>,
    pub placement: Placement,
    /// 1 shows only the current line, 3 also the previous and the next.
    pub lines: u32,
    /// Black plate behind the text, in percent. 0 is no plate; the text keeps its outline and shadow.
    pub plate_opacity: u32,
    /// Strip width in percent of the work area.
    pub max_width_pct: u32,
    pub accent: Accent,
    /// Fixed lyric delay per player, keyed by the SMTC app id.
    pub app_offsets_ms: BTreeMap<String, i32>,
}

impl Default for Settings {
    fn default() -> Self {
        let look = Preset::Default.look().expect("the default preset has values");
        Self {
            font_px: look.font_px,
            visible: true,
            hide_from_capture: true,
            position: None,
            placement: look.placement,
            lines: look.lines,
            plate_opacity: look.plate_opacity,
            max_width_pct: look.max_width_pct,
            accent: look.accent,
            app_offsets_ms: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Brings fields that disagree back in line: values into their ranges, and `placement` with `position`
    /// (a saved position means `Custom`, `Custom` without one means the default place).
    pub fn normalize(&mut self) {
        self.font_px = self.font_px.clamp(*FONT_PX_RANGE.start(), *FONT_PX_RANGE.end());
        self.plate_opacity = self.plate_opacity.clamp(*PLATE_OPACITY_RANGE.start(), *PLATE_OPACITY_RANGE.end());
        self.max_width_pct = self.max_width_pct.clamp(*MAX_WIDTH_RANGE.start(), *MAX_WIDTH_RANGE.end());
        if !LINES.contains(&self.lines) {
            self.lines = Self::default().lines;
        }
        if self.position.is_some() {
            self.placement = Placement::Custom;
        } else if self.placement == Placement::Custom {
            self.placement = Placement::default();
        }
    }

    /// Chooses a place. Anything but `Custom` forgets the dragged position.
    pub fn set_placement(&mut self, placement: Placement) {
        self.placement = placement;
        if placement != Placement::Custom {
            self.position = None;
        }
        self.normalize();
    }

    /// Sets every display value of the preset in one go. The offsets, visibility and capture setting stay.
    pub fn apply_preset(&mut self, preset: Preset) {
        let Some(look) = preset.look() else { return };
        self.font_px = look.font_px;
        self.lines = look.lines;
        self.plate_opacity = look.plate_opacity;
        self.max_width_pct = look.max_width_pct;
        self.accent = look.accent;
        self.set_placement(look.placement);
    }

    /// The preset whose values all match, else `Custom`. Not stored: it follows from the other fields.
    pub fn preset(&self) -> Preset {
        let now = Look {
            placement: self.placement,
            lines: self.lines,
            font_px: self.font_px,
            plate_opacity: self.plate_opacity,
            max_width_pct: self.max_width_pct,
            accent: self.accent,
        };
        Preset::CHOICES.into_iter().find(|p| p.look() == Some(now)).unwrap_or(Preset::Custom)
    }
}

/// Every change to the two files is load, modify, save. The engine, the tray, the settings window and the
/// Ctrl drag do this from different threads, so one lock covers the whole sequence.
static WRITE: Mutex<()> = Mutex::new(());

fn locked() -> std::sync::MutexGuard<'static, ()> {
    WRITE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Best effort like the lyric cache: a file that cannot be read or written is the same as no file.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Defaults when the file is missing or corrupt. Values are put in their ranges (`Settings::normalize`).
    pub fn load(&self) -> Settings {
        let mut s: Settings = self.read("settings.json").unwrap_or_default();
        s.normalize();
        s
    }

    pub fn save(&self, s: &Settings) {
        let _one_at_a_time = locked();
        self.write("settings.json", s);
    }

    /// Loads, applies `change` and saves, all under the write lock, so two changes made at the same time both
    /// stay. Returns what was saved.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let _one_at_a_time = locked();
        let mut s = self.load();
        change(&mut s);
        self.write("settings.json", &s);
        s
    }

    pub fn song_offset_ms(&self, cache_key: &str) -> i32 {
        self.offsets().get(cache_key).copied().unwrap_or(0)
    }

    /// 0 removes the entry.
    pub fn set_song_offset_ms(&self, cache_key: &str, ms: i32) {
        let _one_at_a_time = locked();
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
            if let Err(e) = fs::rename(&temp, &path) {
                eprintln!("encore: cannot save {}: {e}", path.display());
            }
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
            let dir = std::env::temp_dir().join(format!("encore-settings-test-{}-{test}", std::process::id()));
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
        let mut s = Settings { font_px: 30, position: Some((-5, 700)), placement: Placement::Custom, ..loaded };
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
    fn updates_from_two_threads_keep_both_fields() {
        let t = Temp::new("two-threads");
        let store = &t.0;
        for round in 0..200 {
            store.save(&Settings::default());
            std::thread::scope(|scope| {
                scope.spawn(|| _ = store.update(|s| s.visible = false));
                scope.spawn(|| _ = store.update(|s| s.position = Some((round, 7))));
            });
            let s = store.load();
            assert!(!s.visible && s.position == Some((round, 7)), "round {round}: {s:?}");
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

    #[test]
    fn new_fields_default_and_an_old_file_still_loads() {
        let t = Temp::new("old-file");
        fs::create_dir_all(&t.0.dir).unwrap();
        fs::write(
            t.0.dir.join("settings.json"),
            r#"{"font_px":30,"visible":false,"hide_from_capture":false,"app_offsets_ms":{"Spotify.exe":200}}"#,
        )
        .unwrap();
        let s = t.0.load();
        assert_eq!((s.font_px, s.visible, s.hide_from_capture, s.app_offsets_ms["Spotify.exe"]), (30, false, false, 200));
        assert_eq!(
            (s.placement, s.lines, s.plate_opacity, s.max_width_pct, s.accent),
            (Placement::BottomCenter, 3, 35, 60, Accent::White)
        );
        assert_eq!(s.preset(), Preset::Custom, "a size of 30 matches no preset");
    }

    #[test]
    fn an_old_saved_position_means_custom_placement() {
        let t = Temp::new("old-position");
        fs::create_dir_all(&t.0.dir).unwrap();
        fs::write(t.0.dir.join("settings.json"), r#"{"position":[10,20]}"#).unwrap();
        let s = t.0.load();
        assert_eq!((s.placement, s.position), (Placement::Custom, Some((10, 20))));
        // And a custom placement without a position is the default place.
        fs::write(t.0.dir.join("settings.json"), r#"{"placement":"custom"}"#).unwrap();
        assert_eq!(t.0.load().placement, Placement::BottomCenter);
    }

    #[test]
    fn stored_values_are_put_in_range() {
        let t = Temp::new("ranges");
        t.0.save(&Settings { plate_opacity: 99, max_width_pct: 10, lines: 2, ..Settings::default() });
        let s = t.0.load();
        assert_eq!((s.plate_opacity, s.max_width_pct, s.lines), (70, 40, 3));
        t.0.save(&Settings { max_width_pct: 500, ..Settings::default() });
        assert_eq!(t.0.load().max_width_pct, 90);
    }

    #[test]
    fn an_unknown_enum_value_loads_as_the_default_file() {
        let t = Temp::new("bad-enum");
        fs::create_dir_all(&t.0.dir).unwrap();
        fs::write(t.0.dir.join("settings.json"), r#"{"accent":"pink"}"#).unwrap();
        assert_eq!(t.0.load(), Settings::default());
    }

    #[test]
    fn the_presets_have_the_agreed_values() {
        let row = |p: Preset| {
            let l = p.look().unwrap();
            (l.placement, l.lines, l.font_px, l.plate_opacity, l.max_width_pct, l.accent)
        };
        use Accent::{Blue, White, Yellow};
        use Placement::{BottomCenter, TopCenter};
        assert_eq!(row(Preset::Default), (BottomCenter, 3, 22, 35, 60, White));
        assert_eq!(row(Preset::Minimal), (BottomCenter, 1, 20, 0, 60, White));
        assert_eq!(row(Preset::Karaoke), (BottomCenter, 3, 30, 55, 80, Yellow));
        assert_eq!(row(Preset::Focus), (TopCenter, 1, 22, 25, 50, Blue));
        assert_eq!(Preset::Custom.look(), None);
        assert_eq!(Settings::default().preset(), Preset::Default);
        for p in Preset::CHOICES {
            assert_eq!(Preset::from_id(p.id()), Some(p));
        }
        assert_eq!(Preset::from_id("custom"), None);
    }

    #[test]
    fn applying_a_preset_sets_every_value_and_keeps_the_rest() {
        let mut s = Settings { visible: false, hide_from_capture: false, position: Some((5, 5)), ..Settings::default() };
        s.normalize();
        s.app_offsets_ms.insert("Spotify.exe".into(), 200);
        for p in Preset::CHOICES {
            s.apply_preset(p);
            assert_eq!(s.preset(), p);
            assert_eq!(s.position, None);
            assert_eq!((s.visible, s.hide_from_capture, s.app_offsets_ms["Spotify.exe"]), (false, false, 200));
        }
        let before = s.clone();
        s.apply_preset(Preset::Custom);
        assert_eq!(s, before, "custom is not something to apply");
    }

    #[test]
    fn a_manual_change_makes_the_preset_custom() {
        let changes: [fn(&mut Settings); 6] = [
            |s| s.font_px += 1,
            |s| s.lines = if s.lines == 1 { 3 } else { 1 },
            |s| s.plate_opacity += 1,
            |s| s.max_width_pct += 1,
            |s| s.accent = if s.accent == Accent::Green { Accent::Blue } else { Accent::Green },
            |s| s.placement = if s.placement == Placement::BottomRight { Placement::BottomLeft } else { Placement::BottomRight },
        ];
        for p in Preset::CHOICES {
            let mut base = Settings::default();
            base.apply_preset(p);
            for change in changes {
                let mut s = base.clone();
                change(&mut s);
                assert_eq!(s.preset(), Preset::Custom, "{p:?}");
            }
            let mut dragged = base.clone();
            dragged.position = Some((1, 2));
            dragged.normalize();
            assert_eq!((dragged.placement, dragged.preset()), (Placement::Custom, Preset::Custom));
        }
    }

    #[test]
    fn choosing_a_placement_forgets_the_dragged_position() {
        let mut s = Settings { position: Some((1, 2)), ..Settings::default() };
        s.normalize();
        s.set_placement(Placement::BottomLeft);
        assert_eq!((s.placement, s.position), (Placement::BottomLeft, None));
        s.set_placement(Placement::Custom);
        assert_eq!(s.placement, Placement::BottomCenter, "custom needs a position");
    }

    #[test]
    fn accent_colours_keep_4_5_to_1_against_the_black_outline() {
        fn luminance(hex: &str) -> f64 {
            let c = |i: usize| {
                let v = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
        }
        for a in [Accent::White, Accent::Yellow, Accent::Green, Accent::Blue] {
            let ratio = (luminance(a.hex()) + 0.05) / 0.05;
            assert!(ratio >= 4.5, "{a:?} {ratio:.1}");
            eprintln!("{a:?} {} {ratio:.1}:1", a.hex());
        }
    }
}
