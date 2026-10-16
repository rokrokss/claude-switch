//! `claude-switch usage [profile] [--force]`: the profiles' usage limits as
//! the local API's `/v1/limits` JSON, for scripts and agents. One shot: it
//! never starts the app, and never renews a sign-in, which only the app does.
//!
//! Figures the menu bar (or an earlier run) fetched in the last five minutes
//! are reused from its cache; the rest are fetched, one profile at a time, and
//! saved back to it. A Claude profile the usage endpoint rate limited isn't
//! asked again, even with `--force`, until its cooldown has run
//! ([`backoff`]). The command follows OpenUsage's
//! (MIT, <https://github.com/robinebers/openusage>).

use std::collections::HashMap;
use std::io::Write;

use chrono::{DateTime, Local, TimeDelta, Utc};

use crate::app_kind::AppKind;
use crate::cli::{UsageArgs, HELP};
use crate::local_api::limits;
use crate::tray::{self, cache, Target, TrayEntry};
use crate::usage::{metrics, quota, QuotaError};
use crate::{commands, profiles};

mod backoff;

/// The exit status of a misuse, or of a profile that doesn't exist.
const EXIT_MISUSE: i32 = 2;
/// The exit status when a fetch failed or the profiles couldn't be read.
const EXIT_FAILED: i32 = 4;

/// Prints the limits `args` asks for; the process's exit status.
pub fn run(args: &UsageArgs) -> i32 {
    if args.help {
        print!("{HELP}");
        return 0;
    }
    let (targets, all) = match tray::targets().and_then(|targets| Ok((targets, profiles::load()?)))
    {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!(
                "claude-switch: could not read the profiles: {}",
                err.message()
            );
            return EXIT_FAILED;
        }
    };
    let slugs: HashMap<String, String> = all
        .into_iter()
        .map(|profile| (profile.id, profile.slug))
        .collect();
    let chosen: Vec<&Target> = match &args.profile {
        None => targets.iter().collect(),
        Some(token) => {
            let candidates: Vec<Candidate> = targets
                .iter()
                .map(|target| Candidate {
                    id: &target.id,
                    app: target.app,
                    name: &target.name,
                    slug: slugs.get(&target.id).map(String::as_str),
                })
                .collect();
            let ids = matching(&candidates, token);
            if ids.is_empty() {
                eprintln!(
                    "claude-switch: no profile is called {token}\nRun `claude-switch usage --help` for usage."
                );
                return EXIT_MISUSE;
            }
            targets
                .iter()
                .filter(|target| ids.contains(&target.id.as_str()))
                .collect()
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("claude-switch: {err}");
            return EXIT_FAILED;
        }
    };

    let cached = cache::load();
    let holds = backoff::load();
    let now = Utc::now();
    let mut entries = Vec::with_capacity(chosen.len());
    let mut fetched = Vec::new();
    let mut fetched_holds = Vec::new();
    let mut held = Vec::new();
    // One at a time, as the menu bar does: fetched together, profiles have
    // tripped the usage endpoint's rate limit.
    for target in chosen {
        let last = cached.iter().find(|entry| entry.id == target.id);
        match step(args.force, last, backoff::until(&holds, &target.id, now)) {
            Step::Reuse(recent) => entries.push(recent.clone()),
            Step::Hold(until) => {
                let entry = rate_limited(target, last);
                held.push((entry.clone(), until));
                entries.push(entry);
            }
            Step::Fetch => {
                let entry = runtime.block_on(fetch(target, last));
                let until = is_tripped(&entry).then(|| hold_until(&target.id));
                fetched_holds.push((entry.id.clone(), until));
                fetched.push(entry.clone());
                entries.push(entry);
            }
        }
    }
    if !fetched.is_empty() {
        save(&fetched);
        save_holds(&fetched_holds);
    }

    let body = limits::limits(&entries, Utc::now());
    // A closed pipe (`| head`) is no reason to fail.
    let _ = writeln!(std::io::stdout().lock(), "{body}");
    let mut failed = !held.is_empty();
    for entry in &fetched {
        if let Some(error) = entry.error {
            failed = true;
            eprintln!(
                "claude-switch: warning: {}: {}",
                entry.name,
                limits::describe(error, &entry.command)
            );
        }
    }
    for (entry, until) in &held {
        eprintln!(
            "claude-switch: warning: {}: {} (not asked again before {})",
            entry.name,
            limits::describe(QuotaError::RateLimited, &entry.command),
            until.with_timezone(&Local).format("%H:%M:%S")
        );
    }
    if failed {
        EXIT_FAILED
    } else {
        0
    }
}

