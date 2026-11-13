use unicode_normalization::UnicodeNormalization;

/// A group or " - " suffix holding one of these as a whole word is a decoration.
const DECORATIONS: [&str; 25] = [
    "remaster", "remastered", "live", "feat", "ft.", "featuring", "mono", "stereo", "deluxe", "bonus", "explicit",
    "clean", "acoustic", "from ", "ost", "official", "video", "audio", "lyric", "lyrics", "mv", "m/v", "visualizer",
    "hd", "4k",
];
/// These decorate only next to words that leave the lyrics alone: "(Album Version)" is removed,
/// "(Japanese Version)" is kept so that it can only match a record with the same qualifier.
const WEAK_DECORATIONS: [&str; 4] = ["version", "ver", "edit", "mix"];
/// Besides a year and an ordinal ("25th"). The neutral words that are in DECORATIONS need no entry here.
const NEUTRAL: [&str; 9] =
    ["album", "single", "radio", "original", "extended", "studio", "digital", "anniversary", "edition"];
/// Longer titles or artists are not looked up: a web page can put anything in its media title.
pub const MAX_INPUT_CHARS: usize = 300;
const BRACKETS: [(char, char); 5] = [('(', ')'), ('[', ']'), ('（', '）'), ('［', '］'), ('【', '】')];
const DASHES: [&str; 3] = [" - ", " – ", " — "];
/// Where the end of a title may be cut: the dashes and the bar of "Title | Official MV".
const TAIL_SEPARATORS: [&str; 4] = [" - ", " – ", " — ", " | "];
/// A group or tail holding one of these is another recording or version, never a decoration.
const QUALIFIERS: [&str; 6] = ["remix", "instrumental", "inst", "karaoke", "acapella", "a cappella"];
/// What may follow a trailing "Official" for the whole phrase to go.
const OFFICIAL_TAIL: [&str; 10] = ["music", "video", "audio", "lyric", "lyrics", "mv", "m/v", "visualizer", "hd", "4k"];

/// Removes decorations but keeps case and script: "(Remastered 2011)", "- Live at X", "(feat. Y)" ...
pub fn clean_title(s: &str) -> String {
    let groupless = collapse(&strip_groups(s));
    let mut rest = groupless.as_str();
    while let Some(shorter) = cut_tail(rest) {
        rest = shorter.trim_end();
    }
    if rest.is_empty() {
        s.trim().to_string()
    } else {
        rest.to_string()
    }
}

/// One cut off the end of a title: a trailing "Official Video" or "MV", "feat. X", or a decoration after a
/// dash or bar. None when nothing is left to cut. Brackets are not looked into: `strip_groups` decided on them.
fn cut_tail(s: &str) -> Option<&str> {
    let lower = outside_brackets(s);
    let official = lower.rfind(" official ").filter(|&at| {
        let mut tail = lower[at..].split_whitespace().skip(1).peekable();
        tail.peek().is_some() && tail.all(|w| OFFICIAL_TAIL.contains(&w))
    });
    let mv = lower.rfind(' ').filter(|&at| matches!(&lower[at + 1..], "mv" | "m/v"));
    // "feat." before a dash belongs to the artist and stays.
    // A bracket after it ("ft. X (Japanese Version)") may hold a qualifier, so nothing is cut there either.
    let featured = featured_at(&lower).filter(|&at| {
        split_dash(&s[at..], false, &DASHES).is_none()
            && !s[at..].contains(|c| BRACKETS.iter().any(|b| b.0 == c || b.1 == c))
    });
    let cut = [official, mv, featured].into_iter().flatten().min().map(|at| &s[..at]);
    // A bare separator left over ("Song - Official Video" cut at "Official") goes too.
    let cut = cut.map(|head| head.trim_end().trim_end_matches(['-', '–', '—', '|']).trim_end());
    cut.or_else(|| split_dash(s, true, &TAIL_SEPARATORS).filter(|(_, tail)| is_decoration(tail, false)).map(|(head, _)| head))
        .filter(|head| !head.is_empty())
}

