#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Word {
    pub start_ms: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub start_ms: u32,
    pub text: String,
    pub words: Vec<Word>,
}

pub const INTRO_NOTE_MS: u32 = 5_000;
pub const LONG_GAP_MS: u32 = 20_000;
pub const NOTE_AFTER_MS: u32 = 10_000;

/// Longer lines are skipped. The input is untrusted, and each timestamp on a line copies the line's
/// text, so an unbounded line could ask for gigabytes.
const MAX_LINE_BYTES: usize = 4096;

/// Sorted by start_ms. `words` is empty when the line has no word timing. Empty `text` means a break (shown as ♪).
pub fn parse(lrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut offset = 0i32;
    for raw in lrc.trim_start_matches('\u{feff}').lines() {
        if raw.len() > MAX_LINE_BYTES {
            continue;
        }
        let mut rest = raw.trim();
        let mut starts = Vec::new();
        while let Some((tag, after)) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            match timestamp(tag) {
                Some(ms) => starts.push(ms),
                None => {
                    // A bracket after a timestamp is lyric text ("[Chorus]"), not a tag.
                    let offset_tag = tag
                        .split_once(':')
                        .filter(|(key, _)| starts.is_empty() && key.trim().eq_ignore_ascii_case("offset"));
                    if let Some((_, value)) = offset_tag {
                        offset = value.trim().parse().unwrap_or(offset);
                    }
                    break;
                }
            }
            rest = after.trim_start();
        }
        let Some(&first) = starts.first() else { continue };
        let Some((text, mut words)) = split_words(rest, first) else { continue };
        if starts.len() > 1 {
            // Word times are absolute, so they fit only one of the repeats.
            words.clear();
        }
        for start_ms in starts {
            lines.push(Line { start_ms, text: text.clone(), words: words.clone() });
        }
    }
    // A positive offset makes the lyrics earlier.
    let shift = |ms: u32| (i64::from(ms) - i64::from(offset)).clamp(0, i64::from(u32::MAX)) as u32;
    for line in &mut lines {
        line.start_ms = shift(line.start_ms);
        for word in &mut line.words {
            word.start_ms = shift(word.start_ms);
        }
    }
    lines.sort_by_key(|l| l.start_ms); // stable: equal timestamps keep file order
    lines
}

