//! Session and Weekly of a Claude profile the usage endpoint holds back, read
//! the way Claude Code learns them itself: a one-line `claude -p` on the
//! profile, whose `stream-json` output carries the `rate_limit_event` Claude
//! Code builds from the rate-limit headers of the answer. Nothing here asks
//! the usage endpoint, so the hold is neither waited on nor extended.
//!
//! A probe is a real request on the profile's plan (haiku, no tools, MCP
//! servers, skills or hooks, nothing saved: about $0.001 of it), so a profile
//! is probed at most once every [`EVERY`], and its last reading is answered in
//! between. The reading has no model-scoped weekly or spend row; [`over`]
//! keeps those from the endpoint's last answer.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{QuotaUsage, Window};

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

/// Pure: `last`, the endpoint's last answer (or nothing), with Session and
/// Weekly replaced by those `reading` has.
pub fn over(last: Option<QuotaUsage>, reading: Reading) -> QuotaUsage {
    let mut usage = last.unwrap_or_default();
    if reading.primary.is_some() {
        usage.primary = reading.primary;
    }
    if reading.secondary.is_some() {
        usage.secondary = reading.secondary;
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedWindow {
    /// A share, 0..=1.
    utilization: Option<f64>,
    /// Unix seconds.
    resets_at: Option<i64>,
}

/// Pure: Session and Weekly from the last `rate_limit_event` of a probe's
/// `stream-json` output, finished at `read_at`, or None when it has neither.
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
        read_at,
    };
    (reading.primary.is_some() || reading.secondary.is_some()).then_some(reading)
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

    #[test]
    fn a_reading_keeps_the_rows_it_has_none_of() {
        let scoped = Window {
            window_duration_mins: None,
            label: Some("Fable".into()),
            utilization: Some(0.0),
            resets_at: None,
        };
        let last = QuotaUsage {
            primary: Some(into_window(UnifiedWindow {
                utilization: Some(0.21),
                resets_at: Some(1792200000),
            })),
            secondary: Some(into_window(UnifiedWindow {
                utilization: Some(0.16),
                resets_at: Some(1792724400),
            })),
            scoped_weekly: vec![scoped],
            ..QuotaUsage::default()
        };
        let reading = reading_from(OUTPUT, Utc::now()).unwrap();

        let usage = over(Some(last), reading.clone());
        assert_eq!(usage.primary.unwrap().utilization, Some(12.0));
        assert_eq!(usage.secondary.unwrap().utilization, Some(20.0));
        assert_eq!(usage.scoped_weekly[0].label.as_deref(), Some("Fable"));

        let fresh = over(None, reading);
        assert!(fresh.scoped_weekly.is_empty());
        assert!(fresh.primary.is_some());
    }
}
