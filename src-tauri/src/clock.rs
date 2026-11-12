//! Interpolated playback clock. All times are f64 milliseconds; "mono" is ms
//! since an arbitrary process epoch. No clock is read in here: time is passed in.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    pub position_ms: f64,
    pub last_updated_wall_ms: Option<i64>,
    pub rate: Option<f64>,
    pub playing: bool,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sync {
    First,
    Seek { jump_ms: f64 },
    Drift { error_ms: f64 },
}

pub const SEEK_THRESHOLD_MS: f64 = 1_500.0;

#[derive(Debug, Clone, Default)]
pub struct Clock {
    anchor_pos: f64,
    anchor_mono: f64,
    rate: f64,
    playing: bool,
    duration: f64,
    set: bool,
}

impl Clock {
    /// `now_wall_ms` is Unix epoch ms read at the same instant as `now_mono_ms`.
    pub fn sync(&mut self, r: &Report, now_mono_ms: f64, now_wall_ms: i64) -> Sync {
        let staleness = r
            .last_updated_wall_ms
            .map_or(0.0, |t| (now_wall_ms - t).max(0) as f64);
        let next = Clock {
            anchor_pos: if r.position_ms.is_finite() { r.position_ms } else { 0.0 },
            // A paused report is its position, however old.
            anchor_mono: if r.playing { now_mono_ms - staleness } else { now_mono_ms },
            rate: r.rate.filter(|x| x.is_finite() && *x > 0.0).unwrap_or(1.0),
            playing: r.playing,
            duration: if r.duration_ms.is_finite() { r.duration_ms.max(0.0) } else { 0.0 },
            set: true,
        };
        let result = if self.set {
            let error_ms = next.position_ms(now_mono_ms) - self.position_ms(now_mono_ms);
            if error_ms.abs() > SEEK_THRESHOLD_MS {
                Sync::Seek { jump_ms: error_ms }
            } else {
                Sync::Drift { error_ms }
            }
        } else {
            Sync::First
        };
        *self = next;
        result
    }

    /// Clamped to 0..=duration when duration > 0. 0.0 before the first sync.
    pub fn position_ms(&self, now_mono_ms: f64) -> f64 {
        if !self.set {
            return 0.0;
        }
        let mut pos = self.anchor_pos;
        if self.playing {
            pos += (now_mono_ms - self.anchor_mono) * self.rate;
        }
        if self.duration > 0.0 {
            pos = pos.min(self.duration);
        }
        pos.max(0.0)
    }

    pub fn is_playing(&self) -> bool {
        self.set && self.playing
    }

    /// Mono time at which playback reaches `pos_ms`. None when paused, unset, or pos is not ahead.
    pub fn mono_at(&self, pos_ms: f64, now_mono_ms: f64) -> Option<f64> {
        if !self.is_playing() {
            return None;
        }
        let ahead = pos_ms - self.position_ms(now_mono_ms);
        (ahead > 0.0).then(|| now_mono_ms + ahead / self.rate)
    }

