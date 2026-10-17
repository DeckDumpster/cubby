//! The budget engine: what a play costs, what is left, and whether a play may start.
//! Pure functions of the budget row, the window's usage and the clock.

/// A grant's progress as the heartbeats have accumulated it. `position_s` is the position at the
/// last beat (0 at grant), never a value the client claims as watched time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub last_beat_at: i64,
    pub position_s: f64,
    pub watched_s: f64,
}

/// Forward movement beyond wall-clock elapsed plus this much is a scrub, not watching.
pub const SLACK_S: f64 = 2.0;
/// A grant with no beat for this long is closed as `timeout`.
pub const BEAT_TIMEOUT_S: i64 = 60;
/// Grace given to the page to stop after a mid-play cutoff, unless the window closed.
pub const CUTOFF_GRACE_S: u32 = 60;

/// The budget row. `None` limits are unlimited; an empty `daily_windows` means any time of day.
#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    pub video_equivalents: Option<f64>,
    pub window_s: i64,
    pub minutes_cap: Option<f64>,
    pub cutoff_mid_play: bool,
    /// `(start, end)` in minutes after local midnight, `start < end`.
    pub daily_windows: Vec<(u32, u32)>,
}

/// Totals over the grants inside the budget window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub charge: f64,
    pub watched_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remaining {
    pub videos: Option<f64>,
    pub minutes: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    OutOfVideos,
    OutOfMinutes,
    /// Outside every daily window; `next_start` is the unix time the next one opens.
    NotNow {
        next_start: i64,
    },
}

impl Refusal {
    pub fn reason(&self) -> &'static str {
        match self {
            Refusal::OutOfVideos => "out-of-videos",
            Refusal::OutOfMinutes => "out-of-minutes",
            Refusal::NotNow { .. } => "not-now",
        }
    }
}

/// Folds one heartbeat into `p`. Forward movement is credited only up to wall-clock elapsed, and
/// only when it is no more than elapsed plus slack; a forward scrub, a seek back and a pause earn
/// nothing, but the position follows the player so the next beat measures from where it is.
pub fn apply_beat(p: Progress, now: i64, position_s: f64) -> Progress {
    let wall = (now - p.last_beat_at).max(0) as f64;
    let moved = position_s - p.position_s;
    let credit = if moved > 0.0 && moved <= wall + SLACK_S {
        moved.min(wall)
    } else {
        0.0
    };
    Progress {
        last_beat_at: now,
        position_s,
        watched_s: p.watched_s + credit,
    }
}

/// What a grant costs: nothing under `min(30 s, 10%)` of the video, else the fraction watched.
pub fn charge(watched_s: f64, duration_s: f64) -> f64 {
    if duration_s <= 0.0 || watched_s < 30.0_f64.min(0.1 * duration_s) {
        return 0.0;
    }
    (watched_s / duration_s).min(1.0)
}

pub fn remaining(b: &Budget, u: Usage) -> Remaining {
    Remaining {
        videos: b.video_equivalents.map(|v| v - u.charge),
        minutes: b.minutes_cap.map(|m| m - u.watched_s / 60.0),
    }
}

/// Whether a play may start at `now` for a child in time zone `tz` (an IANA name).
pub fn may_start(b: &Budget, u: Usage, tz: &str, now: i64) -> Result<(), Refusal> {
    if let Some(next_start) = next_window_start(&b.daily_windows, tz, now) {
        return Err(Refusal::NotNow { next_start });
    }
    let r = remaining(b, u);
    if r.videos.is_some_and(|v| v <= 0.0) {
        return Err(Refusal::OutOfVideos);
    }
    if r.minutes.is_some_and(|m| m <= 0.0) {
        return Err(Refusal::OutOfMinutes);
    }
    Ok(())
}

/// The mid-play verdict for a beat: `Some(grace_s)` when the page must stop, `None` to play on.
/// Always `None` unless `cutoff_mid_play` is on; the grace is 0 once the time window has closed.
pub fn cutoff(b: &Budget, u: Usage, tz: &str, now: i64) -> Option<u32> {
    if !b.cutoff_mid_play {
        return None;
    }
    if next_window_start(&b.daily_windows, tz, now).is_some() {
        return Some(0);
    }
    let r = remaining(b, u);
    let spent = r.videos.is_some_and(|v| v <= 0.0) || r.minutes.is_some_and(|m| m <= 0.0);
    spent.then_some(CUTOFF_GRACE_S)
}

