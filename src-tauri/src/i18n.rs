//! Language detection and the texts of the tray and the settings window. Indonesian is the source.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Id,
    En,
}

/// A Windows language id: primary language 0x21 is Indonesian, anything else is English.
pub fn from_langid(id: u16) -> Lang {
    if id & 0x3ff == 0x21 { Lang::Id } else { Lang::En }
}

/// The language Windows shows its own interface in. Debug builds also honour `ENCORE_LANG=id|en`, for screenshots and tests.
pub fn detect() -> Lang {
    if cfg!(debug_assertions) {
        match std::env::var("ENCORE_LANG").as_deref() {
            Ok("id") => return Lang::Id,
            Ok("en") => return Lang::En,
            _ => {}
        }
    }
    from_langid(unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() })
}

/// (key, Indonesian, English)
const TEXTS: &[(&str, &str, &str)] = &[
    ("lang", "id", "en"),
    ("tray.show", "Tampilkan lirik", "Show lyrics"),
    ("tray.hide", "Sembunyikan lirik", "Hide lyrics"),
    ("tray.size", "Ukuran teks", "Text size"),
    ("tray.size.small", "Kecil", "Small"),
    ("tray.size.medium", "Sedang", "Medium"),
    ("tray.size.large", "Besar", "Large"),
    ("tray.look", "Tampilan", "Appearance"),
    ("tray.position", "Posisi", "Position"),
    ("preset.default", "Bawaan", "Default"),
    ("preset.minimal", "Minimalis", "Minimal"),
    ("preset.karaoke", "Karaoke", "Karaoke"),
    ("preset.focus", "Fokus", "Focus"),
    ("preset.custom", "Kustom", "Custom"),
    ("place.bottom_center", "Bawah tengah", "Bottom center"),
    ("place.top_center", "Atas tengah", "Top center"),
    ("place.bottom_left", "Kiri bawah", "Bottom left"),
    ("place.bottom_right", "Kanan bawah", "Bottom right"),
    ("place.custom", "Digeser sendiri", "Moved by hand"),
    ("accent.white", "Putih", "White"),
    ("accent.yellow", "Kuning", "Yellow"),
    ("accent.green", "Hijau", "Green"),
    ("accent.blue", "Biru", "Blue"),
    ("tray.reset_position", "Kembalikan posisi ke bawah tengah", "Move lyrics back to bottom center"),
    ("tray.autostart", "Mulai saat Windows menyala", "Start with Windows"),
    ("tray.settings", "Pengaturan", "Settings"),
    ("tray.quit", "Keluar", "Quit"),
    ("settings.title", "Pengaturan Encore", "Encore settings"),
    ("settings.look", "Tampilan", "Appearance"),
    ("settings.preset", "Gaya", "Style"),
    ("settings.position", "Posisi", "Position"),
    ("settings.lines", "Jumlah baris", "Lines shown"),
    ("settings.lines.1", "1 baris", "1 line"),
    ("settings.lines.3", "3 baris", "3 lines"),
    (
        "settings.lines.help",
        "Satu baris hanya menampilkan baris yang sedang dinyanyikan.",
        "One line shows only the line being sung.",
    ),
    ("settings.plate", "Latar gelap di belakang teks", "Dark background behind the text"),
    (
        "settings.plate.help",
        "Di 0% tidak ada latar. Teks tetap punya garis tepi hitam.",
        "At 0% there is no background. The text keeps its black outline.",
    ),
    ("settings.width", "Lebar, dari lebar layar", "Width, as a share of the screen"),
    ("settings.accent", "Warna baris yang sedang dinyanyikan", "Color of the line being sung"),
    ("settings.reset_look", "Kembalikan ke bawaan", "Reset to default"),
    ("settings.size", "Ukuran teks", "Text size"),
    ("settings.offsets", "Geser waktu lirik per aplikasi", "Lyric timing per app"),
    (
        "settings.offsets.help",
        "Kalau lirik selalu terlambat atau terlalu cepat di satu aplikasi, geser di sini. Angka positif membuat lirik muncul lebih awal.",
        "If lyrics are always late or early in one app, adjust them here. A positive number shows lyrics earlier.",
    ),
    (
        "settings.offsets.empty",
        "Belum ada aplikasi. Putar lagu dulu, nanti aplikasinya muncul di sini.",
        "No apps yet. Play a song and the app will show up here.",
    ),
    (
        "settings.capture",
        "Sembunyikan lirik dari berbagi layar dan tangkapan layar",
        "Hide lyrics from screen sharing and screenshots",
    ),
    (
        "settings.capture.help",
        "Lirik tetap terlihat di layarmu sendiri. Perlu Windows 10 versi 2004 atau lebih baru.",
        "Lyrics stay visible on your own screen. Needs Windows 10 version 2004 or newer.",
    ),
    ("settings.autostart", "Mulai saat Windows menyala", "Start with Windows"),
    ("settings.shortcuts", "Pintasan", "Shortcuts"),
    ("settings.shortcut.toggle", "Tampilkan atau sembunyikan lirik", "Show or hide lyrics"),
    ("settings.shortcut.earlier", "Lirik 250 ms lebih awal", "Lyrics 250 ms earlier"),
    ("settings.shortcut.later", "Lirik 250 ms lebih lambat", "Lyrics 250 ms later"),
    ("settings.shortcut.settings", "Buka pengaturan", "Open settings"),
    (
        "settings.shortcut.unavailable",
        "Tidak bisa dipakai karena sudah diambil aplikasi lain",
        "Unavailable because another app already uses it",
    ),
    (
        "settings.move_hint",
        "Tahan Ctrl lalu seret lirik untuk memindahkannya.",
        "Hold Ctrl and drag the lyrics to move them.",
    ),
    ("settings.version", "Versi", "Version"),
    (
        "settings.error",
        "Pengaturan tidak tersimpan. Periksa lagi angka yang kamu isi.",
        "Settings were not saved. Check the numbers you entered.",
    ),
    ("unit.ms", "ms", "ms"),
    ("unit.px", "px", "px"),
];