/// Lowercase copy of `s` with everything inside brackets blanked to `_`, byte for byte, so offsets carry over.
fn outside_brackets(s: &str) -> String {
    let mut depth = 0usize;
    s.chars()
        .map(|c| {
            if BRACKETS.iter().any(|b| b.0 == c) {
                depth += 1;
            }
            let inside = depth > 0;
            if BRACKETS.iter().any(|b| b.1 == c) {
                depth = depth.saturating_sub(1);
            }
            if inside { "_".repeat(c.len_utf8()) } else { c.to_ascii_lowercase().to_string() }
        })
        .collect()
}

/// Where " feat", " ft." or " featuring" starts as a whole word in a lowercase string.
fn featured_at(lower: &str) -> Option<usize> {
    [" feat", " ft.", " featuring"].iter().filter_map(|p| find_word(lower, p)).min()
}

pub fn clean_artist(s: &str) -> String {
    let full = s.trim();
    let mut rest = full.strip_suffix(" - Topic").unwrap_or(full);
    rest = rest.strip_suffix("VEVO").unwrap_or(rest);
    let lower = rest.to_ascii_lowercase();
    if let Some(at) = featured_at(&lower) {
        rest = &rest[..at];
    }
    let rest = rest.trim();
    if rest.is_empty() { full } else { rest }.to_string()
}

/// Comparison key: NFKD, combining marks U+0300..=U+036F removed, lowercase, "&" -> "and",
/// every char that is not alphanumeric (Unicode) or a space removed, whitespace collapsed.
pub fn key(s: &str) -> String {
    // Recomposing keeps marks outside the Latin range attached, so が does not fall back to か.
    let folded: String = s.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).nfc().collect();
    let spelled = folded.to_lowercase().replace('&', " and ");
    collapse(&spelled.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>())
}

/// Query pairs (title, artist) to try, most likely first, deduplicated. Always includes the cleaned pair;
/// adds ("Judul", "Artis") when the title has the form "Artis - Judul" (spec B12).
/// Empty when the title or the artist is longer than MAX_INPUT_CHARS.
pub fn candidates(title: &str, artist: &str) -> Vec<(String, String)> {
    if too_long(title) || too_long(artist) {
        return Vec::new();
    }
    let mut pairs = vec![(clean_title(title), clean_artist(artist))];
    let groupless = collapse(&strip_groups(title));
    if let Some((left, right)) = split_dash(&groupless, false, &DASHES) {
        let split = (clean_title(right), clean_artist(left));
        if !split.0.is_empty() && !split.1.is_empty() && split != pairs[0] {
            let spaceless = |s: &str| key(s).replace(' ', "");
            // The channel name repeats the left side ("KeaneVEVO"): the title is certainly "Artis - Judul".
            let at = if spaceless(&split.1) == spaceless(&pairs[0].1) { 0 } else { 1 };
            pairs.insert(at, split);
            // "Artis - Judul (Official Video)" cleans to "Artis" when "Judul" reads like a decoration: that is no title.
            if at == 0 && key(&pairs[1].0) == key(left) {
                pairs.remove(1);
            }
        }
    }
    pairs
}

pub(crate) fn too_long(s: &str) -> bool {
    s.chars().nth(MAX_INPUT_CHARS).is_some()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drops every bracketed group that is a decoration. A group that is kept is still searched for inner groups.
fn strip_groups(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        let group = BRACKETS.iter().find(|b| b.0 == c).and_then(|&(open, close)| {
            let end = group_end(rest, open, close)?;
            is_decoration(&rest[..end], matches!(open, '(' | '（')).then_some(end + close.len_utf8())
        });
        match group {
            Some(len) => rest = &rest[len..],
            None => out.push(c),
        }
    }
    out
}

/// Byte offset of the closer that matches an opener just before `s`.
fn group_end(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
    }
    None
}

/// Splits at the first or the last spaced dash.
fn split_dash<'a>(s: &'a str, last: bool, separators: &[&str]) -> Option<(&'a str, &'a str)> {
    let hits = separators.iter().filter_map(|d| if last { s.rfind(d) } else { s.find(d) }.map(|at| (at, d.len())));
    let (at, len) = if last { hits.max() } else { hits.min() }?;
    Some((&s[..at], &s[at + len..]))
}