/// `None` when `now` is inside a window (or there are none); else when the next one opens.
pub fn next_window_start(windows: &[(u32, u32)], tz: &str, now: i64) -> Option<i64> {
    use jiff::{Span, Timestamp};
    if windows.is_empty() {
        return None;
    }
    // A zone or instant we cannot read is never "now": fail closed, ask again in an hour.
    let closed = Some(now + 3600);
    let Ok(ts) = Timestamp::from_second(now) else {
        return closed;
    };
    let Ok(zone) = jiff::tz::TimeZone::get(tz) else {
        return closed;
    };
    let local = ts.to_zoned(zone.clone());
    let secs =
        i64::from(local.hour()) * 3600 + i64::from(local.minute()) * 60 + i64::from(local.second());
    if windows
        .iter()
        .any(|&(s, e)| i64::from(s) * 60 <= secs && secs < i64::from(e) * 60)
    {
        return None;
    }
    let mut next: Option<i64> = None;
    for day in 0..=1 {
        let Ok(date) = local.date().checked_add(Span::new().days(day)) else {
            continue;
        };
        for &(s, _) in windows {
            let Ok(z) = date
                .at((s / 60) as i8, (s % 60) as i8, 0, 0)
                .to_zoned(zone.clone())
            else {
                continue;
            };
            let t = z.timestamp().as_second();
            if t > now && next.is_none_or(|n| t < n) {
                next = Some(t);
            }
        }
    }
    next.or(closed)
}

/// Parses `[["07:00","19:00"]]`-style pairs as the budgets column stores them.
pub fn parse_windows(pairs: &[(String, String)]) -> Option<Vec<(u32, u32)>> {
    fn hm(s: &str) -> Option<u32> {
        let (h, m) = s.split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (h < 24 && m < 60).then_some(h * 60 + m)
    }
    pairs
        .iter()
        .map(|(a, b)| {
            let (a, b) = (hm(a)?, hm(b)?);
            (a < b).then_some((a, b))
        })
        .collect()
}

/// Whether `tz` names a time zone this build knows.
pub fn valid_tz(tz: &str) -> bool {
    jiff::tz::TimeZone::get(tz).is_ok()
}

