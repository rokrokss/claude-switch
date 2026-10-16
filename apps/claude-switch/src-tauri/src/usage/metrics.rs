//! A profile's usage as a list of named metrics: the one shape the menu bar's
//! strip and panel, notifications, the command line and the local API read,
//! whichever app the profile belongs to. Pure: built from a [`ProfileUsage`].
//!
//! Metric ids are what settings refer to (which metrics are starred for the
//! menu bar, where each sits in the panel), so they stay stable.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::pace::{self, MeterState};
use super::{ProfileUsage, QuotaUsage, Window};
use crate::app_kind::AppKind;

/// The ids of the metrics a profile can report.
pub mod id {
    /// The short window: Claude's 5 hours, or whichever Codex window isn't weekly.
    pub const SESSION: &str = "session";
    pub const WEEKLY: &str = "weekly";
    /// Pay-as-you-go spend beyond the plan.
    pub const EXTRA: &str = "extra";
    /// Rate-limit resets the account can use.
    pub const RESETS: &str = "resets";
    // Spend and history from the local logs; reported once those are read.
    #[allow(dead_code)]
    pub const TODAY: &str = "today";
    #[allow(dead_code)]
    pub const YESTERDAY: &str = "yesterday";
    #[allow(dead_code)]
    pub const LAST_30_DAYS: &str = "last30";
    #[allow(dead_code)]
    pub const TREND: &str = "trend";

    /// A weekly window scoped to one model: `weekly.<model>`, e.g. `weekly.fable`.
    pub fn scoped_weekly(model: &str) -> String {
        format!("{WEEKLY}.{}", model.to_lowercase())
    }
}

/// The metrics starred for the menu bar when the user hasn't chosen any.
pub const DEFAULT_STARS: [&str; 2] = [id::SESSION, id::WEEKLY];

