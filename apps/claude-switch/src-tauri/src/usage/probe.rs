//! Session and Weekly of a Claude profile the usage endpoint holds back, read
//! the way Claude Code learns them itself: a one-line `claude -p` on the
//! profile, whose `stream-json` output carries the `rate_limit_event` Claude
//! Code builds from the rate-limit headers of the answer. Nothing here asks
//! the usage endpoint, so the hold is neither waited on nor extended.
//!
//! A probe is a real request on the profile's plan (haiku, no tools, MCP
//! servers, skills or hooks, nothing saved: about $0.001 of it), so a profile
//! is probed at most once every [`EVERY`], and its last reading is answered in
//! between. The reading can carry the model-scoped weekly too, as Claude
//! Code's overage-included window, but never spend; [`over`] keeps what it
//! doesn't read from the endpoint's last answer, marked with when that was.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{ProfileUsage, QuotaUsage, Window};

/// How often a held profile is probed.
pub const EVERY: Duration = Duration::from_secs(15 * 60);
/// How long a probe may take before it is killed.
const DEADLINE: Duration = Duration::from_secs(60);
/// The `claude` arguments of a probe.
const ARGS: [&str; 14] = [
    "-p",
    "Reply with just: ok",
    "--model",
    "haiku",
    "--output-format",
    "stream-json",
    "--verbose",
    "--no-session-persistence",
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--tools",
    "",
    "--settings",
    r#"{"disableAllHooks":true}"#,
];

/// Session and Weekly as a probe read them, and when.
#[derive(Debug, Clone)]
pub struct Reading {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
    /// Claude Code's overage-included weekly (`seven_day_overage_included`),
    /// which it calls the "Fable limit": the endpoint's model-scoped weekly,
    /// sent only when the account's answers carry it.
    pub overage_included: Option<Window>,
    pub read_at: DateTime<Utc>,
}

/// A profile's last probe: when it was tried, and what it read.
struct Probe {
    tried: Instant,
    reading: Option<Reading>,
}

fn probes() -> &'static Mutex<HashMap<String, Probe>> {
    static PROBES: OnceLock<Mutex<HashMap<String, Probe>>> = OnceLock::new();
    PROBES.get_or_init(Default::default)
}

/// Session and Weekly of profile `profile_id`, whose Claude Code config home
/// is `cli_config_dir`: probed when its last probe is [`EVERY`] old, else
/// what that one read. None when it read nothing.
pub async fn read(profile_id: &str, cli_config_dir: &Path) -> Option<Reading> {
    let mut probes = probes().lock().await;
    if let Some(last) = probes.get(profile_id) {
        if last.tried.elapsed() < EVERY {
            return last.reading.clone();
        }
    }
    let started = Instant::now();
    let reading = probe(cli_config_dir).await;
    match &reading {
        Some(_) => crate::logging::info(
            "usage",
            &format!(
                "{profile_id}: held, Session and Weekly read through claude -p in {:.1}s",
                started.elapsed().as_secs_f32()
            ),
        ),
        None => crate::logging::warn(
            "usage",
            &format!("{profile_id}: held, and claude -p read no Session or Weekly"),
        ),
    }
    probes.insert(
        profile_id.to_owned(),
        Probe {
            tried: Instant::now(),
            reading: reading.clone(),
        },
    );
    reading
}

/// Pure: `last`, the last usage shown (or nothing), with what `reading` read
/// in place of its figures: Session and Weekly, and the model-scoped weekly
/// when the reading has the overage-included window and `last` has a single
/// scoped weekly for it to be. With several, which is which isn't guessed.
/// What the reading leaves as it was keeps when it was read, in its
/// `read_at`, so it isn't shown as read now.
pub fn over(last: Option<ProfileUsage>, reading: Reading) -> QuotaUsage {
    let last_read_at = last.as_ref().map(|usage| usage.fetched_at.clone());
    let mut usage = last.and_then(|usage| usage.quota).unwrap_or_default();
    let carry = |read_at: &mut Option<String>| {
        if read_at.is_none() {
            read_at.clone_from(&last_read_at);
        }
    };
    for (window, read) in [
        (&mut usage.primary, reading.primary),
        (&mut usage.secondary, reading.secondary),
    ] {
        match read {
            Some(read) => *window = Some(read),
            None => window.iter_mut().for_each(|kept| carry(&mut kept.read_at)),
        }
    }
    match (reading.overage_included, usage.scoped_weekly.as_mut_slice()) {
        (Some(read), [scoped]) => {
            scoped.utilization = read.utilization;
            scoped.resets_at = read.resets_at;
            scoped.read_at = None;
        }
        (_, scoped) => scoped
            .iter_mut()
            .for_each(|window| carry(&mut window.read_at)),
    }
    if let Some(spend) = usage.spend.as_mut() {
        carry(&mut spend.read_at);
    }
    usage
}