pub fn timed_out(last_beat_at: i64, now: i64) -> bool {
    now - last_beat_at > BEAT_TIMEOUT_S
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn start(at: i64) -> Progress {
        Progress {
            last_beat_at: at,
            position_s: 0.0,
            watched_s: 0.0,
        }
    }

    fn budget() -> Budget {
        Budget {
            video_equivalents: Some(3.0),
            window_s: 86_400,
            minutes_cap: Some(60.0),
            cutoff_mid_play: false,
            daily_windows: vec![],
        }
    }

    /// 2026-10-14 12:00:00 UTC, a Wednesday.
    const NOON_UTC: i64 = 1_791_979_200;

    #[test]
    fn a_forward_scrub_earns_no_watched_time() {
        let p = apply_beat(start(1000), 1010, 300.0);
        assert_eq!(p.watched_s, 0.0);
        assert_eq!(p.position_s, 300.0);
    }

    #[test]
    fn honest_playing_earns_the_time_it_moved() {
        let p = apply_beat(start(1000), 1010, 10.0);
        assert_eq!(p.watched_s, 10.0);
        let p = apply_beat(p, 1020, 20.0);
        assert_eq!(p.watched_s, 20.0);
    }

    #[test]
    fn movement_within_slack_is_credited_only_up_to_wall_time() {
        let p = apply_beat(start(1000), 1010, 11.5);
        assert_eq!(p.watched_s, 10.0);
    }

    #[test]
    fn seeking_back_and_a_pause_earn_nothing() {
        let p = Progress {
            last_beat_at: 1000,
            position_s: 100.0,
            watched_s: 50.0,
        };
        assert_eq!(apply_beat(p, 1010, 40.0).watched_s, 50.0);
        assert_eq!(apply_beat(p, 1010, 100.0).watched_s, 50.0);
    }

    #[test]
    fn a_scrub_then_quit_at_95_percent_is_not_charged_95_percent() {
        let p = apply_beat(start(1000), 1005, 570.0);
        assert_eq!(charge(p.watched_s, 600.0), 0.0);
    }

    #[test]
    fn a_play_under_ten_percent_or_thirty_seconds_is_free() {
        assert_eq!(charge(29.0, 600.0), 0.0);
        assert_eq!(charge(9.0, 100.0), 0.0);
        assert_eq!(charge(10.0, 100.0), 0.1);
        assert_eq!(charge(30.0, 600.0), 0.05);
    }

    #[test]
    fn watching_95_percent_charges_0_95() {
        assert_eq!(charge(570.0, 600.0), 0.95);
    }

    #[test]
    fn a_charge_never_exceeds_one() {
        assert_eq!(charge(900.0, 600.0), 1.0);
    }

    #[test]
    fn remaining_subtracts_the_usage() {
        let r = remaining(
            &budget(),
            Usage {
                charge: 1.5,
                watched_s: 600.0,
            },
        );
        assert_eq!(r.videos, Some(1.5));
        assert_eq!(r.minutes, Some(50.0));
    }

    #[test]
    fn unlimited_limits_never_refuse() {
        let b = Budget {
            video_equivalents: None,
            minutes_cap: None,
            ..budget()
        };
        let u = Usage {
            charge: 99.0,
            watched_s: 1e6,
        };
        assert_eq!(may_start(&b, u, "UTC", NOON_UTC), Ok(()));
    }

    #[test]
    fn a_spent_budget_refuses_with_the_reason() {
        let u = Usage {
            charge: 3.0,
            watched_s: 0.0,
        };
        assert_eq!(
            may_start(&budget(), u, "UTC", NOON_UTC),
            Err(Refusal::OutOfVideos)
        );
        let u = Usage {
            charge: 0.0,
            watched_s: 3600.0,
        };
        assert_eq!(
            may_start(&budget(), u, "UTC", NOON_UTC),
            Err(Refusal::OutOfMinutes)
        );
    }

    #[test]
    fn the_cutoff_fires_only_when_cutoff_mid_play_is_on() {
        let spent = Usage {
            charge: 3.0,
            watched_s: 0.0,
        };
        assert_eq!(cutoff(&budget(), spent, "UTC", NOON_UTC), None);
        let on = Budget {
            cutoff_mid_play: true,
            ..budget()
        };
        assert_eq!(cutoff(&on, spent, "UTC", NOON_UTC), Some(CUTOFF_GRACE_S));
        assert_eq!(cutoff(&on, Usage::default(), "UTC", NOON_UTC), None);
    }

    #[test]
    fn the_cutoff_has_no_grace_once_the_window_closed() {
        let on = Budget {
            cutoff_mid_play: true,
            daily_windows: vec![(7 * 60, 11 * 60)],
            ..budget()
        };
        assert_eq!(cutoff(&on, Usage::default(), "UTC", NOON_UTC), Some(0));
    }

    #[test]
    fn windows_are_read_in_the_childs_time_zone() {
        let b = Budget {
            daily_windows: vec![(7 * 60, 19 * 60)],
            ..budget()
        };
        // 12:00 UTC is 05:00 in Los Angeles (PDT): before the window opens at 07:00 local.
        let la = may_start(&b, Usage::default(), "America/Los_Angeles", NOON_UTC);
        assert_eq!(
            la,
            Err(Refusal::NotNow {
                next_start: NOON_UTC + 2 * 3600
            })
        );
        assert_eq!(may_start(&b, Usage::default(), "UTC", NOON_UTC), Ok(()));
        // 12:00 UTC is 21:00 in Tokyo: after the window closes, next opens 07:00 the next day.
        let tokyo = may_start(&b, Usage::default(), "Asia/Tokyo", NOON_UTC);
        assert_eq!(
            tokyo,
            Err(Refusal::NotNow {
                next_start: NOON_UTC + 10 * 3600
            })
        );
    }

    #[test]
    fn window_edges_start_inclusive_end_exclusive() {
        let w = [(12 * 60, 13 * 60)];
        assert_eq!(next_window_start(&w, "UTC", NOON_UTC), None);
        assert_eq!(next_window_start(&w, "UTC", NOON_UTC + 3599), None);
        assert_eq!(
            next_window_start(&w, "UTC", NOON_UTC + 3600),
            Some(NOON_UTC + 24 * 3600)
        );
    }

    #[test]
    fn a_dst_day_still_finds_the_next_window() {
        // 2026-03-08 is the US spring-forward day; 07:00 PDT is 14:00 UTC.
        let w = [(7 * 60, 8 * 60)];
        let at = 1_772_949_600; // 2026-03-08 06:00 UTC = 2026-03-07 22:00 PST
        assert_eq!(
            next_window_start(&w, "America/Los_Angeles", at),
            Some(1_772_978_400)
        );
    }

    #[test]
    fn an_unknown_time_zone_is_never_now() {
        let w = [(0, 24 * 60)];
        assert!(next_window_start(&w, "Mars/Olympus", NOON_UTC).is_some());
    }

    #[test]
    fn windows_parse_and_reject_garbage() {
        let ok = parse_windows(&[("07:00".into(), "19:30".into())]).unwrap();
        assert_eq!(ok, vec![(420, 1170)]);
        assert!(parse_windows(&[("25:00".into(), "26:00".into())]).is_none());
        assert!(parse_windows(&[("19:00".into(), "07:00".into())]).is_none());
        assert!(parse_windows(&[("7am".into(), "9pm".into())]).is_none());
    }

    #[test]
    fn a_beat_is_missed_after_sixty_seconds() {
        assert!(!timed_out(1000, 1060));
        assert!(timed_out(1000, 1061));
    }

    proptest! {
        #[test]
        fn no_beat_sequence_earns_more_than_wall_time(
            steps in proptest::collection::vec((1i64..30, -50.0f64..400.0), 1..40)
        ) {
            let mut p = start(0);
            let mut now = 0;
            for (dt, pos) in steps {
                now += dt;
                p = apply_beat(p, now, pos);
                prop_assert!(p.watched_s <= now as f64 + 1e-9);
                prop_assert!(p.watched_s >= 0.0);
            }
        }

        #[test]
        fn a_forward_scrub_of_any_size_beyond_slack_earns_nothing(
            dt in 1i64..60, extra in 2.01f64..5000.0
        ) {
            let p = apply_beat(start(0), dt, dt as f64 + extra);
            prop_assert_eq!(p.watched_s, 0.0);
        }

        #[test]
        fn the_charge_is_between_zero_and_one(w in 0.0f64..1e5, d in 1.0f64..1e5) {
            let c = charge(w, d);
            prop_assert!((0.0..=1.0).contains(&c));
        }
    }
}
