//! The JSON `claude-switch usage` prints and the local API serves. Pure:
//! built from the menu bar's entries.
//!
//! - [`limits`]: each profile's limits as plain numbers under stable keys,
//!   schema `claude-switch.limits.v1`. What scripts and other apps should read.
//! - [`usage`]: each profile as the panel shows it, metrics and pacing included.
//!
//! The limits envelope follows OpenUsage's `/v1/limits`
//! (MIT, <https://github.com/robinebers/openusage>).

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde::{Serialize, Serializer};

use crate::app_kind::AppKind;
use crate::tray::TrayEntry;
use crate::usage::metrics::{self, id, Metric, MetricValue, Unit};
use crate::usage::QuotaError;

pub const SCHEMA: &str = "claude-switch.limits.v1";

/// How long a profile's figures count as current: the menu bar fetches them
/// this often.
pub const FRESH_FOR: TimeDelta = TimeDelta::minutes(5);

/// Spend and history from the local logs: not limits, so left out of them.
const HISTORY: [&str; 4] = [id::TODAY, id::YESTERDAY, id::LAST_30_DAYS, id::TREND];

/// Counts of something the account has to use, rather than has used.
const BALANCES: [&str; 1] = [id::RESETS];

/// When `entry`'s figures were fetched, if it has any.
pub fn fetched_at(entry: &TrayEntry) -> Option<DateTime<Utc>> {
    let usage = entry.usage.as_ref()?;
    DateTime::parse_from_rfc3339(&usage.fetched_at)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// The limits envelope for `entries`, as of `now`.
///
/// A profile without figures yet has no entry under `profiles`, rather than
/// zeros; one whose latest fetch failed is also listed under `errors`, its
/// last good figures kept.
pub fn limits(entries: &[TrayEntry], now: DateTime<Utc>) -> String {
    let profiles = entries
        .iter()
        .filter_map(|entry| Some((entry.id.clone(), profile_limits(entry, now)?)))
        .collect();
    let errors = entries
        .iter()
        .filter_map(|entry| {
            let error = entry.error?;
            Some(LimitsError {
                profile_id: entry.id.clone(),
                code: error,
                message: describe(error, &entry.command),
            })
        })
        .collect();
    let envelope = Limits {
        schema: SCHEMA,
        generated_at: timestamp(now),
        profiles: Keyed(profiles),
        errors,
    };
    serde_json::to_string(&envelope).unwrap_or_default()
}

/// `entries` as the panel shows them, each meter's pacing as of `now`.
pub fn usage(entries: &[TrayEntry], now: DateTime<Utc>) -> String {
    let profiles: Vec<UsageProfile> = entries
        .iter()
        .map(|entry| UsageProfile {
            id: &entry.id,
            app: entry.app,
            name: &entry.name,
            plan: entry.plan.as_deref(),
            fetched_at: fetched_at(entry).map(timestamp),
            error: entry.error,
            metrics: metrics::with_states(&entry.metrics, now),
        })
        .collect();
    serde_json::to_string(&profiles).unwrap_or_default()
}

/// What went wrong with a fetch, in a line; `command` is the CLI command
/// the profile signs in with.
pub fn describe(error: QuotaError, command: &str) -> String {
    match error {
        QuotaError::NoCredentials => format!("not signed in: run `{command}` and sign in"),
        QuotaError::NeedsLogin => format!("sign-in expired: run `{command}` and sign in again"),
        QuotaError::Unauthorized => {
            format!("token refresh needed: open claude-switch, or run `{command}` once")
        }
        QuotaError::Forbidden => "the usage request was blocked upstream".to_owned(),
        QuotaError::RateLimited => "rate limited: try again in a few minutes".to_owned(),
        QuotaError::Network => "couldn't reach the usage service".to_owned(),
        QuotaError::Unknown => "couldn't read usage".to_owned(),
    }
}

fn profile_limits(entry: &TrayEntry, now: DateTime<Utc>) -> Option<ProfileLimits> {
    let fetched_at = fetched_at(entry)?;
    let expires_at = fetched_at + FRESH_FOR;
    let resources = entry
        .metrics
        .iter()
        .filter(|metric| !HISTORY.contains(&metric.id.as_str()))
        .filter_map(|metric| Some((metric.id.clone(), resource(metric)?)))
        .collect();
    Some(ProfileLimits {
        app: entry.app,
        name: entry.name.clone(),
        plan: entry.plan.clone(),
        fetched_at: timestamp(fetched_at),
        expires_at: timestamp(expires_at),
        stale: now >= expires_at,
        resources: Keyed(resources),
    })
}

fn resource(metric: &Metric) -> Option<Resource> {
    match &metric.value {
        MetricValue::Meter {
            used,
            limit,
            unit,
            resets_at,
            period_secs,
            ..
        } => {
            let used = used.max(0.0);
            let limit = limit.max(0.0);
            Some(Resource {
                kind: Kind::Consumption,
                unit: unit_name(unit),
                used: Some(used),
                limit: Some(limit),
                remaining: Some((limit - used).max(0.0)),
                utilization: (limit > 0.0).then(|| used / limit),
                resets_at: resets_at.as_deref().map(normalized),
                window_seconds: *period_secs,
                ..Resource::default()
            })
        }
        MetricValue::Values {
            values, expires_at, ..
        } => {
            let reading = values.first()?;
            let balance = BALANCES.contains(&metric.id.as_str());
            Some(Resource {
                kind: if balance {
                    Kind::Balance
                } else {
                    Kind::Consumption
                },
                unit: unit_name(&reading.unit),
                used: (!balance).then_some(reading.amount),
                available: balance.then_some(reading.amount),
                expires_at: (!expires_at.is_empty())
                    .then(|| expires_at.iter().map(|at| normalized(at)).collect()),
                ..Resource::default()
            })
        }
        MetricValue::Chart { .. } => None,
    }
}

fn unit_name(unit: &Unit) -> String {
    match unit {
        Unit::Percent => "percent".to_owned(),
        Unit::Count => "count".to_owned(),
        Unit::Money { currency } => currency.to_lowercase(),
        Unit::Tokens => "tokens".to_owned(),
    }
}

/// `at` to the millisecond, in UTC: `2026-10-17T12:00:00.000Z`.
fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A time an endpoint reported, in [`timestamp`]'s form when it parses.
fn normalized(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|at| timestamp(at.with_timezone(&Utc)))
        .unwrap_or_else(|_| at.to_owned())
}

