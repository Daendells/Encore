//! Language detection and the texts of the tray and the settings window. Indonesian is the source.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Id,
    En,
}

/// "id", "id-ID" and "ID_id" are Indonesian; anything else is English.
pub fn from_locale(name: &str) -> Lang {
    let name = name.trim().to_ascii_lowercase();
    match name.strip_prefix("id") {
        Some(rest) if rest.is_empty() || rest.starts_with(['-', '_']) => Lang::Id,
        _ => Lang::En,
    }
}

/// The user's Windows language. Debug builds also honour `LARIK_LANG=id|en`, for screenshots and tests.
pub fn detect() -> Lang {
    if cfg!(debug_assertions) {
        match std::env::var("LARIK_LANG").as_deref() {
            Ok("id") => return Lang::Id,
            Ok("en") => return Lang::En,
            _ => {}
        }
    }
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let len = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut buf) };
    let len = usize::try_from(len).unwrap_or(0).saturating_sub(1); // the count includes the final NUL
    from_locale(&String::from_utf16_lossy(&buf[..len]))
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
    ("tray.reset_position", "Kembalikan posisi ke bawah tengah", "Move lyrics back to bottom center"),
    ("tray.autostart", "Mulai saat Windows menyala", "Start with Windows"),
    ("tray.settings", "Pengaturan", "Settings"),
    ("tray.quit", "Keluar", "Quit"),
    ("settings.title", "Pengaturan Larik", "Larik settings"),
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
    fn locale_names_map_to_a_language() {
        for name in ["id-ID", "id", "ID-id", "id_ID"] {
            assert_eq!(from_locale(name), Lang::Id, "{name}");
        }
        for name in ["en-US", "ja-JP", "", "ind", "idx-XX", "de-DE"] {
            assert_eq!(from_locale(name), Lang::En, "{name}");
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