/// Inserts break lines per spec B5.
pub fn fill_gaps(lines: Vec<Line>) -> Vec<Line> {
    let note = |start_ms| Line { start_ms, text: String::new(), words: Vec::new() };
    let mut out = Vec::with_capacity(lines.len() + 1);
    if lines.first().is_some_and(|l| l.start_ms > INTRO_NOTE_MS) {
        out.push(note(0));
    }
    let mut iter = lines.into_iter().peekable();
    while let Some(line) = iter.next() {
        let long_gap = !line.text.is_empty()
            && iter.peek().is_some_and(|next| next.start_ms.saturating_sub(line.start_ms) > LONG_GAP_MS);
        let start_ms = line.start_ms;
        out.push(line);
        if long_gap {
            out.push(note(start_ms + NOTE_AFTER_MS));
        }
    }
    out
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx`, or `:` before the fraction. None when it is not a
/// timestamp or does not fit in u32 milliseconds.
fn timestamp(tag: &str) -> Option<u32> {
    let (min, rest) = tag.trim().split_once(':')?;
    let (sec, frac) = rest.split_once(['.', ':']).unwrap_or((rest, "000"));
    let frac_unit = match frac.len() {
        1 => 100,
        2 => 10,
        3 => 1,
        _ => return None,
    };
    let ms = digits(min)?
        .checked_mul(60_000)?
        .checked_add(digits(sec)?.checked_mul(1_000)?)?
        .checked_add(digits(frac)? * frac_unit)?;
    u32::try_from(ms).ok()
}

/// Digits only: `str::parse` alone would also accept a leading `+`.
fn digits(s: &str) -> Option<u64> {
    s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok())?
}

/// The next `<...>` in `s` that is shaped like a timestamp (digits, `:` and `.`), valid or not:
/// where it opens, where the text after it begins, and what is inside it.
fn next_word_tag(s: &str) -> Option<(usize, usize, &str)> {
    let mut from = 0;
    while let Some(open) = s[from..].find('<').map(|i| i + from) {
        // Stopping at the next `<` as well keeps the scan linear on input full of stray `<`.
        let end = s[open + 1..].find(['<', '>'])? + open + 1;
        let tag = s[open + 1..end].trim();
        let shaped = tag.starts_with(|c: char| c.is_ascii_digit())
            && tag.contains(':')
            && tag.bytes().all(|b| b.is_ascii_digit() || b == b':' || b == b'.');
        if shaped && s[end..].starts_with('>') {
            return Some((open, end + 1, tag));
        }
        from = end;
    }
    None
}

/// Splits the text after the line timestamps into the display text and its timed words.
/// None when a word tag is not a valid timestamp: the line is skipped, not shown with the tag in it.
fn split_words(body: &str, line_start: u32) -> Option<(String, Vec<Word>)> {
    let mut words: Vec<Word> = Vec::new();
    let mut push = |start_ms: u32, text: &str| {
        if !text.trim().is_empty() {
            words.push(Word { start_ms, text: text.to_string() });
        } else if let Some(last) = words.last_mut() {
            last.text.push_str(text); // spaces between two tags stay with the word before them
        }
    };
    let (mut rest, mut start) = (body, line_start);
    while let Some((open, after, tag)) = next_word_tag(rest) {
        push(start, &rest[..open]);
        (rest, start) = (&rest[after..], timestamp(tag)?);
    }
    if rest.len() == body.len() {
        return Some((body.to_string(), Vec::new())); // no word tags
    }
    push(start, rest);
    let text: String = words.iter().map(|w| w.text.as_str()).collect();
    Some((text.trim().to_string(), words))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(lines: &[Line]) -> Vec<u32> {
        lines.iter().map(|l| l.start_ms).collect()
    }

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    fn line(start_ms: u32, text: &str) -> Line {
        Line { start_ms, text: text.to_string(), words: Vec::new() }
    }

    #[test]
    fn parses_basic_line() {
        let lines = parse("[00:12.34]Hello");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_ms, 12_340);
        assert_eq!(lines[0].text, "Hello");
        assert!(lines[0].words.is_empty());
    }

    #[test]
    fn parses_fraction_widths() {
        let lines = parse("[01:02]a\n[01:02.5]b\n[01:02.50]c\n[01:02.500]d\n[01:02:50]e");
        assert_eq!(starts(&lines), [62_000, 62_500, 62_500, 62_500, 62_500]);
        assert_eq!(texts(&lines), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn minutes_may_have_more_than_two_digits() {
        assert_eq!(starts(&parse("[100:00.00]late")), [6_000_000]);
    }

    #[test]
    fn multiple_timestamps_make_multiple_lines() {
        let lines = parse("[00:10.00][00:20.00]La");
        assert_eq!(starts(&lines), [10_000, 20_000]);
        assert_eq!(texts(&lines), ["La", "La"]);
    }

    #[test]
    fn multiple_timestamps_drop_word_timing() {
        let lines = parse("[00:10.00][00:50.00]<00:10.00>La <00:10.50>la");
        assert_eq!(lines, [line(10_000, "La la"), line(50_000, "La la")]);
    }

    #[test]
    fn sorts_out_of_order_and_keeps_file_order_for_ties() {
        let lines = parse("[00:30.00]c\n[00:10.00]a1\n[00:20.00]b\n[00:10.00]a2\n[00:10.00]a3");
        assert_eq!(starts(&lines), [10_000, 10_000, 10_000, 20_000, 30_000]);
        assert_eq!(texts(&lines), ["a1", "a2", "a3", "b", "c"]);
    }

    #[test]
    fn applies_offset_tag() {
        assert_eq!(starts(&parse("[offset:+500]\n[00:10.00]A")), [9_500]);
        assert_eq!(starts(&parse("[offset:-500]\n[00:10.00]A")), [10_500]);
        assert_eq!(starts(&parse("[offset:+20000]\n[00:10.00]A")), [0]);
    }

    #[test]
    fn offset_shifts_word_starts_too() {
        let lines = parse("[offset:+500]\n[00:10.00]<00:10.00>Never <00:10.50>gonna");
        let word_starts: Vec<u32> = lines[0].words.iter().map(|w| w.start_ms).collect();
        assert_eq!(lines[0].start_ms, 9_500);
        assert_eq!(word_starts, [9_500, 10_000]);
    }

    #[test]
    fn ignores_metadata_and_untimed_lines() {
        assert!(parse("[ar:X]\n[ti:Y]\n[length:03:00]\nplain text\n").is_empty());
    }

    #[test]
    fn empty_text_is_a_break() {
        assert_eq!(parse("[00:30.00]"), [line(30_000, "")]);
        assert_eq!(parse("[00:30.00]   \t"), [line(30_000, "")]);
    }

    #[test]
    fn text_is_trimmed() {
        assert_eq!(texts(&parse("[00:10.00]  Hello there  ")), ["Hello there"]);
    }

    #[test]
    fn bracketed_text_after_a_timestamp_is_kept() {
        assert_eq!(texts(&parse("[00:10.00][Chorus]")), ["[Chorus]"]);
    }

    #[test]
    fn parses_enhanced_words() {
        let lines = parse("[00:10.00]<00:10.00>Never <00:10.50>gonna <00:11.00>give<00:11.40>");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Never gonna give");
        assert_eq!(
            lines[0].words,
            [
                Word { start_ms: 10_000, text: "Never ".to_string() },
                Word { start_ms: 10_500, text: "gonna ".to_string() },
                Word { start_ms: 11_000, text: "give".to_string() },
            ]
        );
    }

    #[test]
    fn enhanced_line_without_leading_word_tag() {
        let lines = parse("[00:10.00]Never <00:10.50>gonna");
        assert_eq!(lines[0].text, "Never gonna");
        assert_eq!(
            lines[0].words,
            [
                Word { start_ms: 10_000, text: "Never ".to_string() },
                Word { start_ms: 10_500, text: "gonna".to_string() },
            ]
        );
    }

    #[test]
    fn joined_words_reproduce_text() {
        let lines = parse("[00:10.00] <00:10.00> Never <00:10.50> <00:10.60> gonna <00:11.00>");
        let joined: String = lines[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(lines[0].text, joined.trim());
        assert_eq!(lines[0].words.len(), 2);
        assert!(lines[0].words.iter().all(|w| !w.text.trim().is_empty()));
    }

    #[test]
    fn angle_brackets_that_are_not_word_tags_stay_in_text() {
        let lines = parse("[00:10.00]I <3 you > me");
        assert_eq!(lines[0].text, "I <3 you > me");
        assert!(lines[0].words.is_empty());
    }

    #[test]
    fn enhanced_line_with_only_tags_is_a_break() {
        assert_eq!(parse("[00:10.00]<00:10.00> <00:11.00>"), [line(10_000, "")]);
    }

    #[test]
    fn tolerates_bom_crlf_and_garbage() {
        let lines = parse("\u{feff}[00:01.00]one\r\n[00:02.00]two\r\n");
        assert_eq!(lines, [line(1_000, "one"), line(2_000, "two")]);

        assert!(parse("[xx:yy]bad").is_empty());
        assert!(parse("[00:99999999999.00]big").is_empty());
        assert!(parse("[00:10.00").is_empty());
        assert!(parse("[99999999999:00.00]overflow").is_empty());
        assert!(parse("[+1:+2.+3]signs").is_empty());
        assert!(parse("[00:10.0000]four fraction digits").is_empty());
        assert!(parse("[]\n[:]\n[00:]\n<00:10.00>\n[offset:abc]\n[\u{e9}\u{e9}:\u{e9}\u{e9}]").is_empty());
        assert_eq!(texts(&parse("[00:10.00]caf\u{e9} <\u{e9}> <00:1")), ["caf\u{e9} <\u{e9}> <00:1"]);
    }

    #[test]
    fn overlong_line_is_skipped() {
        let fits = format!("[00:02.00]{}", "a".repeat(4096 - 10));
        let too_long = format!("[00:03.00]{}", "b".repeat(4096 - 9));
        assert_eq!((fits.len(), too_long.len()), (4096, 4097));

        let lines = parse(&format!("[00:01.00]before\n{too_long}\n{fits}\r\n[00:04.00]after"));
        assert_eq!(starts(&lines), [1_000, 2_000, 4_000]);
        assert_eq!(lines[0].text, "before");
        assert_eq!(lines[1].text.len(), 4096 - 10);
        assert_eq!(lines[2].text, "after");

        // Many repeated timestamps with a long text must not multiply into a huge allocation.
        let bomb = format!("{}{}", "[00:01]".repeat(10_000), "x".repeat(100_000));
        assert_eq!(texts(&parse(&format!("{bomb}\n[00:05.00]ok"))), ["ok"]);
    }

    #[test]
    fn offset_tag_ignores_case_and_spaces() {
        assert_eq!(starts(&parse("[Offset:500]\n[00:10.00]A")), [9_500]);
        assert_eq!(starts(&parse("[ offset: 500 ]\n[00:10.00]A")), [9_500]);
        assert_eq!(starts(&parse("[OFFSET : -500]\n[00:10.00]A")), [10_500]);
    }

    #[test]
    fn malformed_word_tag_skips_the_line() {
        let lines = parse(
            "[00:10.00]<00:10.0000>Never <00:10.50>gonna\n\
             [00:20.00]give <99999999999:00.00>you\n\
             [00:30.00]up <1:2:3:4>\n\
             [00:40.00]ok",
        );
        assert_eq!(lines, [line(40_000, "ok")]);
    }

    #[test]
    fn stray_angle_bracket_before_a_word_tag_is_text() {
        let lines = parse("[00:10.00]a <b <00:10.50>c > d");
        assert_eq!(lines[0].text, "a <b c > d");
        assert_eq!(
            lines[0].words,
            [
                Word { start_ms: 10_000, text: "a <b ".to_string() },
                Word { start_ms: 10_500, text: "c > d".to_string() },
            ]
        );
    }

    #[test]
    fn fill_gaps_adds_intro_break() {
        let filled = fill_gaps(vec![line(6_000, "A")]);
        assert_eq!(filled, [line(0, ""), line(6_000, "A")]);

        let unchanged = fill_gaps(vec![line(5_000, "A")]);
        assert_eq!(unchanged, [line(5_000, "A")]);
    }

    #[test]
    fn fill_gaps_adds_long_gap_break() {
        let filled = fill_gaps(vec![line(1_000, "A"), line(10_000, "B"), line(40_000, "C")]);
        assert_eq!(
            filled,
            [line(1_000, "A"), line(10_000, "B"), line(20_000, ""), line(40_000, "C")]
        );

        let short = vec![line(1_000, "A"), line(10_000, "B"), line(30_000, "C")];
        assert_eq!(fill_gaps(short.clone()), short);

        let after_break = vec![line(1_000, "A"), line(10_000, ""), line(40_000, "C")];
        assert_eq!(fill_gaps(after_break.clone()), after_break);
    }

    #[test]
    fn fill_gaps_empty_input() {
        assert!(fill_gaps(Vec::new()).is_empty());
    }
}