/// Pairs serialized as a JSON object, in their order.
struct Keyed<T>(Vec<(String, T)>);

impl<T: Serialize> Serialize for Keyed<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(key, value)| (key, value)))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    schema: &'static str,
    generated_at: String,
    profiles: Keyed<ProfileLimits>,
    errors: Vec<LimitsError>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileLimits {
    app: AppKind,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    fetched_at: String,
    expires_at: String,
    stale: bool,
    resources: Keyed<Resource>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LimitsError {
    profile_id: String,
    code: QuotaError,
    message: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
enum Kind {
    #[default]
    Consumption,
    Balance,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Resource {
    kind: Kind,
    unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    available: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    utilization: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageProfile<'a> {
    id: &'a str,
    app: AppKind,
    name: &'a str,
    plan: Option<&'a str>,
    fetched_at: Option<String>,
    error: Option<QuotaError>,
    metrics: Vec<Metric>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::usage::metrics::Reading;
    use crate::usage::ProfileUsage;
    use serde_json::{json, Value};

    pub(crate) fn meter(metric_id: &str, used: f64, resets_at: Option<&str>) -> Metric {
        Metric {
            id: metric_id.into(),
            label: metric_id.into(),
            value: MetricValue::Meter {
                used,
                limit: 100.0,
                unit: Unit::Percent,
                resets_at: resets_at.map(str::to_owned),
                period_secs: Some(18_000),
                not_started: false,
            },
            state: None,
        }
    }

    pub(crate) fn entry(entry_id: &str, app: AppKind, fetched_at: Option<&str>) -> TrayEntry {
        TrayEntry {
            id: entry_id.into(),
            app,
            name: format!("Profile {entry_id}"),
            command: format!("claude-{entry_id}"),
            plan: None,
            usage: fetched_at.map(|at| ProfileUsage {
                quota: None,
                quota_error: None,
                fetched_at: at.into(),
            }),
            metrics: if fetched_at.is_some() {
                vec![meter(id::SESSION, 18.0, Some("2026-10-17T15:00:00+00:00"))]
            } else {
                Vec::new()
            },
            error: None,
            retry_at: None,
        }
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn parsed(body: &str) -> Value {
        serde_json::from_str(body).unwrap()
    }

    #[test]
    fn a_meter_is_consumption_with_its_numbers_in_full() {
        let mut work = entry(
            "w",
            AppKind::Claude,
            Some("2026-10-17T12:00:00.123456+00:00"),
        );
        work.plan = Some("Max 20x".into());
        let body = parsed(&limits(&[work], at("2026-10-17T12:01:00Z")));
        assert_eq!(
            body,
            json!({
                "schema": "claude-switch.limits.v1",
                "generatedAt": "2026-10-17T12:01:00.000Z",
                "profiles": {
                    "w": {
                        "app": "claude",
                        "name": "Profile w",
                        "plan": "Max 20x",
                        "fetchedAt": "2026-10-17T12:00:00.123Z",
                        "expiresAt": "2026-10-17T12:05:00.123Z",
                        "stale": false,
                        "resources": {
                            "session": {
                                "kind": "consumption",
                                "unit": "percent",
                                "used": 18.0,
                                "limit": 100.0,
                                "remaining": 82.0,
                                "utilization": 0.18,
                                "resetsAt": "2026-10-17T15:00:00.000Z",
                                "windowSeconds": 18000
                            }
                        }
                    }
                },
                "errors": []
            })
        );
    }

    #[test]
    fn figures_older_than_five_minutes_are_stale() {
        let work = entry("w", AppKind::Claude, Some("2026-10-17T12:00:00Z"));
        let fresh = parsed(&limits(
            std::slice::from_ref(&work),
            at("2026-10-17T12:04:59Z"),
        ));
        assert_eq!(fresh["profiles"]["w"]["stale"], false);
        let stale = parsed(&limits(&[work], at("2026-10-17T12:05:00Z")));
        assert_eq!(stale["profiles"]["w"]["stale"], true);
    }

    #[test]
    fn resets_are_a_balance_and_uncapped_spend_is_consumption() {
        let mut work = entry("w", AppKind::Claude, Some("2026-10-17T12:00:00Z"));
        work.metrics = vec![
            Metric {
                id: id::RESETS.into(),
                label: "Rate limit resets".into(),
                value: MetricValue::Values {
                    values: vec![Reading {
                        amount: 2.0,
                        unit: Unit::Count,
                    }],
                    expires_at: vec!["2026-11-01T00:00:00+00:00".into()],
                    breakdown: None,
                },
                state: None,
            },
            Metric {
                id: id::EXTRA.into(),
                label: "Extra usage".into(),
                value: MetricValue::Values {
                    values: vec![Reading {
                        amount: 4.5,
                        unit: Unit::Money {
                            currency: "GBP".into(),
                        },
                    }],
                    expires_at: Vec::new(),
                    breakdown: None,
                },
                state: None,
            },
        ];
        let body = parsed(&limits(&[work], at("2026-10-17T12:01:00Z")));
        let resources = &body["profiles"]["w"]["resources"];
        assert_eq!(
            resources["resets"],
            json!({
                "kind": "balance",
                "unit": "count",
                "available": 2.0,
                "expiresAt": ["2026-11-01T00:00:00.000Z"]
            })
        );
        assert_eq!(
            resources["extra"],
            json!({ "kind": "consumption", "unit": "gbp", "used": 4.5 })
        );
    }

    #[test]
    fn history_metrics_are_not_limits() {
        let mut work = entry("w", AppKind::Codex, Some("2026-10-17T12:00:00Z"));
        work.metrics.push(Metric {
            id: id::TODAY.into(),
            label: "Today".into(),
            value: MetricValue::Values {
                values: vec![Reading {
                    amount: 5.17,
                    unit: Unit::Money {
                        currency: "USD".into(),
                    },
                }],
                expires_at: Vec::new(),
                breakdown: None,
            },
            state: None,
        });
        let body = parsed(&limits(&[work], at("2026-10-17T12:01:00Z")));
        let resources = body["profiles"]["w"]["resources"].as_object().unwrap();
        assert_eq!(resources.keys().collect::<Vec<_>>(), ["session"]);
    }

    #[test]
    fn a_profile_without_figures_is_left_out_and_a_failed_fetch_is_an_error() {
        let mut never = entry("n", AppKind::Claude, None);
        never.error = Some(QuotaError::NeedsLogin);
        let mut kept = entry("k", AppKind::Codex, Some("2026-10-17T12:00:00Z"));
        kept.command = "codex-k".into();
        kept.error = Some(QuotaError::Network);
        let body = parsed(&limits(&[never, kept], at("2026-10-17T12:01:00Z")));
        let profiles = body["profiles"].as_object().unwrap();
        assert_eq!(profiles.keys().collect::<Vec<_>>(), ["k"]);
        assert_eq!(
            body["errors"],
            json!([
                {
                    "profileId": "n",
                    "code": "needs_login",
                    "message": "sign-in expired: run `claude-n` and sign in again"
                },
                {
                    "profileId": "k",
                    "code": "network",
                    "message": "couldn't reach the usage service"
                }
            ])
        );
    }

    #[test]
    fn profiles_keep_their_order() {
        let entries = [
            entry("zeta", AppKind::Claude, Some("2026-10-17T12:00:00Z")),
            entry("alpha", AppKind::Claude, Some("2026-10-17T12:00:00Z")),
        ];
        let body = limits(&entries, at("2026-10-17T12:01:00Z"));
        assert!(body.find("\"zeta\"").unwrap() < body.find("\"alpha\"").unwrap());
    }

    #[test]
    fn usage_lists_every_profile_with_its_pacing_and_no_credentials() {
        let mut work = entry("w", AppKind::Claude, Some("2026-10-17T12:00:00Z"));
        work.plan = Some("Pro".into());
        let never = entry("n", AppKind::Codex, None);
        let body = parsed(&usage(&[work, never], at("2026-10-17T12:01:00Z")));
        let profiles = body.as_array().unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0]["id"], "w");
        assert_eq!(profiles[0]["plan"], "Pro");
        assert_eq!(profiles[0]["fetchedAt"], "2026-10-17T12:00:00.000Z");
        assert_eq!(profiles[0]["metrics"][0]["id"], "session");
        assert!(profiles[0]["metrics"][0]["state"].is_object());
        assert_eq!(profiles[1]["fetchedAt"], Value::Null);
        assert_eq!(profiles[1]["metrics"], json!([]));
        let keys: Vec<&String> = profiles[0].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["id", "app", "name", "plan", "fetchedAt", "error", "metrics"]
        );
    }
}