fn is_decoration(content: &str, parenthesised: bool) -> bool {
    let content = content.trim().to_ascii_lowercase();
    if QUALIFIERS.iter().any(|w| find_word(&content, w).is_some()) {
        return false;
    }
    if WEAK_DECORATIONS.iter().any(|w| find_word(&content, w).is_some()) {
        // "(Japanese Version Official Video)" is another version whatever else it says, and so is "(🇯🇵 Ver.)".
        let plain = content.chars().all(|c| c.is_alphanumeric() || c.is_whitespace() || c.is_ascii_punctuation());
        let neutral = |w: &str| {
            w.is_empty()
                || WEAK_DECORATIONS.contains(&w)
                || NEUTRAL.contains(&w)
                || is_year(w)
                || is_ordinal(w)
                || DECORATIONS.iter().any(|d| d.trim_end_matches(['.', ' ']) == w)
        };
        return plain && content.split(|c: char| !c.is_alphanumeric() && c != '/').all(neutral);
    }
    is_year(&content)
        || DECORATIONS.iter().any(|w| find_word(&content, w).is_some())
        || (parenthesised && find_word(&content, "with ").is_some())
}

fn is_year(s: &str) -> bool {
    s.len() == 4 && (s.starts_with("19") || s.starts_with("20")) && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_ordinal(s: &str) -> bool {
    let digits = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    ["st", "nd", "rd", "th"].iter().any(|end| s.strip_suffix(end).is_some_and(digits))
}

/// First place where `word` stands as a whole word: "live" is not found in "alive" or "olivia".
fn find_word(s: &str, word: &str) -> Option<usize> {
    // Scripts without letter case (kanji, kana, hangul) do not glue to a Latin word: "公式MV" holds the word "mv".
    let wordy = |c: char| c.is_ascii_alphanumeric() || c.is_lowercase() || c.is_uppercase();
    s.match_indices(word).map(|(at, _)| at).find(|&at| {
        let glued_before = word.starts_with(wordy) && s[..at].ends_with(wordy);
        let glued_after = word.ends_with(wordy) && s[at + word.len()..].starts_with(wordy);
        !glued_before && !glued_after
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(title: &str, artist: &str) -> (String, String) {
        (title.to_string(), artist.to_string())
    }

    #[test]
    fn clean_title_removes_decorations() {
        for (input, want) in [
            ("Bohemian Rhapsody - Remastered 2011", "Bohemian Rhapsody"),
            ("Hotel California (Live on MTV, 1994)", "Hotel California"),
            ("Stay (with Justin Bieber)", "Stay"),
            ("Lagu (feat. Artis Lain)", "Lagu"),
            ("Song [Official Music Video]", "Song"),
            ("Title - 2011 Remaster", "Title"),
            ("Song (ft. Someone)", "Song"),
            ("Song (2011)", "Song"),
            ("Song (M/V)", "Song"),
            ("Song [4K]", "Song"),
            ("Song (Goblin OST)", "Song"),
            ("Let It Go - From \"Frozen\"", "Let It Go"),
            ("Song (Radio Edit) Part 2", "Song Part 2"),
            ("曲名（feat. 誰か）", "曲名"),
            ("曲名【Official Video】", "曲名"),
            ("曲名【公式MV】", "曲名"),
            ("Song – Remastered 2011", "Song"),
        ] {
            assert_eq!(clean_title(input), want, "{input}");
        }
    }

    #[test]
    fn clean_title_keeps_titles_without_decorations() {
        for input in [
            "(Sittin' On) The Dock of the Bay",
            "Live",
            "夜に駆ける",
            "봄날 (Spring Day)",
            "Keane - Somewhere Only We Know",
            "Song - With You",
            "Song (Live",
            "1999",
        ] {
            assert_eq!(clean_title(input), input);
        }
    }

    #[test]
    fn clean_title_matches_whole_words_only() {
        for input in [
            "Song (Alive)",
            "Song (Olivia)",
            "Song (Edith)",
            "Song (Mixtape)",
            "Song (Skrillex Remix)",
            "Song (Cleaner)",
            "Song (Turn Left.)",
            "Song (Most Wanted)",
            "Song (20112)",
            "Stay - Alive",
        ] {
            assert_eq!(clean_title(input), input);
        }
    }

    #[test]
    fn clean_title_removes_stacked_decorations() {
        assert_eq!(clean_title("Song (feat. X) - Remastered 2011 [Official Audio]"), "Song");
        assert_eq!(clean_title("Song - Live - 2009 Remaster"), "Song");
        assert_eq!(clean_title("Song (feat. X (of Y)) [HD]"), "Song");
    }

    #[test]
    fn version_group_with_lyric_changing_qualifier_is_kept() {
        for input in [
            "Cupid - Twin Version",
            "Song (Japanese Version)",
            "Song (Instrumental Version)",
            "Song (Karaoke Version)",
            "Song -Japanese ver.-",
            "Song (Japanese Ver.)",
            "Song (日本語 Version)",
            "Song (Club Mix)",
            "Song (Fan Edit)",
            "Song (Version 2)",
            "Love Story (Taylor's Version)",
        ] {
            assert_eq!(clean_title(input), input);
        }
        for (input, want) in [
            ("Song (Album Version)", "Song"),
            ("Song - Single Version", "Song"),
            ("Song - Radio Edit", "Song"),
            ("Title - 2011 Remaster", "Title"),
            ("Song (Original Mix)", "Song"),
            ("Song (Extended Version)", "Song"),
            ("Song (Album Ver.)", "Song"),
            ("Song (2019 Mix)", "Song"),
            ("Song (25th Anniversary Version)", "Song"),
            ("Song (2nd Version)", "Song"),
            ("Song (Japanese Version) [Official Video]", "Song (Japanese Version)"),
        ] {
            assert_eq!(clean_title(input), want, "{input}");
        }
    }

    #[test]
    fn qualifiers_next_to_decorations_are_kept() {
        for input in [
            "Song (Japanese Version Official Video)",
            "Song [English Ver. MV]",
            "Song (Japanese Ver. M/V)",
            "Song - Live (Japanese Version)",
            "Song (Skrillex Remix feat. X)",
            "Song (Skrillex Remix)",
            "Song - Remix",
            "Song (Instrumental)",
            "Song [Karaoke]",
            "Song (Acapella)",
            "Song (A Cappella)",
            "Song (Inst.)",
            "Song (🇯🇵 Ver.)",
        ] {
            assert_eq!(clean_title(input), input, "{input}");
        }
        assert_eq!(clean_title("Song (Album Ver. M/V)"), "Song");
        assert_eq!(clean_title("Song (Live Version)"), "Song");
    }

    #[test]
    fn youtube_endings_outside_brackets_are_cut() {
        for (input, want) in [
            ("Song Official Video", "Song"),
            ("Song Official Music Video", "Song"),
            ("Song Official Audio", "Song"),
            ("Song Official Lyric Video", "Song"),
            ("Song Official Visualizer", "Song"),
            ("Song MV", "Song"),
            ("Song M/V", "Song"),
            ("Artist - Song Official MV", "Artist - Song"),
            ("Song | Official MV", "Song"),
            ("Song | Official Video", "Song"),
            ("Song ft. Someone", "Song"),
            ("Song feat. Someone Else", "Song"),
            ("Song Featuring Someone", "Song"),
            ("Song ft. Someone Official Video", "Song"),
            ("Song ft. X", "Song"),
            ("Song ft. X (Japanese Version)", "Song ft. X (Japanese Version)"),
            ("Song feat. X (Skrillex Remix)", "Song feat. X (Skrillex Remix)"),
            ("Song ft. X [English Ver.]", "Song ft. X [English Ver.]"),
            ("Artist feat. B - Song", "Artist feat. B - Song"),
            ("Song | Artist", "Song | Artist"),
            ("The Official Story", "The Official Story"),
            ("Official Secrets", "Official Secrets"),
            ("Soft. Cell", "Soft. Cell"),
            ("MV", "MV"),
        ] {
            assert_eq!(clean_title(input), want, "{input}");
        }
    }

    #[test]
    fn candidates_never_collapse_the_title_to_the_artist() {
        // The channel repeats the artist: "Oasis" by Oasis would be another song.
        for channel in ["Oasis", "OasisVEVO", "Oasis - Topic"] {
            let got = candidates("Oasis - Live Forever (Official Video)", channel);
            assert_eq!(got, [pair("Live Forever", "Oasis")], "{channel}");
        }
    }

    #[test]
    fn overlong_input_is_rejected() {
        let long = "a".repeat(301);
        assert_eq!(candidates(&long, "Artist"), []);
        assert_eq!(candidates("Song", &long), []);
        assert_eq!(candidates(&long[..300], "Artist").len(), 1);
    }

    #[test]
    fn clean_title_returns_trimmed_input_when_nothing_is_left() {
        assert_eq!(clean_title(" (Live) "), "(Live)");
        assert_eq!(clean_title("  Song  "), "Song");
    }

    #[test]
    fn clean_artist_removes_channel_suffixes_and_featured_artists() {
        for (input, want) in [
            ("KeaneVEVO", "Keane"),
            ("Tulus - Topic", "Tulus"),
            ("A feat. B", "A"),
            ("A Feat B", "A"),
            ("A ft. B", "A"),
            ("A featuring B", "A"),
        ] {
            assert_eq!(clean_artist(input), want, "{input}");
        }
    }

    #[test]
    fn clean_artist_keeps_lists_and_lookalike_names() {
        for input in ["Dewa 19, Ari Lasso", "Simon & Garfunkel", "John Featherstone", "VEVO", "米津玄師"] {
            assert_eq!(clean_artist(input), input);
        }
    }

    #[test]
    fn key_folds_latin_accents_and_case() {
        assert_eq!(key("Beyoncé"), key("BEYONCE"));
    }

    #[test]
    fn key_keeps_japanese_voicing_marks() {
        assert_ne!(key("が"), key("か"));
        assert_eq!(key("夜に駆ける"), "夜に駆ける");
        assert_eq!(key("ｶﾞ"), key("ガ"));
    }

    #[test]
    fn key_keeps_korean() {
        assert_eq!(key("봄날"), "봄날");
    }

    #[test]
    fn key_removes_punctuation() {
        assert_eq!(key("Don't Stop Believin'"), "dont stop believin");
    }

    #[test]
    fn key_folds_fullwidth_letters() {
        assert_eq!(key("ＡＢＣ"), "abc");
    }

    #[test]
    fn key_spells_out_ampersand() {
        assert_eq!(key("Simon & Garfunkel"), "simon and garfunkel");
    }

    #[test]
    fn key_collapses_whitespace() {
        assert_eq!(key("  a \t b　c  "), "a b c");
    }

    #[test]
    fn key_of_punctuation_is_empty() {
        assert_eq!(key("!!!"), "");
    }

    #[test]
    fn candidates_splits_youtube_title() {
        let got = candidates("Keane - Somewhere Only We Know (Official Music Video)", "KeaneVEVO");
        assert!(got.contains(&pair("Somewhere Only We Know", "Keane")), "{got:?}");
    }

    #[test]
    fn candidates_puts_split_first_when_left_side_is_the_artist() {
        let got = candidates("Taylor Swift - Love Story", "TaylorSwiftVEVO");
        assert_eq!(got[0], pair("Love Story", "Taylor Swift"));
        assert!(got.contains(&pair("Taylor Swift - Love Story", "TaylorSwift")), "{got:?}");
    }

    #[test]
    fn candidates_splits_title_that_looks_like_a_decoration() {
        let got = candidates("Oasis - Live Forever (Official Video)", "Some Channel");
        assert!(got.contains(&pair("Live Forever", "Oasis")), "{got:?}");
    }

    #[test]
    fn candidates_puts_cleaned_pair_first_for_spotify_title() {
        let got = candidates("Bohemian Rhapsody - Remastered 2011", "Queen");
        assert_eq!(got[0], pair("Bohemian Rhapsody", "Queen"));
        assert!(got.len() <= 3);
    }

    #[test]
    fn candidates_is_one_pair_for_plain_title() {
        assert_eq!(candidates("夜に駆ける", "YOASOBI"), [pair("夜に駆ける", "YOASOBI")]);
    }

    #[test]
    fn candidates_splits_when_the_player_sends_no_artist() {
        let got = candidates("Keane - Somewhere Only We Know", "");
        assert!(got.contains(&pair("Somewhere Only We Know", "Keane")), "{got:?}");
    }
}