/// A profile as the command line can name it.
struct Candidate<'a> {
    id: &'a str,
    app: AppKind,
    name: &'a str,
    /// None for a stock install.
    slug: Option<&'a str>,
}

/// Pure: the ids of the `candidates` that `token` names, in their order.
/// An app (`claude`, `codex`) names every profile of it; anything else names
/// the profiles whose id, slug or name it is. Case doesn't matter.
fn matching<'a>(candidates: &[Candidate<'a>], token: &str) -> Vec<&'a str> {
    let token = token.trim().to_lowercase();
    let app = AppKind::from_token(&token);
    candidates
        .iter()
        .filter(|candidate| match app {
            Some(app) => candidate.app == app,
            None => {
                candidate.id.to_lowercase() == token
                    || candidate.slug == Some(token.as_str())
                    || candidate.name.to_lowercase() == token
            }
        })
        .map(|candidate| candidate.id)
        .collect()
}

/// Whether `entry`'s figures are recent enough to print as they are.
fn is_recent(entry: &TrayEntry) -> bool {
    limits::fetched_at(entry).is_some_and(|at| Utc::now() - at < limits::FRESH_FOR)
}

/// What a run does for a profile.
enum Step<'a> {
    /// Print its cached entry as it is.
    Reuse(&'a TrayEntry),
    /// Print its cached entry as rate limited, without asking: it may be
    /// asked again at this time.
    Hold(DateTime<Utc>),
    Fetch,
}

/// Pure: a profile held back `until` a rate limit's cooldown has run isn't
/// asked, even when `force`d, and reads as rate limited all that time; the
/// rest reuse their `last` entry while recent, unless `force`d.
fn step(force: bool, last: Option<&TrayEntry>, until: Option<DateTime<Utc>>) -> Step<'_> {
    match (last, until) {
        (_, Some(until)) => Step::Hold(until),
        (Some(recent), None) if !force && is_recent(recent) => Step::Reuse(recent),
        _ => Step::Fetch,
    }
}

/// `target`'s `last` entry, or a bare one, marked rate limited: what a
/// profile held back reads as.
fn rate_limited(target: &Target, last: Option<&TrayEntry>) -> TrayEntry {
    let mut entry = last.cloned().unwrap_or_else(|| TrayEntry {
        id: target.id.clone(),
        app: target.app,
        name: target.name.clone(),
        command: target.command.clone(),
        plan: None,
        usage: None,
        metrics: Vec::new(),
        error: None,
    });
    entry.error = Some(QuotaError::RateLimited);
    entry
}

/// Pure: whether fetching `entry` tripped Claude's usage endpoint's rate
/// limit. A Codex profile reads as rate limited when its quota is spent,
/// which asking again doesn't make worse.
fn is_tripped(entry: &TrayEntry) -> bool {
    entry.app == AppKind::Claude && matches!(entry.error, Some(QuotaError::RateLimited))
}

/// When a Claude profile just rate limited may be asked again: once the
/// cooldown its `Retry-After` set has run, or the default one.
fn hold_until(profile_id: &str) -> DateTime<Utc> {
    let cooldown =
        commands::rate_limit_remaining(profile_id).unwrap_or(quota::DEFAULT_RATE_LIMIT_COOLDOWN);
    Utc::now() + TimeDelta::from_std(cooldown).unwrap_or_default()
}

/// `target`'s entry, fetched now, with its stored sign-in as it is. A fetch
/// that fails keeps `last`'s figures, as the menu bar does.
async fn fetch(target: &Target, last: Option<&TrayEntry>) -> TrayEntry {
    let fetched = tokio::time::timeout(
        tray::FETCH_DEADLINE,
        commands::stored_profile_usage(&target.id),
    )
    .await;
    let last_usage = last.and_then(|entry| entry.usage.clone());
    let (usage, error) = match fetched {
        Ok(Ok(usage)) if usage.quota.is_some() => (Some(usage), None),
        Ok(Ok(usage)) => (
            last_usage,
            Some(usage.quota_error.unwrap_or(QuotaError::Unknown)),
        ),
        Ok(Err(_)) => (last_usage, Some(QuotaError::Unknown)),
        Err(_) => (last_usage, Some(QuotaError::Network)),
    };
    let metrics = usage
        .as_ref()
        .map(|usage| metrics::from_usage(target.app, usage))
        .unwrap_or_default();
    let plan =
        commands::profile_plan(&target.id).or_else(|| last.and_then(|entry| entry.plan.clone()));
    TrayEntry {
        id: target.id.clone(),
        app: target.app,
        name: target.name.clone(),
        command: target.command.clone(),
        plan,
        usage,
        metrics,
        error,
    }
}

