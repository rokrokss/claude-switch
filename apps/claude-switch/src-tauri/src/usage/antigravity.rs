//! Usage of an Antigravity profile, read the way Antigravity's CLI reports it:
//! `agy -p /usage --output-format json`, which runs the slash command and
//! starts no model turn (agy 1.1.11 on). CodexBar, cc-switch and agy-hud read
//! it the same way; it is the one source that needs neither the desktop app
//! running nor agy's local server, which wants a CSRF token from 1.2.2 on.
//!
//! Models share limits by group. Gemini's 5-hour and weekly windows are the
//! profile's Session and Weekly; every other group ("Claude and GPT models",
//! "Gemini 4 Argon") is an additional limit of its own.
//!
//! A run boots agy's language server and takes seconds, so runs go one at a
//! time and an answer is reused for [`REUSE`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;

use super::{NamedLimit, QuotaError, QuotaUsage, Window};
use crate::app_kind::ANTIGRAVITY;

/// How long a run may take before it is killed: agy's own print timeout, and
/// time to start.
const DEADLINE: Duration = Duration::from_secs(75);
/// The `agy` arguments of a run.
const ARGS: [&str; 6] = [
    "-p",
    "/usage",
    "--output-format",
    "json",
    "--print-timeout",
    "60s",
];
/// How long an answer is reused for.
const REUSE: Duration = Duration::from_secs(120);
/// Where in a profile's config dir agy keeps its sign-in when it uses a file.
pub(crate) const TOKEN_FILE: &str = "antigravity-oauth-token";
/// Window lengths of the buckets, in minutes.
const FIVE_HOURS: i64 = 5 * 60;
const WEEK: i64 = 7 * 24 * 60;

/// Fetches Antigravity usage through `agy`.
pub struct AntigravityQuotaProvider;

#[async_trait::async_trait]
impl super::QuotaProvider for AntigravityQuotaProvider {
    /// `config_dir` is the stock `~/.gemini/antigravity-cli`, which agy reads
    /// as it is, or a profile's `cli-config`, which agy is given as its home.
    async fn fetch(&self, config_dir: &Path) -> Result<QuotaUsage, QuotaError> {
        let stock =
            crate::paths::stock_cli_config_dir(&ANTIGRAVITY).is_ok_and(|stock| stock == config_dir);
        // Signed out, agy would start a sign-in; a profile's sign-in is a file,
        // so that is told without running it.
        if !stock && !signed_in(config_dir) {
            return Err(QuotaError::NoCredentials);
        }
        let mut answers = answers().lock().await;
        if let Some((at, answer)) = answers.get(config_dir) {
            if at.elapsed() < REUSE {
                return answer.clone();
            }
        }
        let answer = run(config_dir, stock).await;
        answers.insert(config_dir.to_path_buf(), (Instant::now(), answer.clone()));
        answer
    }
}

type Answer = Result<QuotaUsage, QuotaError>;

fn answers() -> &'static Mutex<HashMap<PathBuf, (Instant, Answer)>> {
    static ANSWERS: OnceLock<Mutex<HashMap<PathBuf, (Instant, Answer)>>> = OnceLock::new();
    ANSWERS.get_or_init(Default::default)
}

/// Whether the profile with config home `config_dir` (its `cli-config`) has
/// a sign-in file.
fn signed_in(config_dir: &Path) -> bool {
    config_dir
        .join(ANTIGRAVITY.cli_config_subdir.unwrap_or_default())
        .join(TOKEN_FILE)
        .is_file()
}