    pub fn reset(&mut self) {
        *self = Clock::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i64 = 1_700_000_000_000;

    fn rep(pos: f64, playing: bool) -> Report {
        Report {
            position_ms: pos,
            last_updated_wall_ms: Some(W),
            rate: None,
            playing,
            duration_ms: 0.0,
        }
    }

    /// A report stamped at wall time `w` (no staleness when received at `w`).
    fn rep_at(pos: f64, playing: bool, w: i64) -> Report {
        Report { last_updated_wall_ms: Some(w), ..rep(pos, playing) }
    }

    fn started(pos: f64) -> Clock {
        let mut c = Clock::default();
        c.sync(&rep(pos, true), 1_000.0, W);
        c
    }

    #[test]
    fn unset_clock_is_zero_and_not_playing() {
        let c = Clock::default();
        assert_eq!(c.position_ms(5_000.0), 0.0);
        assert!(!c.is_playing());
        assert_eq!(c.mono_at(1_000.0, 0.0), None);
    }

    #[test]
    fn advances_while_playing() {
        let mut c = Clock::default();
        assert_eq!(c.sync(&rep(10_000.0, true), 1_000.0, W), Sync::First);
        assert_eq!(c.position_ms(3_000.0), 12_000.0);
        assert!(c.is_playing());
    }

    #[test]
    fn paused_clock_stands_still() {
        let mut c = Clock::default();
        c.sync(&rep(10_000.0, false), 1_000.0, W);
        assert_eq!(c.position_ms(1_000.0), 10_000.0);
        assert_eq!(c.position_ms(9_000.0), 10_000.0);
        assert!(!c.is_playing());
    }

    #[test]
    fn resume_continues_from_paused_position() {
        let mut c = Clock::default();
        c.sync(&rep(10_000.0, false), 1_000.0, W);
        let s = c.sync(&rep_at(10_000.0, true, W + 4_000), 5_000.0, W + 4_000);
        assert_eq!(s, Sync::Drift { error_ms: 0.0 });
        assert_eq!(c.position_ms(7_000.0), 12_000.0);
    }

    #[test]
    fn stale_report_is_extrapolated() {
        let mut c = Clock::default();
        let r = Report { last_updated_wall_ms: Some(W - 60_000), ..rep(10_000.0, true) };
        c.sync(&r, 1_000.0, W);
        assert_eq!(c.position_ms(1_000.0), 70_000.0);
        assert_eq!(c.position_ms(3_000.0), 72_000.0);
    }

    #[test]
    fn future_last_updated_is_clamped() {
        let mut c = Clock::default();
        let r = Report { last_updated_wall_ms: Some(W + 5_000), ..rep(10_000.0, true) };
        c.sync(&r, 1_000.0, W);
        assert_eq!(c.position_ms(1_000.0), 10_000.0);
        assert_eq!(c.position_ms(2_000.0), 11_000.0);
    }

    #[test]
    fn missing_last_updated_uses_receipt_time() {
        let mut c = Clock::default();
        let r = Report { last_updated_wall_ms: None, ..rep(10_000.0, true) };
        c.sync(&r, 1_000.0, W);
        assert_eq!(c.position_ms(1_000.0), 10_000.0);
        assert_eq!(c.position_ms(2_500.0), 11_500.0);
    }

    #[test]
    fn rate_none_or_zero_or_nan_while_playing_is_one() {
        for rate in [None, Some(0.0), Some(f64::NAN), Some(-1.0), Some(f64::INFINITY)] {
            let mut c = Clock::default();
            c.sync(&Report { rate, ..rep(10_000.0, true) }, 1_000.0, W);
            assert_eq!(c.position_ms(3_000.0), 12_000.0, "rate {rate:?}");
        }
    }

    #[test]
    fn rate_two_doubles_speed() {
        let mut c = Clock::default();
        c.sync(&Report { rate: Some(2.0), ..rep(10_000.0, true) }, 1_000.0, W);
        assert_eq!(c.position_ms(3_000.0), 14_000.0);
    }

    #[test]
    fn rate_half() {
        let mut c = Clock::default();
        c.sync(&Report { rate: Some(0.5), ..rep(10_000.0, true) }, 1_000.0, W);
        assert_eq!(c.position_ms(3_000.0), 11_000.0);
    }

    #[test]
    fn seek_detected_above_threshold() {
        // Clock reads 10_000 at mono 1_000.
        let mut c = started(10_000.0);
        assert_eq!(c.sync(&rep(11_501.0, true), 1_000.0, W), Sync::Seek { jump_ms: 1_501.0 });

        let mut c = started(10_000.0);
        assert_eq!(c.sync(&rep(11_500.0, true), 1_000.0, W), Sync::Drift { error_ms: 1_500.0 });

        let mut c = started(10_000.0);
        assert_eq!(c.sync(&rep(8_000.0, true), 1_000.0, W), Sync::Seek { jump_ms: -2_000.0 });
    }

    #[test]
    fn drift_reports_signed_error() {
        let mut c = started(10_000.0);
        // At mono 4_000 the clock says 13_000; the report says 13_200.
        let s = c.sync(&rep_at(13_200.0, true, W + 3_000), 4_000.0, W + 3_000);
        assert_eq!(s, Sync::Drift { error_ms: 200.0 });
        // At mono 7_400 the clock says 13_200 + 3_400 = 16_600; the report says 16_000.
        let s = c.sync(&rep_at(16_000.0, true, W + 6_400), 7_400.0, W + 6_400);
        assert_eq!(s, Sync::Drift { error_ms: -600.0 });
    }

    #[test]
    fn stale_paused_report_is_not_extrapolated() {
        let mut c = Clock::default();
        let r = Report { last_updated_wall_ms: Some(W - 60_000), ..rep(10_000.0, false) };
        c.sync(&r, 1_000.0, W);
        assert_eq!(c.position_ms(1_000.0), 10_000.0);
        assert_eq!(c.position_ms(50_000.0), 10_000.0);
    }

    #[test]
    fn pausing_without_a_jump_is_drift() {
        let mut c = started(10_000.0);
        let s = c.sync(&rep_at(12_040.0, false, W + 2_000), 3_000.0, W + 2_000);
        assert_eq!(s, Sync::Drift { error_ms: 40.0 });
        assert_eq!(c.position_ms(9_000.0), 12_040.0);
    }

    #[test]
    fn position_clamped_to_duration_and_zero() {
        let mut c = Clock::default();
        c.sync(&Report { duration_ms: 20_000.0, ..rep(19_000.0, true) }, 1_000.0, W);
        assert_eq!(c.position_ms(10_000.0), 20_000.0);

        let mut c = Clock::default();
        c.sync(&rep(-500.0, false), 1_000.0, W);
        assert_eq!(c.position_ms(1_000.0), 0.0);
    }

    #[test]
    fn duration_zero_has_no_upper_clamp() {
        let c = started(10_000.0);
        assert_eq!(c.position_ms(1_000_000.0), 1_009_000.0);
    }

    #[test]
    fn non_finite_report_values_do_not_poison_the_clock() {
        let mut c = Clock::default();
        let r = Report { duration_ms: f64::NAN, ..rep(f64::NAN, true) };
        assert_eq!(c.sync(&r, 1_000.0, W), Sync::First);
        assert_eq!(c.position_ms(3_000.0), 2_000.0);

        for (pos, dur) in [(f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, -5.0)] {
            let mut c = Clock::default();
            c.sync(&Report { duration_ms: dur, ..rep(pos, true) }, 1_000.0, W);
            let p = c.position_ms(3_000.0);
            assert!(p.is_finite());
            assert_eq!(p, 2_000.0);
        }
    }

    #[test]
    fn mono_at_future_position() {
        let c = started(10_000.0);
        assert_eq!(c.mono_at(15_000.0, 3_000.0), Some(6_000.0));
    }

    #[test]
    fn mono_at_none_when_paused_or_past() {
        let c = started(10_000.0);
        assert_eq!(c.mono_at(12_000.0, 3_000.0), None); // equal to current
        assert_eq!(c.mono_at(5_000.0, 3_000.0), None);
        let mut p = Clock::default();
        p.sync(&rep(10_000.0, false), 1_000.0, W);
        assert_eq!(p.mono_at(15_000.0, 1_000.0), None);
    }

    #[test]
    fn mono_at_respects_rate() {
        let mut c = Clock::default();
        c.sync(&Report { rate: Some(2.0), ..rep(10_000.0, true) }, 1_000.0, W);
        assert_eq!(c.mono_at(14_000.0, 1_000.0), Some(3_000.0));
    }

    #[test]
    fn reset_unsets_the_clock() {
        let mut c = started(10_000.0);
        c.reset();
        assert_eq!(c.position_ms(3_000.0), 0.0);
        assert!(!c.is_playing());
        assert_eq!(c.sync(&rep(1.0, true), 3_000.0, W), Sync::First);
    }
}
