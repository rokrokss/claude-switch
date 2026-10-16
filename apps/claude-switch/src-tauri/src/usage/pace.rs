//! How a meter is faring: the use so far, projected over its window, says
//! whether it lasts until the reset. The rules are OpenUsage's pacing
//! (MIT; `Support/Pace.swift` and `WidgetData.meterState`), kept exact so the
//! panel's colours, its "~N% spare" notes and the notifications agree.
//!
//! Every band keys off the share used, so the state never changes with
//! whether the panel shows usage as used or as left.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::metrics::{MetricValue, Unit};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MeterState {
    /// What's left rounds to nothing at the precision shown.
    Spent,
    /// Projected to run out before the reset, or to land right on the limit.
    /// `runs_out_at` is set only when the run-out comes before the reset.
    #[serde(rename_all = "camelCase")]
    RunningOut {
        runs_out_at: Option<String>,
        projected_fraction: f64,
    },
    /// Projected to land in the last tenth of the limit, with at least 1% to spare.
    #[serde(rename_all = "camelCase")]
    CloseToLimit {
        spare_percent: i64,
        projected_fraction: f64,
    },
    /// Projected to finish with a tenth of the limit or more to spare.
    #[serde(rename_all = "camelCase")]
    Healthy { projected_fraction: f64 },
    /// Nothing trustworthy to project: coloured by the share used alone.
    #[serde(rename_all = "camelCase")]
    Level { severity: Severity },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Projected to finish with at least 10% to spare.
    Ahead,
    /// Projected to land in the last 10%.
    OnTrack,
    /// Projected past the limit, or already at it.
    Behind,
}

struct Projection {
    status: Status,
    /// Use at the reset at the current rate, in the meter's unit.
    projected: f64,
}

/// Below this share used, a projection is distrusted: a coarse whole-percent
/// meter can read 1% minutes into a window and project a bogus blow-out.
const NEAR_EMPTY: f64 = 0.05;

/// The state of a meter at `now`; `None` for anything that isn't a meter.
pub fn meter_state(value: &MetricValue, now: DateTime<Utc>) -> Option<MeterState> {
    let MetricValue::Meter {
        used,
        limit,
        unit,
        resets_at,
        period_secs,
        not_started,
    } = value
    else {
        return None;
    };
    let (used, limit) = (*used, *limit);
    if limit <= 0.0 {
        return Some(MeterState::Level {
            severity: Severity::Normal,
        });
    }
    if rounded_as_shown(limit - used, unit) <= 0.0 {
        return Some(MeterState::Spent);
    }
    // A session that hasn't started has nothing to pace.
    if *not_started {
        return Some(level(used, limit));
    }
    let window = resets_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc))
        .zip(period_secs.filter(|secs| *secs > 0));
    let Some((resets_at, period_secs)) = window else {
        return Some(level(used, limit));
    };
    let period = period_secs as f64;
    let Some(projection) = project(used, limit, resets_at, period, now) else {
        return Some(level(used, limit));
    };
    let projected_fraction = projection.projected / limit;
    let state = match projection.status {
        Status::Ahead => MeterState::Healthy { projected_fraction },
        _ if used / limit < NEAR_EMPTY => level(used, limit),
        Status::OnTrack => {
            let spare_percent = ((1.0 - projected_fraction) * 100.0).round() as i64;
            if spare_percent < 1 {
                MeterState::RunningOut {
                    runs_out_at: None,
                    projected_fraction,
                }
            } else {
                MeterState::CloseToLimit {
                    spare_percent,
                    projected_fraction,
                }
            }
        }
        Status::Behind => MeterState::RunningOut {
            runs_out_at: runs_out_in(used, limit, resets_at, period, &projection, now)
                .map(|secs| (now + Duration::milliseconds((secs * 1000.0) as i64)).to_rfc3339()),
            projected_fraction,
        },
    };
    Some(state)
}

/// Projects `used` to the end of the window, or `None` when there's nothing
/// to go on: no use yet, the window already reset, or too early in it for a
/// stable rate.
fn project(
    used: f64,
    limit: f64,
    resets_at: DateTime<Utc>,
    period: f64,
    now: DateTime<Utc>,
) -> Option<Projection> {
    if limit <= 0.0 || period <= 0.0 || used <= 0.0 {
        return None;
    }
    let elapsed = elapsed_secs(resets_at, period, now);
    if elapsed < minimum_elapsed(period) || now >= resets_at {
        return None;
    }
    let projected = used / elapsed * period;
    let status = if used >= limit || projected > limit {
        Status::Behind
    } else if projected <= limit * 0.9 {
        Status::Ahead
    } else {
        Status::OnTrack
    };
    Some(Projection { status, projected })
}

/// Seconds until the limit is reached at the current rate, if that comes
/// before the reset.
fn runs_out_in(
    used: f64,
    limit: f64,
    resets_at: DateTime<Utc>,
    period: f64,
    projection: &Projection,
    now: DateTime<Utc>,
) -> Option<f64> {
    let rate = projection.projected / period;
    if rate <= 0.0 {
        return None;
    }
    let eta = (limit - used) / rate;
    let until_reset = (resets_at - now).num_milliseconds() as f64 / 1000.0;
    (eta > 0.0 && eta < until_reset).then_some(eta)
}

fn elapsed_secs(resets_at: DateTime<Utc>, period: f64, now: DateTime<Utc>) -> f64 {
    let start = resets_at - Duration::milliseconds((period * 1000.0) as i64);
    (now - start).num_milliseconds() as f64 / 1000.0
}

/// Too early in a window for a stable rate: a minute, or 1% of the window.
fn minimum_elapsed(period: f64) -> f64 {
    (period * 0.01).max(60.0)
}