/// The text for `key`; an unknown key gives the key itself.
pub fn t(lang: Lang, key: &str) -> &str {
    match TEXTS.iter().find(|(k, ..)| *k == key) {
        Some((_, id, en)) => match lang {
            Lang::Id => id,
            Lang::En => en,
        },
        None => key,
    }
}

pub fn all_keys() -> Vec<&'static str> {
    TEXTS.iter().map(|(k, ..)| *k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids_map_to_a_language() {
        // Indonesian has primary language 0x21 (id-ID is 0x0421); the sublanguage bits do not matter.
        for id in [0x0421, 0x0021, 0x0821] {
            assert_eq!(from_langid(id), Lang::Id, "{id:#06x}");
        }
        // en-US, ja-JP, de-DE, Arabic (0x0001) and zero.
        for id in [0x0409, 0x0411, 0x0407, 0x0001, 0x0000] {
            assert_eq!(from_langid(id), Lang::En, "{id:#06x}");
        }
    }

    #[test]
    fn every_key_has_both_languages() {
        for key in all_keys() {
            for lang in [Lang::Id, Lang::En] {
                let text = t(lang, key);
                assert!(!text.is_empty() && (text != key), "{key} {lang:?}");
            }
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut keys = all_keys();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), TEXTS.len());
    }

    #[test]
    fn no_em_dash_in_any_string() {
        for (key, id, en) in TEXTS {
            assert!(!id.contains('\u{2014}') && !en.contains('\u{2014}'), "{key}");
        }
    }

    #[test]
    fn unknown_key_is_returned_as_is() {
        assert_eq!(t(Lang::Id, "nope"), "nope");
    }
}