/// Saves `fetched` into the cache, read again first: the app may have saved
/// since. An entry there with newer figures than one of these is kept.
fn save(fetched: &[TrayEntry]) {
    let mut entries = cache::load();
    merge(&mut entries, fetched);
    if let Err(err) = cache::save(&entries) {
        eprintln!(
            "claude-switch: could not save the figures fetched: {}",
            err.message()
        );
    }
}

/// Records this run's rate limits in the back-off file, read again first: an
/// overlapping run may have saved since.
fn save_holds(fetched: &[(String, Option<DateTime<Utc>>)]) {
    let mut holds = backoff::load();
    if !backoff::record(&mut holds, fetched, Utc::now()) {
        return;
    }
    if let Err(err) = backoff::save(&holds) {
        eprintln!(
            "claude-switch: could not save when to ask again: {}",
            err.message()
        );
    }
}

/// Pure: `fetched` merged into `entries` by id.
fn merge(entries: &mut Vec<TrayEntry>, fetched: &[TrayEntry]) {
    for entry in fetched {
        match entries.iter_mut().find(|saved| saved.id == entry.id) {
            Some(saved) => {
                if limits::fetched_at(saved) <= limits::fetched_at(entry) {
                    *saved = entry.clone();
                }
            }
            None => entries.push(entry.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_api::limits::tests::entry;

    fn candidates() -> Vec<Candidate<'static>> {
        vec![
            Candidate {
                id: "default:claude",
                app: AppKind::Claude,
                name: "Claude",
                slug: None,
            },
            Candidate {
                id: "1b4e28ba-2fa1-11d2-883f-0016d3cca427",
                app: AppKind::Claude,
                name: "My Work",
                slug: Some("my-work"),
            },
            Candidate {
                id: "default:codex",
                app: AppKind::Codex,
                name: "ChatGPT",
                slug: None,
            },
            Candidate {
                id: "6fa459ea-ee8a-3ca4-894e-db77e160355e",
                app: AppKind::Codex,
                name: "home",
                slug: Some("personal"),
            },
        ]
    }

    #[test]
    fn an_app_names_every_profile_of_it() {
        assert_eq!(
            matching(&candidates(), "claude"),
            ["default:claude", "1b4e28ba-2fa1-11d2-883f-0016d3cca427"]
        );
        assert_eq!(
            matching(&candidates(), "Codex"),
            ["default:codex", "6fa459ea-ee8a-3ca4-894e-db77e160355e"]
        );
    }

    #[test]
    fn a_profile_is_named_by_its_id_slug_or_name() {
        let work = ["1b4e28ba-2fa1-11d2-883f-0016d3cca427"];
        assert_eq!(
            matching(&candidates(), "1b4e28ba-2fa1-11d2-883f-0016d3cca427"),
            work
        );
        assert_eq!(matching(&candidates(), "my-work"), work);
        assert_eq!(matching(&candidates(), "my work"), work);
        assert_eq!(matching(&candidates(), " MY WORK "), work);
        assert_eq!(
            matching(&candidates(), "default:claude"),
            ["default:claude"]
        );
        assert_eq!(matching(&candidates(), "chatgpt"), ["default:codex"]);
        assert_eq!(
            matching(&candidates(), "personal"),
            ["6fa459ea-ee8a-3ca4-894e-db77e160355e"]
        );
    }

    #[test]
    fn a_token_naming_nothing_names_nothing() {
        assert!(matching(&candidates(), "nobody").is_empty());
        assert!(matching(&candidates(), "").is_empty());
        assert!(matching(&[], "claude").is_empty());
    }

    #[test]
    fn figures_younger_than_five_minutes_are_recent() {
        let at = |age: chrono::TimeDelta| (Utc::now() - age).to_rfc3339();
        let recent = entry(
            "r",
            AppKind::Claude,
            Some(&at(chrono::TimeDelta::minutes(4))),
        );
        let old = entry(
            "o",
            AppKind::Claude,
            Some(&at(chrono::TimeDelta::minutes(6))),
        );
        let never = entry("n", AppKind::Claude, None);
        assert!(is_recent(&recent));
        assert!(!is_recent(&old));
        assert!(!is_recent(&never));
    }

    #[test]
    fn a_held_profile_is_not_asked_even_when_forced() {
        let now = Utc::now();
        let until = Some(now + chrono::TimeDelta::minutes(30));
        let recent = entry(
            "r",
            AppKind::Claude,
            Some(&(now - chrono::TimeDelta::minutes(1)).to_rfc3339()),
        );
        let old = entry(
            "o",
            AppKind::Claude,
            Some(&(now - chrono::TimeDelta::minutes(30)).to_rfc3339()),
        );
        // A held profile isn't asked, forced or not, its figures recent,
        // old or missing.
        assert!(matches!(step(true, Some(&recent), until), Step::Hold(at) if Some(at) == until));
        assert!(matches!(step(false, Some(&recent), until), Step::Hold(_)));
        assert!(matches!(step(false, Some(&old), until), Step::Hold(_)));
        assert!(matches!(step(true, None, until), Step::Hold(_)));
        // The rest reuse recent figures unless forced, and are asked otherwise.
        assert!(matches!(step(false, Some(&recent), None), Step::Reuse(_)));
        assert!(matches!(step(true, Some(&recent), None), Step::Fetch));
        assert!(matches!(step(false, Some(&old), None), Step::Fetch));
        assert!(matches!(step(false, None, None), Step::Fetch));
    }

    #[test]
    fn a_held_profile_prints_its_cached_figures_as_rate_limited() {
        let target = Target {
            id: "p1".into(),
            app: AppKind::Claude,
            name: "Work".into(),
            command: "claude-work".into(),
        };
        let last = entry("p1", AppKind::Claude, Some("2026-10-17T12:00:00Z"));
        let held = rate_limited(&target, Some(&last));
        assert!(matches!(held.error, Some(QuotaError::RateLimited)));
        assert_eq!(held.metrics, last.metrics);
        assert_eq!(
            held.usage.unwrap().fetched_at,
            last.usage.unwrap().fetched_at
        );

        let bare = rate_limited(&target, None);
        assert!(matches!(bare.error, Some(QuotaError::RateLimited)));
        assert_eq!(bare.name, "Work");
        assert!(bare.usage.is_none());
        let body: serde_json::Value =
            serde_json::from_str(&limits::limits(&[bare], Utc::now())).unwrap();
        assert_eq!(body["errors"][0]["code"], "rate_limited");
    }

    #[test]
    fn only_claudes_rate_limit_holds_a_profile_back() {
        let tripped = |app, error| {
            let mut fetched = entry("p", app, None);
            fetched.error = error;
            is_tripped(&fetched)
        };
        assert!(tripped(AppKind::Claude, Some(QuotaError::RateLimited)));
        // Codex reads as rate limited when its quota is spent.
        assert!(!tripped(AppKind::Codex, Some(QuotaError::RateLimited)));
        assert!(!tripped(AppKind::Claude, Some(QuotaError::Network)));
        assert!(!tripped(AppKind::Claude, None));
    }

    #[test]
    fn fetched_entries_replace_older_ones_and_join_the_rest() {
        let mut entries = vec![
            entry("a", AppKind::Claude, Some("2026-10-17T12:00:00Z")),
            entry("b", AppKind::Claude, Some("2026-10-17T12:10:00Z")),
        ];
        let mut failed = entry("b", AppKind::Claude, Some("2026-10-17T12:00:00Z"));
        failed.error = Some(QuotaError::Network);
        let fetched = [
            entry("a", AppKind::Claude, Some("2026-10-17T12:05:00Z")),
            failed,
            entry("c", AppKind::Codex, Some("2026-10-17T12:05:00Z")),
        ];
        merge(&mut entries, &fetched);
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(
            entries[0].usage.as_ref().unwrap().fetched_at,
            "2026-10-17T12:05:00Z"
        );
        // The app fetched b since: its newer figures stay.
        assert_eq!(
            entries[1].usage.as_ref().unwrap().fetched_at,
            "2026-10-17T12:10:00Z"
        );
        assert!(entries[1].error.is_none());
    }
}