async fn run(config_dir: &Path, stock: bool) -> Answer {
    let Some(binary) = crate::deps::resolve_cli_binary_path(ANTIGRAVITY.cli_binary) else {
        crate::logging::warn(
            "usage",
            "agy isn't on the PATH, so Antigravity usage can't be read",
        );
        return Err(QuotaError::Network);
    };
    let (Ok(cwd), Ok(no_browser)) = (crate::paths::refresh_cwd_dir(), no_browser_dir()) else {
        return Err(QuotaError::Unknown);
    };
    let _ = std::fs::create_dir_all(&cwd);
    let mut command = tokio::process::Command::new(binary);
    command
        .args(ARGS)
        // An empty dir: agy reads its working dir as a workspace.
        .current_dir(&cwd)
        // A sign-in that lapsed would open a browser every few minutes; the
        // `open` and `BROWSER` it would use fail instead.
        .env(
            "PATH",
            format!("{}:{}", no_browser.display(), crate::deps::shell_path()),
        )
        .env("BROWSER", no_browser.join("open"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in crate::inherited_env::current() {
        command.env_remove(key);
    }
    // With a key, agy reports the API's usage rather than the account's.
    command.env_remove("GEMINI_API_KEY");
    if !stock {
        command.env(ANTIGRAVITY.cli_config_env, config_dir);
        for (key, value) in ANTIGRAVITY.cli_extra_env {
            command.env(key, value);
        }
    }
    let output = match tokio::time::timeout(DEADLINE, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            crate::logging::warn("usage", &format!("agy couldn't be started: {err}"));
            return Err(QuotaError::Network);
        }
        Err(_) => {
            crate::logging::warn("usage", "agy -p /usage didn't answer in time");
            return Err(QuotaError::Network);
        }
    };
    let answer = parse(&String::from_utf8_lossy(&output.stdout));
    if let Err(error) = &answer {
        crate::logging::warn(
            "usage",
            &format!(
                "agy -p /usage answered {error:?} (exit {:?})",
                output.status.code()
            ),
        );
    }
    answer
}

/// `<app data>/agy-no-browser/`, holding an `open` and an `xdg-open` that
/// fail, made on first use.
fn no_browser_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = crate::paths::app_data_dir()
        .map_err(|err| std::io::Error::other(err.message()))?
        .join("agy-no-browser");
    std::fs::create_dir_all(&dir)?;
    for name in ["open", "xdg-open"] {
        let stub = dir.join(name);
        if !stub.is_file() {
            std::fs::write(&stub, "#!/bin/sh\nexit 1\n")?;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(dir)
}

/// What `agy -p /usage --output-format json` prints. Field names are taken in
/// either case, as agy's local server spells them in camelCase.
#[derive(Debug, Deserialize)]
struct Report {
    status: Option<String>,
    /// The command's text, which says why when it failed.
    response: Option<String>,
    command: Option<Command>,
}

#[derive(Debug, Deserialize)]
struct Command {
    data: Option<Data>,
}

#[derive(Debug, Deserialize)]
struct Data {
    #[serde(default)]
    groups: Vec<Group>,
}

#[derive(Debug, Deserialize)]
struct Group {
    #[serde(alias = "displayName")]
    name: Option<String>,
    #[serde(default)]
    buckets: Vec<Bucket>,
}

#[derive(Debug, Deserialize)]
struct Bucket {
    #[serde(alias = "bucketId")]
    id: Option<String>,
    /// `5h` or `weekly`.
    window: Option<String>,
    #[serde(alias = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(alias = "resetTime")]
    reset_time: Option<String>,
    #[serde(default)]
    disabled: bool,
}

/// Pure: the usage `stdout` of `agy -p /usage --output-format json` reports.
fn parse(stdout: &str) -> Answer {
    // Anything agy logs ahead of the report isn't JSON.
    let json = match (stdout.find('{'), stdout.rfind('}')) {
        (Some(start), Some(end)) if start < end => &stdout[start..=end],
        _ => return Err(QuotaError::Unknown),
    };
    let report: Report = serde_json::from_str(json).map_err(|_| QuotaError::Unknown)?;
    if report.status.as_deref() != Some("SUCCESS") {
        return Err(failure(report.response.as_deref().unwrap_or_default()));
    }
    let groups = report
        .command
        .and_then(|command| command.data)
        .map(|data| data.groups)
        .unwrap_or_default();
    let mut usage = QuotaUsage::default();
    for group in groups {
        let five_hours = window_of(&group.buckets, "5h", FIVE_HOURS);
        let weekly = window_of(&group.buckets, "weekly", WEEK);
        if group
            .buckets
            .iter()
            .any(|bucket| bucket.id.as_deref().is_some_and(is_gemini_main))
        {
            usage.primary = five_hours;
            usage.secondary = weekly;
        } else if five_hours.is_some() || weekly.is_some() {
            usage.additional_limits.push(NamedLimit {
                name: group.name.unwrap_or_else(|| "Other models".to_owned()),
                primary: five_hours,
                secondary: weekly,
            });
        }
    }
    if usage.primary.is_none() && usage.secondary.is_none() && usage.additional_limits.is_empty() {
        return Err(QuotaError::Unknown);
    }
    Ok(usage)
}

/// Gemini's main buckets, by id: matched exactly, as the Argon group's ids
/// start with `gemini-` too.
fn is_gemini_main(id: &str) -> bool {
    matches!(id, "gemini-5h" | "gemini-weekly")
}

/// The enabled bucket of `buckets` for `window` (`5h` or `weekly`), named by
/// its `window` or, failing that, its id's suffix, as a [`Window`] of
/// `minutes`. A bucket that doesn't say how much is left is unknown, not
/// used up.
fn window_of(buckets: &[Bucket], window: &str, minutes: i64) -> Option<Window> {
    let bucket = buckets.iter().find(|bucket| {
        !bucket.disabled
            && (bucket.window.as_deref() == Some(window)
                || bucket.window.is_none()
                    && bucket
                        .id
                        .as_deref()
                        .is_some_and(|id| id.ends_with(&format!("-{window}"))))
    })?;
    let remaining = bucket.remaining_fraction?.clamp(0.0, 1.0);
    Some(Window {
        window_duration_mins: Some(minutes),
        label: None,
        utilization: Some(((1.0 - remaining) * 100.0) as f32),
        resets_at: bucket.reset_time.clone(),
    })
}

/// Pure: what a failed report's `response` means.
fn failure(response: &str) -> QuotaError {
    let response = response.to_lowercase();
    let signed_out = [
        "sign in",
        "sign-in",
        "log in",
        "login",
        "authenticat",
        "credential",
    ]
    .iter()
    .any(|hint| response.contains(hint));
    if signed_out {
        QuotaError::NoCredentials
    } else {
        QuotaError::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape agy 1.4 prints, as CodexBar's and cc-switch's fixtures of it
    /// have it (values changed), with the third group agy 1.4.0 added.
    const REPORT: &str = r#"{
      "status": "SUCCESS",
      "conversation_id": "",
      "response": "Models share limits",
      "duration_seconds": 0,
      "num_turns": 0,
      "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0},
      "command": {"name": "usage", "data": {
        "description": "Models share limits",
        "groups": [
          {"name": "Gemini Models", "buckets": [
            {"id": "gemini-weekly", "name": "Weekly Limit Remaining", "window": "weekly",
             "remaining_fraction": 0.86, "reset_time": "2026-10-24T18:40:27Z"},
            {"id": "gemini-5h", "name": "Five Hour Limit Remaining", "window": "5h",
             "remaining_fraction": 0.25, "reset_time": "2026-10-22T03:10:00Z"}]},
          {"name": "Claude and GPT models", "buckets": [
            {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 1.0,
             "reset_time": "2026-10-24T18:40:27Z"},
            {"id": "3p-5h", "window": "5h", "disabled": true}]},
          {"name": "Gemini 4 Argon", "buckets": [
            {"id": "gemini-argon-wtu-weekly", "window": "weekly", "remaining_fraction": 0.5,
             "reset_time": "2026-10-25T00:00:00Z"}]}
        ]}}
    }"#;

    #[test]
    fn gemini_is_session_and_weekly_and_the_other_groups_are_limits_of_their_own() {
        let usage = parse(REPORT).unwrap();
        let session = usage.primary.unwrap();
        assert_eq!(session.utilization, Some(75.0));
        assert_eq!(session.window_duration_mins, Some(300));
        assert_eq!(session.resets_at.as_deref(), Some("2026-10-22T03:10:00Z"));
        let weekly = usage.secondary.unwrap();
        assert!((weekly.utilization.unwrap() - 14.0).abs() < 0.01);
        assert_eq!(weekly.window_duration_mins, Some(10080));

        let names: Vec<&str> = usage
            .additional_limits
            .iter()
            .map(|limit| limit.name.as_str())
            .collect();
        assert_eq!(names, ["Claude and GPT models", "Gemini 4 Argon"]);
        let third_party = &usage.additional_limits[0];
        // Disabled: unknown, not used up.
        assert!(third_party.primary.is_none());
        assert_eq!(
            third_party.secondary.as_ref().unwrap().utilization,
            Some(0.0)
        );
        let argon = &usage.additional_limits[1];
        assert!(argon.primary.is_none());
        assert_eq!(argon.secondary.as_ref().unwrap().utilization, Some(50.0));
    }

    /// agy's local server spells the same report in camelCase.
    #[test]
    fn camel_case_fields_read_the_same() {
        let usage = parse(
            r#"{"status":"SUCCESS","command":{"data":{"groups":[{"displayName":"Gemini Models",
              "buckets":[{"bucketId":"gemini-5h","window":"5h","remainingFraction":0.6,
              "resetTime":"2026-10-22T03:10:00Z"}]}]}}}"#,
        )
        .unwrap();
        assert!((usage.primary.unwrap().utilization.unwrap() - 40.0).abs() < 0.01);
        assert!(usage.secondary.is_none());
    }

    #[test]
    fn log_lines_ahead_of_the_report_are_skipped() {
        let usage = parse(&format!("Starting language server…\n{REPORT}\n")).unwrap();
        assert!(usage.primary.is_some());
    }

    #[test]
    fn a_bucket_that_doesnt_say_what_is_left_is_unknown() {
        let usage = parse(
            r#"{"status":"SUCCESS","command":{"data":{"groups":[{"name":"Gemini Models",
              "buckets":[{"id":"gemini-5h","window":"5h","reset_time":"2026-10-22T03:10:00Z"},
              {"id":"gemini-weekly","window":"weekly","remaining_fraction":0.0}]}]}}}"#,
        )
        .unwrap();
        assert!(usage.primary.is_none());
        assert_eq!(usage.secondary.unwrap().utilization, Some(100.0));
    }

    #[test]
    fn a_signed_out_report_reads_as_no_credentials() {
        let answer =
            parse(r#"{"status":"ERROR","response":"You are not signed in. Run agy and sign in."}"#);
        assert!(matches!(answer, Err(QuotaError::NoCredentials)));
    }

    #[test]
    fn anything_else_is_unknown() {
        assert!(matches!(parse(""), Err(QuotaError::Unknown)));
        assert!(matches!(parse("not json"), Err(QuotaError::Unknown)));
        assert!(matches!(
            parse(r#"{"status":"ERROR","response":"Internal error"}"#),
            Err(QuotaError::Unknown)
        ));
        // A success with no bucket anyone can read says nothing.
        assert!(matches!(
            parse(r#"{"status":"SUCCESS","command":{"data":{"groups":[]}}}"#),
            Err(QuotaError::Unknown)
        ));
    }

    #[test]
    fn a_profile_is_signed_in_once_its_token_file_is_there() {
        let home = tempfile::tempdir().unwrap();
        assert!(!signed_in(home.path()));
        let dir = home.path().join(".gemini").join("antigravity-cli");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(TOKEN_FILE), "{}").unwrap();
        assert!(signed_in(home.path()));
    }
}