/// Colour by the share used: amber from 80%, red from 90%.
fn level(used: f64, limit: f64) -> MeterState {
    let percent = ((used / limit).clamp(0.0, 1.0) * 100.0).round();
    let severity = if percent >= 90.0 {
        Severity::Critical
    } else if percent >= 80.0 {
        Severity::Warning
    } else {
        Severity::Normal
    };
    MeterState::Level { severity }
}

/// `value` rounded the way the panel prints it: whole percent, a tenth of a
/// count, a cent.
fn rounded_as_shown(value: f64, unit: &Unit) -> f64 {
    match unit {
        Unit::Percent => value.round(),
        Unit::Count => (value * 10.0).round() / 10.0,
        Unit::Money { .. } => (value * 100.0).round() / 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * 60;
    const SESSION: i64 = 5 * HOUR;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// A percent meter `used`% through a window of `period` seconds that
    /// resets `remaining` seconds from now.
    fn meter(used: f64, period: Option<i64>, remaining: Option<i64>) -> MetricValue {
        MetricValue::Meter {
            used,
            limit: 100.0,
            unit: Unit::Percent,
            resets_at: remaining.map(|secs| (now() + Duration::seconds(secs)).to_rfc3339()),
            period_secs: period,
            not_started: false,
        }
    }

    fn state(value: &MetricValue) -> MeterState {
        meter_state(value, now()).unwrap()
    }

    fn level(severity: Severity) -> MeterState {
        MeterState::Level { severity }
    }

    #[test]
    fn halfway_through_at_a_fifth_used_is_healthy() {
        // 20% used with half the window gone projects to 40%.
        let MeterState::Healthy { projected_fraction } =
            state(&meter(20.0, Some(SESSION), Some(SESSION / 2)))
        else {
            panic!("expected healthy");
        };
        assert!((projected_fraction - 0.4).abs() < 1e-9);
    }

    #[test]
    fn landing_in_the_last_tenth_is_close_to_the_limit() {
        // 47% at halfway projects to 94%: 6% to spare.
        assert_eq!(
            state(&meter(47.0, Some(SESSION), Some(SESSION / 2))),
            MeterState::CloseToLimit {
                spare_percent: 6,
                projected_fraction: 0.94
            }
        );
    }

    #[test]
    fn a_cushion_that_rounds_to_nothing_is_running_out_without_a_time() {
        // 49.8% at halfway projects to 99.6%: the spare rounds to 0%.
        let MeterState::RunningOut { runs_out_at, .. } =
            state(&meter(49.8, Some(SESSION), Some(SESSION / 2)))
        else {
            panic!("expected running out");
        };
        assert_eq!(runs_out_at, None);
    }

    #[test]
    fn projected_past_the_limit_runs_out_before_the_reset() {
        // 60% at halfway projects to 120%; the remaining 40% goes in a third
        // of the window, before the reset half a window away.
        let MeterState::RunningOut {
            runs_out_at,
            projected_fraction,
        } = state(&meter(60.0, Some(SESSION), Some(SESSION / 2)))
        else {
            panic!("expected running out");
        };
        assert!((projected_fraction - 1.2).abs() < 1e-9);
        assert_eq!(
            runs_out_at.as_deref(),
            Some(
                (now() + Duration::seconds(SESSION / 3))
                    .to_rfc3339()
                    .as_str()
            )
        );
    }

    #[test]
    fn a_projection_from_almost_nothing_used_is_not_trusted() {
        // 4% a minute and a half in projects far past the limit.
        assert_eq!(
            state(&meter(4.0, Some(SESSION), Some(SESSION - 300))),
            level(Severity::Normal)
        );
    }

    #[test]
    fn too_early_in_the_window_falls_back_to_the_level() {
        // Under 1% of a week in.
        assert_eq!(
            state(&meter(85.0, Some(7 * 24 * HOUR), Some(7 * 24 * HOUR - 600))),
            level(Severity::Warning)
        );
    }

    #[test]
    fn a_session_that_has_not_started_is_never_paced() {
        let value = MetricValue::Meter {
            used: 0.0,
            limit: 100.0,
            unit: Unit::Percent,
            resets_at: None,
            period_secs: Some(SESSION),
            not_started: true,
        };
        assert_eq!(state(&value), level(Severity::Normal));
    }

    #[test]
    fn without_a_window_the_share_used_sets_the_colour() {
        assert_eq!(state(&meter(79.0, None, None)), level(Severity::Normal));
        assert_eq!(state(&meter(80.0, None, None)), level(Severity::Warning));
        assert_eq!(state(&meter(90.0, None, None)), level(Severity::Critical));
    }

    #[test]
    fn a_window_past_its_reset_is_not_paced() {
        assert_eq!(
            state(&meter(95.0, Some(SESSION), Some(-60))),
            level(Severity::Critical)
        );
    }

    #[test]
    fn nothing_left_at_the_shown_precision_is_spent() {
        assert_eq!(
            state(&meter(99.6, Some(SESSION), Some(HOUR))),
            MeterState::Spent
        );
        assert_eq!(state(&meter(100.0, None, None)), MeterState::Spent);
        let money = MetricValue::Meter {
            used: 49.996,
            limit: 50.0,
            unit: Unit::Money {
                currency: "USD".into(),
            },
            resets_at: None,
            period_secs: None,
            not_started: false,
        };
        assert_eq!(state(&money), MeterState::Spent);
    }

    #[test]
    fn readings_without_a_limit_have_no_state() {
        let values = MetricValue::Values {
            values: Vec::new(),
            expires_at: Vec::new(),
        };
        assert_eq!(meter_state(&values, now()), None);
    }
}