const HOUR_SECS: i64 = 60 * 60;
const WEEK_SECS: i64 = 7 * 24 * HOUR_SECS;
/// Claude's session window.
const SESSION_SECS: i64 = 5 * HOUR_SECS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    pub id: String,
    /// What the panel calls it: "Session", "Weekly · Fable".
    pub label: String,
    #[serde(flatten)]
    pub value: MetricValue,
    /// How a meter is faring at the time the snapshot was sent. Worked out
    /// when sent rather than stored: it moves with the clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<MeterState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MetricValue {
    /// Used out of a limit.
    #[serde(rename_all = "camelCase")]
    Meter {
        used: f64,
        limit: f64,
        unit: Unit,
        resets_at: Option<String>,
        /// The window's length, when known: what pacing projects against.
        period_secs: Option<i64>,
        /// The window opens with the first message, and none has been sent.
        not_started: bool,
    },
    /// Readings without a limit: spend, a count of resets.
    #[serde(rename_all = "camelCase")]
    Values {
        values: Vec<Reading>,
        /// When each counted item lapses, soonest first (resets).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        expires_at: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub amount: f64,
    pub unit: Unit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Unit {
    Percent,
    Count,
    /// An amount of `currency` (ISO 4217), in major units.
    Money {
        currency: String,
    },
}

/// The metrics `usage` reports for a profile of `app`, in the panel's
/// default order. A window without data is left out rather than reported
/// as zero.
pub fn from_usage(app: AppKind, usage: &ProfileUsage) -> Vec<Metric> {
    let Some(quota) = &usage.quota else {
        return Vec::new();
    };
    let mut metrics = match app {
        AppKind::Claude => claude_windows(quota),
        AppKind::Codex => codex_windows(quota),
    };
    metrics.extend(extra(quota));
    metrics.extend(resets(quota));
    metrics
}

/// `metrics` with each meter's state worked out for `now`.
pub fn with_states(metrics: &[Metric], now: DateTime<Utc>) -> Vec<Metric> {
    metrics
        .iter()
        .map(|metric| Metric {
            state: pace::meter_state(&metric.value, now),
            ..metric.clone()
        })
        .collect()
}

fn claude_windows(quota: &QuotaUsage) -> Vec<Metric> {
    let mut metrics = Vec::new();
    // Claude's session starts with the first message; until then the
    // endpoint reports no reset time for it.
    metrics.extend(percent_meter(
        id::SESSION,
        "Session",
        quota.primary.as_ref(),
        Some(SESSION_SECS),
        true,
    ));
    metrics.extend(percent_meter(
        id::WEEKLY,
        "Weekly",
        quota.secondary.as_ref(),
        Some(WEEK_SECS),
        false,
    ));
    for window in &quota.scoped_weekly {
        let Some(model) = window.label.as_deref().filter(|model| !model.is_empty()) else {
            continue;
        };
        metrics.extend(percent_meter(
            &id::scoped_weekly(model),
            &format!("Weekly · {model}"),
            Some(window),
            Some(WEEK_SECS),
            false,
        ));
    }
    metrics
}

/// Codex reports its windows by position, and a plan may put either length
/// in either slot, so each is named after the length it reports: a week is
/// the weekly window, anything else the session. Unreported lengths fall
/// back to the slot.
fn codex_windows(quota: &QuotaUsage) -> Vec<Metric> {
    let mut metrics: Vec<Metric> = Vec::new();
    for (window, slot_id) in [
        (quota.primary.as_ref(), id::SESSION),
        (quota.secondary.as_ref(), id::WEEKLY),
    ] {
        let Some(window) = window else {
            continue;
        };
        let period = window.window_duration_mins.map(|minutes| minutes * 60);
        let mut metric_id = match period {
            Some(WEEK_SECS) => id::WEEKLY,
            Some(_) => id::SESSION,
            None => slot_id,
        };
        if metrics.iter().any(|metric| metric.id == metric_id) {
            metric_id = if metric_id == id::WEEKLY {
                id::SESSION
            } else {
                id::WEEKLY
            };
        }
        let label = if metric_id == id::WEEKLY {
            "Weekly"
        } else {
            "Session"
        };
        metrics.extend(percent_meter(metric_id, label, Some(window), period, false));
    }
    metrics
}

fn percent_meter(
    metric_id: &str,
    label: &str,
    window: Option<&Window>,
    period_secs: Option<i64>,
    starts_with_first_message: bool,
) -> Option<Metric> {
    let window = window?;
    let used = f64::from(window.utilization?);
    if !used.is_finite() {
        return None;
    }
    Some(Metric {
        id: metric_id.to_owned(),
        label: label.to_owned(),
        value: MetricValue::Meter {
            used,
            limit: 100.0,
            unit: Unit::Percent,
            resets_at: window.resets_at.clone(),
            period_secs,
            not_started: starts_with_first_message && window.resets_at.is_none() && used <= 0.0,
        },
        state: None,
    })
}

/// Pay-as-you-go spend: a meter against the cap, or a plain amount for an
/// uncapped account.
fn extra(quota: &QuotaUsage) -> Option<Metric> {
    let spend = quota.spend.as_ref()?;
    let scale = 10f64.powi(i32::try_from(spend.exponent).ok()?);
    let unit = Unit::Money {
        currency: spend.currency.clone(),
    };
    let used = spend.used_minor as f64 / scale;
    let value = match spend.limit_minor {
        Some(limit) => MetricValue::Meter {
            used,
            limit: limit as f64 / scale,
            unit,
            resets_at: None,
            period_secs: None,
            not_started: false,
        },
        None => MetricValue::Values {
            values: vec![Reading { amount: used, unit }],
            expires_at: Vec::new(),
        },
    };
    Some(Metric {
        id: id::EXTRA.to_owned(),
        label: "Extra usage".to_owned(),
        value,
        state: None,
    })
}

fn resets(quota: &QuotaUsage) -> Option<Metric> {
    let credits = quota.rate_limit_reset_credits.as_ref()?;
    let mut expires_at: Vec<String> = credits
        .credits
        .iter()
        .flatten()
        .filter(|credit| credit.status == "available")
        .filter_map(|credit| DateTime::from_timestamp(credit.expires_at?, 0))
        .map(|at| at.to_rfc3339())
        .collect();
    expires_at.sort();
    Some(Metric {
        id: id::RESETS.to_owned(),
        label: "Rate limit resets".to_owned(),
        value: MetricValue::Values {
            values: vec![Reading {
                amount: credits.available_count as f64,
                unit: Unit::Count,
            }],
            expires_at,
        },
        state: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{RateLimitResetCredits, ResetCredit, Spend};

    fn window(utilization: Option<f32>, resets_at: Option<&str>, minutes: Option<i64>) -> Window {
        Window {
            window_duration_mins: minutes,
            label: None,
            utilization,
            resets_at: resets_at.map(str::to_owned),
        }
    }

    fn usage(quota: QuotaUsage) -> ProfileUsage {
        ProfileUsage {
            quota: Some(quota),
            quota_error: None,
            fetched_at: "2026-10-17T12:00:00Z".into(),
        }
    }

    fn quota(primary: Option<Window>, secondary: Option<Window>) -> QuotaUsage {
        QuotaUsage {
            primary,
            secondary,
            scoped_weekly: Vec::new(),
            spend: None,
            rate_limit_reset_credits: None,
        }
    }

    fn ids(metrics: &[Metric]) -> Vec<&str> {
        metrics.iter().map(|metric| metric.id.as_str()).collect()
    }

    #[test]
    fn claude_reports_its_windows_in_order() {
        let mut q = quota(
            Some(window(Some(18.0), Some("2026-10-17T15:00:00Z"), None)),
            Some(window(Some(24.0), Some("2026-10-22T06:59:00Z"), None)),
        );
        q.scoped_weekly = vec![Window {
            label: Some("Fable".into()),
            ..window(Some(3.0), None, None)
        }];
        let metrics = from_usage(AppKind::Claude, &usage(q));
        assert_eq!(ids(&metrics), ["session", "weekly", "weekly.fable"]);
        assert_eq!(metrics[2].label, "Weekly · Fable");
        assert_eq!(
            metrics[0].value,
            MetricValue::Meter {
                used: 18.0,
                limit: 100.0,
                unit: Unit::Percent,
                resets_at: Some("2026-10-17T15:00:00Z".into()),
                period_secs: Some(SESSION_SECS),
                not_started: false,
            }
        );
    }

    #[test]
    fn a_claude_session_without_a_reset_time_has_not_started() {
        let metrics = from_usage(
            AppKind::Claude,
            &usage(quota(Some(window(Some(0.0), None, None)), None)),
        );
        assert!(matches!(
            metrics[0].value,
            MetricValue::Meter {
                not_started: true,
                ..
            }
        ));
    }

    #[test]
    fn a_window_without_data_is_left_out() {
        let metrics = from_usage(
            AppKind::Claude,
            &usage(quota(
                Some(window(None, None, None)),
                Some(window(Some(5.0), None, None)),
            )),
        );
        assert_eq!(ids(&metrics), ["weekly"]);
    }

    #[test]
    fn no_quota_reports_nothing() {
        let usage = ProfileUsage {
            quota: None,
            quota_error: Some(crate::usage::QuotaError::Network),
            fetched_at: String::new(),
        };
        assert!(from_usage(AppKind::Claude, &usage).is_empty());
    }

    #[test]
    fn codex_names_its_windows_by_their_length() {
        // A weekly-only plan puts its week in the first slot.
        let metrics = from_usage(
            AppKind::Codex,
            &usage(quota(Some(window(Some(2.0), None, Some(10080))), None)),
        );
        assert_eq!(ids(&metrics), ["weekly"]);

        let metrics = from_usage(
            AppKind::Codex,
            &usage(quota(
                Some(window(Some(4.0), None, Some(300))),
                Some(window(Some(2.0), None, Some(10080))),
            )),
        );
        assert_eq!(ids(&metrics), ["session", "weekly"]);
        assert!(matches!(
            metrics[0].value,
            MetricValue::Meter {
                period_secs: Some(18000),
                ..
            }
        ));
    }

    #[test]
    fn codex_never_reports_two_windows_under_one_id() {
        let metrics = from_usage(
            AppKind::Codex,
            &usage(quota(
                Some(window(Some(1.0), None, Some(10080))),
                Some(window(Some(2.0), None, Some(10080))),
            )),
        );
        assert_eq!(ids(&metrics), ["weekly", "session"]);
    }

    #[test]
    fn capped_spend_is_a_meter_and_uncapped_spend_an_amount() {
        let mut q = quota(None, None);
        q.spend = Some(Spend {
            used_minor: 1250,
            currency: "USD".into(),
            exponent: 2,
            limit_minor: Some(5000),
            percent: None,
        });
        let metrics = from_usage(AppKind::Claude, &usage(q.clone()));
        assert!(matches!(
            &metrics[0].value,
            MetricValue::Meter { used, limit, .. } if *used == 12.5 && *limit == 50.0
        ));

        q.spend.as_mut().unwrap().limit_minor = None;
        let metrics = from_usage(AppKind::Claude, &usage(q));
        assert_eq!(
            metrics[0].value,
            MetricValue::Values {
                values: vec![Reading {
                    amount: 12.5,
                    unit: Unit::Money {
                        currency: "USD".into()
                    }
                }],
                expires_at: Vec::new(),
            }
        );
    }

    #[test]
    fn resets_count_the_available_ones_with_their_expiries() {
        let mut q = quota(None, None);
        q.rate_limit_reset_credits = Some(RateLimitResetCredits {
            available_count: 2,
            credits: Some(vec![
                ResetCredit {
                    title: None,
                    status: "available".into(),
                    expires_at: Some(1_800_000_000),
                },
                ResetCredit {
                    title: None,
                    status: "used".into(),
                    expires_at: Some(1_700_000_000),
                },
                ResetCredit {
                    title: None,
                    status: "available".into(),
                    expires_at: Some(1_790_000_000),
                },
            ]),
        });
        let metrics = from_usage(AppKind::Codex, &usage(q));
        let MetricValue::Values { values, expires_at } = &metrics[0].value else {
            panic!("resets are a count");
        };
        assert_eq!(values[0].amount, 2.0);
        assert_eq!(expires_at.len(), 2);
        assert!(expires_at[0] < expires_at[1]);
    }

    #[test]
    fn metrics_round_trip_through_json() {
        let metrics = from_usage(
            AppKind::Claude,
            &usage(quota(Some(window(Some(18.0), None, None)), None)),
        );
        let json = serde_json::to_string(&metrics).unwrap();
        assert!(json.contains(r#""kind":"meter""#));
        let back: Vec<Metric> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, metrics);
    }
}