async fn probe(cli_config_dir: &Path) -> Option<Reading> {
    let binary = crate::deps::resolve_cli_binary_path("claude")?;
    // The refresh's empty scratch dir: claude scans its cwd on startup.
    let cwd = crate::paths::refresh_cwd_dir().ok()?;
    std::fs::create_dir_all(&cwd).ok()?;
    let mut command = tokio::process::Command::new(binary);
    command
        .args(ARGS)
        .current_dir(&cwd)
        .env("PATH", crate::deps::shell_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // As the refresh does: nothing inherited, and the stock profile on no
    // config home of its own (see `refresh::spawn_claude`).
    for key in crate::inherited_env::current() {
        command.env_remove(key);
    }
    if !super::is_stock_default_cli_config_dir(cli_config_dir) {
        command.env("CLAUDE_CONFIG_DIR", cli_config_dir);
    }
    let output = tokio::time::timeout(DEADLINE, command.output())
        .await
        .ok()?
        .ok()?;
    reading_from(&String::from_utf8_lossy(&output.stdout), Utc::now())
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: Option<String>,
    rate_limit_info: Option<RateLimitInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitInfo {
    unified_windows: Option<UnifiedWindows>,
}

#[derive(Deserialize)]
struct UnifiedWindows {
    five_hour: Option<UnifiedWindow>,
    seven_day: Option<UnifiedWindow>,
    seven_day_overage_included: Option<UnifiedWindow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedWindow {
    /// A share, 0..=1.
    utilization: Option<f64>,
    /// Unix seconds.
    resets_at: Option<i64>,
}

/// Pure: Session, Weekly and the overage-included weekly from the last
/// `rate_limit_event` of a probe's `stream-json` output, finished at
/// `read_at`, or None when it has none of them.
fn reading_from(output: &str, read_at: DateTime<Utc>) -> Option<Reading> {
    let windows = output
        .lines()
        .filter_map(|line| serde_json::from_str::<Event>(line).ok())
        .filter(|event| event.kind.as_deref() == Some("rate_limit_event"))
        .filter_map(|event| event.rate_limit_info?.unified_windows)
        .next_back()?;
    let reading = Reading {
        primary: windows.five_hour.map(into_window),
        secondary: windows.seven_day.map(into_window),
        overage_included: windows.seven_day_overage_included.map(into_window),
        read_at,
    };
    (reading.primary.is_some() || reading.secondary.is_some() || reading.overage_included.is_some())
        .then_some(reading)
}

/// Pure: an unified window as the endpoint would have given it: a percentage,
/// and an RFC 3339 reset.
fn into_window(raw: UnifiedWindow) -> Window {
    Window {
        window_duration_mins: None,
        label: None,
        utilization: raw
            .utilization
            .filter(|share| share.is_finite() && *share >= 0.0)
            .map(|share| ((share * 1000.0).round() / 10.0) as f32),
        resets_at: raw
            .resets_at
            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
            .map(|at| at.to_rfc3339()),
        read_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a probe printed (trimmed), its rate-limit event last but one.
    const OUTPUT: &str = r#"{"type":"system","subtype":"init","session_id":"s"}
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1792218000,"rateLimitType":"five_hour","unifiedWindows":{"five_hour":{"utilization":0.12,"resetsAt":1792218000},"seven_day":{"utilization":0.2,"resetsAt":1792724400}}}}
{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}
{"type":"result","subtype":"success","total_cost_usd":0.0008}"#;

    #[test]
    fn session_and_weekly_come_from_the_rate_limit_event() {
        let reading = reading_from(OUTPUT, Utc::now()).unwrap();
        let primary = reading.primary.unwrap();
        assert_eq!(primary.utilization, Some(12.0));
        assert_eq!(
            primary.resets_at.as_deref(),
            Some("2026-10-17T06:20:00+00:00")
        );
        let secondary = reading.secondary.unwrap();
        assert_eq!(secondary.utilization, Some(20.0));
        assert_eq!(
            secondary.resets_at.as_deref(),
            Some("2026-10-23T03:00:00+00:00")
        );
    }

    #[test]
    fn the_last_event_wins() {
        let later = r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.13,"resetsAt":1792218000}}}}"#;
        let reading = reading_from(&format!("{OUTPUT}\n{later}"), Utc::now()).unwrap();
        assert_eq!(reading.primary.unwrap().utilization, Some(13.0));
        assert!(reading.secondary.is_none());
    }

    #[test]
    fn output_without_windows_reads_nothing() {
        assert!(reading_from("", Utc::now()).is_none());
        assert!(reading_from("not json\n{\"type\":\"result\"}", Utc::now()).is_none());
        let bare = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#;
        assert!(reading_from(bare, Utc::now()).is_none());
    }

    /// When the endpoint last answered, in the tests below.
    const LAST_FETCHED_AT: &str = "2026-11-13T09:50:22+00:00";

    fn scoped(model: &str, share: f64) -> Window {
        Window {
            label: Some(model.into()),
            ..into_window(UnifiedWindow {
                utilization: Some(share),
                resets_at: Some(1795125600),
            })
        }
    }

    /// The endpoint's last answer: Session, Weekly, `scoped` and spend.
    fn last(scoped_weekly: Vec<Window>) -> ProfileUsage {
        let window = |share| {
            Some(into_window(UnifiedWindow {
                utilization: Some(share),
                resets_at: Some(1792724400),
            }))
        };
        ProfileUsage {
            quota: Some(QuotaUsage {
                primary: window(0.21),
                secondary: window(0.16),
                scoped_weekly,
                spend: Some(super::super::Spend {
                    used_minor: 7788,
                    currency: "GBP".into(),
                    exponent: 2,
                    limit_minor: Some(30000),
                    percent: Some(26.0),
                    read_at: None,
                }),
                ..QuotaUsage::default()
            }),
            quota_error: None,
            stored_account_email: None,
            fetched_at: LAST_FETCHED_AT.into(),
        }
    }

    #[test]
    fn the_overage_included_window_is_read_when_sent() {
        let event = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"seven_day","unifiedWindows":{"seven_day":{"utilization":0.21,"resetsAt":1795125600},"seven_day_overage_included":{"utilization":0.62,"resetsAt":1795125600}}}}"#;
        let reading = reading_from(event, Utc::now()).unwrap();
        let fable = reading.overage_included.unwrap();
        assert_eq!(fable.utilization, Some(62.0));
        assert_eq!(
            fable.resets_at.as_deref(),
            Some("2026-11-19T22:00:00+00:00")
        );
        assert!(reading_from(OUTPUT, Utc::now())
            .unwrap()
            .overage_included
            .is_none());
    }

    #[test]
    fn a_reading_marks_the_rows_it_has_none_of_as_read_before() {
        let reading = reading_from(OUTPUT, Utc::now()).unwrap();

        let usage = over(Some(last(vec![scoped("Fable", 0.38)])), reading.clone());
        let primary = usage.primary.unwrap();
        assert_eq!(primary.utilization, Some(12.0));
        assert!(primary.read_at.is_none());
        assert_eq!(usage.secondary.unwrap().utilization, Some(20.0));
        // The probe said nothing of Fable or spend: still the endpoint's,
        // and marked as read when the endpoint answered.
        let fable = &usage.scoped_weekly[0];
        assert_eq!(fable.label.as_deref(), Some("Fable"));
        assert_eq!(fable.utilization, Some(38.0));
        assert_eq!(fable.read_at.as_deref(), Some(LAST_FETCHED_AT));
        assert_eq!(
            usage.spend.unwrap().read_at.as_deref(),
            Some(LAST_FETCHED_AT)
        );

        let fresh = over(None, reading);
        assert!(fresh.scoped_weekly.is_empty());
        assert!(fresh.primary.is_some());
    }

    #[test]
    fn a_row_carried_twice_keeps_when_it_was_first_read() {
        let first = over(
            Some(last(vec![scoped("Fable", 0.38)])),
            reading_from(OUTPUT, Utc::now()).unwrap(),
        );
        let held = ProfileUsage {
            quota: Some(first),
            quota_error: None,
            stored_account_email: None,
            fetched_at: "2026-11-13T10:05:00+00:00".into(),
        };
        let only_weekly = r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"seven_day":{"utilization":0.22,"resetsAt":1792724400}}}}"#;
        let usage = over(Some(held), reading_from(only_weekly, Utc::now()).unwrap());
        assert_eq!(
            usage.scoped_weekly[0].read_at.as_deref(),
            Some(LAST_FETCHED_AT)
        );
        // Session was read by the first probe, so it dates from that one.
        assert_eq!(
            usage.primary.unwrap().read_at.as_deref(),
            Some("2026-11-13T10:05:00+00:00")
        );
    }

    #[test]
    fn the_overage_included_window_updates_the_one_scoped_weekly() {
        let event = r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"seven_day":{"utilization":0.21,"resetsAt":1795125600},"seven_day_overage_included":{"utilization":0.62,"resetsAt":1795125600}}}}"#;
        let reading = reading_from(event, Utc::now()).unwrap();
        let usage = over(Some(last(vec![scoped("Fable", 0.38)])), reading);
        let fable = &usage.scoped_weekly[0];
        assert_eq!(fable.label.as_deref(), Some("Fable"));
        assert_eq!(fable.utilization, Some(62.0));
        assert!(fable.read_at.is_none());
    }

    #[test]
    fn with_several_scoped_weeklies_none_is_guessed_at() {
        let event = r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"seven_day_overage_included":{"utilization":0.62,"resetsAt":1795125600}}}}"#;
        let reading = reading_from(event, Utc::now()).unwrap();
        let usage = over(
            Some(last(vec![scoped("Fable", 0.38), scoped("Opus", 0.05)])),
            reading,
        );
        assert_eq!(usage.scoped_weekly[0].utilization, Some(38.0));
        assert_eq!(usage.scoped_weekly[1].utilization, Some(5.0));
        assert!(usage
            .scoped_weekly
            .iter()
            .all(|window| window.read_at.as_deref() == Some(LAST_FETCHED_AT)));
    }
}
