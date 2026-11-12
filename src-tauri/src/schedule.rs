//! Which lyric line is current at a playback position, and when it changes.
//! Lines must be sorted by `start_ms`.

use crate::lrc::Line;

/// Index of the last line with start_ms <= pos. None before the first line or for empty lyrics.
pub fn index_at(lines: &[Line], pos_ms: f64) -> Option<usize> {
    lines
        .partition_point(|l| f64::from(l.start_ms) <= pos_ms)
        .checked_sub(1)
}

/// Start (ms) of the next line after pos, if any.
pub fn next_start_after(lines: &[Line], pos_ms: f64) -> Option<u32> {
    let i = lines.partition_point(|l| f64::from(l.start_ms) <= pos_ms);
    lines.get(i).map(|l| l.start_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(starts: &[u32]) -> Vec<Line> {
        starts
            .iter()
            .map(|&s| Line { start_ms: s, text: format!("l{s}"), words: vec![] })
            .collect()
    }

    #[test]
    fn index_at_before_first_is_none() {
        assert_eq!(index_at(&lines(&[1_000, 2_000]), 999.9), None);
    }

    #[test]
    fn index_at_exact_start() {
        let l = lines(&[1_000, 2_000, 3_000]);
        assert_eq!(index_at(&l, 1_000.0), Some(0));
        assert_eq!(index_at(&l, 2_000.0), Some(1));
    }

    #[test]
    fn index_at_between() {
        assert_eq!(index_at(&lines(&[1_000, 2_000, 3_000]), 2_500.0), Some(1));
    }

    #[test]
    fn index_at_after_last() {
        assert_eq!(index_at(&lines(&[1_000, 2_000]), 99_000.0), Some(1));
    }

    #[test]
    fn index_at_duplicate_timestamps_returns_last() {
        assert_eq!(index_at(&lines(&[1_000, 2_000, 2_000, 2_000, 3_000]), 2_000.0), Some(3));
    }

    #[test]
    fn next_start_after_basic() {
        let l = lines(&[1_000, 2_000, 3_000]);
        assert_eq!(next_start_after(&l, 0.0), Some(1_000));
        assert_eq!(next_start_after(&l, 1_000.0), Some(2_000));
        assert_eq!(next_start_after(&l, 2_500.0), Some(3_000));
    }

    #[test]
    fn next_start_after_skips_duplicates() {
        let l = lines(&[1_000, 2_000, 2_000, 3_000]);
        assert_eq!(next_start_after(&l, 1_500.0), Some(2_000));
        assert_eq!(next_start_after(&l, 2_000.0), Some(3_000));
    }

    #[test]
    fn next_start_after_last_is_none() {
        let l = lines(&[1_000, 2_000]);
        assert_eq!(next_start_after(&l, 2_000.0), None);
        assert_eq!(next_start_after(&l, 9_000.0), None);
    }

    #[test]
    fn empty_lines() {
        assert_eq!(index_at(&[], 5_000.0), None);
        assert_eq!(next_start_after(&[], 5_000.0), None);
    }
}
