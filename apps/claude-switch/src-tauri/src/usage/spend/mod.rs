//! Local spend and usage history: what each profile's Claude Code and Codex
//! sessions cost, from the logs they leave on this Mac, priced at public API
//! rates. Nothing here goes over the network but the price lists
//! ([`pricing::store`]), which carry no usage.
//!
//! One scan reads every profile's logs ([`scan_all`]); only one runs at a
//! time, and the menu bar waits a few seconds for it before showing what it
//! has, the rest arriving when the scan finishes ([`start_scan`]). That and
//! the reading rules follow OpenUsage (MIT,
//! <https://github.com/robinebers/openusage>).

pub mod claude;
pub mod codex;
pub mod daily;
pub mod jsonl;
pub mod pricing;
pub mod tiles;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Local, TimeZone};
use tokio::sync::watch;

use crate::app_kind::AppKind;
use crate::usage::metrics::{id, Metric};
use crate::{app_state, paths, sessions};
use daily::UsageHistory;
use jsonl::ScanCache;
use pricing::{FallbackOption, CODEX_PROVIDER};

/// Every home's usage history, by home id: a profile's id, or
/// `default:<app>` for an app's stock install.
pub type Histories = HashMap<String, UsageHistory>;

static CLAUDE_CACHE: LazyLock<ScanCache<claude::ClaudeParser>> =
    LazyLock::new(|| ScanCache::new(cache_dir("claude"), claude::SCHEMA));
static CODEX_CACHE: LazyLock<ScanCache<codex::CodexParser>> =
    LazyLock::new(|| ScanCache::new(cache_dir("codex"), codex::SCHEMA));

/// `<app data>/spend-cache/<name>/`.
fn cache_dir(name: &str) -> Option<PathBuf> {
    paths::app_data_dir()
        .ok()
        .map(|dir| dir.join("spend-cache").join(name))
}

/// Reads every Claude and Codex home's logs as of `now`. Blocking: it walks
/// and parses files, the first time all of a month's.
pub fn scan_all<Tz: TimeZone>(now: &DateTime<Tz>) -> Histories {
    let pricing = pricing::store::current();
    let fallback = app_state::load()
        .ok()
        .and_then(|state| state.settings.advanced.codex_fallback_model);
    let mut histories = Histories::new();
    match sessions::homes_of(AppKind::Claude) {
        Ok(homes) => {
            let owners = sessions::transcript_owners(&homes);
            histories.extend(claude::scan(&homes, &owners, now, &pricing, &CLAUDE_CACHE));
        }
        Err(err) => eprintln!(
            "claude-switch: could not list the Claude profiles' logs: {}",
            err.message()
        ),
    }
    match sessions::homes_of(AppKind::Codex) {
        Ok(homes) => histories.extend(codex::scan(
            &homes,
            now,
            &pricing,
            fallback.as_deref(),
            &CODEX_CACHE,
        )),
        Err(err) => eprintln!(
            "claude-switch: could not list the Codex profiles' logs: {}",
            err.message()
        ),
    }
    histories
}

/// The metrics of home `home_id`'s history, as of `now`.
pub fn metrics_for<Tz: TimeZone>(
    histories: &Histories,
    home_id: &str,
    now: &DateTime<Tz>,
) -> Vec<Metric> {
    histories
        .get(home_id)
        .map(|history| tiles::metrics(history, now))
        .unwrap_or_default()
}

/// Whether metric `metric_id` comes from the local logs.
pub fn is_spend_metric(metric_id: &str) -> bool {
    [id::TODAY, id::YESTERDAY, id::LAST_30_DAYS, id::TREND].contains(&metric_id)
}

type Outcome = Option<Arc<Histories>>;

/// The scan running, if one is: a scan can outlast a refresh, and two never
/// run at once.
static IN_FLIGHT: Mutex<Option<watch::Receiver<Outcome>>> = Mutex::new(None);

/// Clears [`IN_FLIGHT`] when the scan ends, panicking or not, so the next
/// refresh starts another.
struct EndsScan;

impl Drop for EndsScan {
    fn drop(&mut self) {
        IN_FLIGHT
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

/// A scan's result, when it comes.
pub struct Pending(watch::Receiver<Outcome>);

impl Pending {
    /// The histories, if the scan finishes within `limit`.
    pub async fn within(&mut self, limit: Duration) -> Option<Arc<Histories>> {
        tokio::time::timeout(limit, self.0.wait_for(Option::is_some))
            .await
            .ok()?
            .ok()?
            .clone()
    }

    /// The histories, however long the scan takes; `None` if it failed.
    pub async fn finished(mut self) -> Option<Arc<Histories>> {
        self.0.wait_for(Option::is_some).await.ok()?.clone()
    }
}

/// Starts a scan of every home's logs, unless one is running, and hands
/// back its result to wait on.
pub fn start_scan() -> Pending {
    let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(running) = in_flight.as_ref() {
        return Pending(running.clone());
    }
    let (sender, receiver) = watch::channel(None);
    *in_flight = Some(receiver.clone());
    drop(in_flight);
    tauri::async_runtime::spawn_blocking(move || {
        let ends = EndsScan;
        let histories = Arc::new(scan_all(&Local::now()));
        // Sent before the slot clears, so a refresh in between gets it.
        let _ = sender.send(Some(histories));
        drop(ends);
    });
    Pending(receiver)
}

/// The models offered to price Codex usage that has no price of its own.
#[tauri::command]
pub fn codex_fallback_models() -> Vec<FallbackOption> {
    pricing::store::current().fallback_options(CODEX_PROVIDER)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Scans real homes and prints their daily totals, to compare with
    /// other tools. `CLAUDE_SWITCH_SPEND_HOMES` lists them as
    /// `<app>|<id>|<config dir>|<gui data dir>`, `;`-separated, and
    /// `CLAUDE_SWITCH_SPEND_CACHE` names a folder for the parse cache. Run:
    /// `cargo test --lib real_logs -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this Mac's own session logs"]
    fn real_logs() {
        let homes: Vec<sessions::Home> = std::env::var("CLAUDE_SWITCH_SPEND_HOMES")
            .expect("CLAUDE_SWITCH_SPEND_HOMES")
            .split(';')
            .map(|spec| {
                let parts: Vec<&str> = spec.splitn(4, '|').collect();
                sessions::Home {
                    id: parts[1].to_owned(),
                    app: if parts[0] == "codex" {
                        AppKind::Codex
                    } else {
                        AppKind::Claude
                    },
                    label: parts[1].to_owned(),
                    config_dir: PathBuf::from(parts[2]),
                    gui_data_dir: PathBuf::from(parts[3]),
                    stock: parts[1].starts_with("default:"),
                    desktop_reads_config_dir: true,
                }
            })
            .collect();
        let cache_root = PathBuf::from(
            std::env::var("CLAUDE_SWITCH_SPEND_CACHE").expect("CLAUDE_SWITCH_SPEND_CACHE"),
        );
        let claude_cache = ScanCache::new(Some(cache_root.join("claude")), claude::SCHEMA);
        let codex_cache = ScanCache::new(Some(cache_root.join("codex")), codex::SCHEMA);
        let pricing = pricing::store::PricingStore::new(cache_root.join("pricing")).current();
        let claude_homes: Vec<sessions::Home> = homes
            .iter()
            .filter(|home| home.app == AppKind::Claude)
            .cloned()
            .collect();
        let codex_homes: Vec<sessions::Home> = homes
            .iter()
            .filter(|home| home.app == AppKind::Codex)
            .cloned()
            .collect();
        let now = Local::now();
        for pass in 1..=2 {
            let started = Instant::now();
            let owners = sessions::transcript_owners(&claude_homes);
            let mut histories = claude::scan(&claude_homes, &owners, &now, &pricing, &claude_cache);
            histories.extend(codex::scan(
                &codex_homes,
                &now,
                &pricing,
                None,
                &codex_cache,
            ));
            println!("pass {pass}: {:?}", started.elapsed());
            if pass == 2 {
                for (home, history) in &histories {
                    for (day, usage) in &history.days {
                        let models: Vec<String> = usage
                            .models
                            .iter()
                            .map(|(model, used)| {
                                format!("{model}={}/{:.2}", used.tokens, used.cost)
                            })
                            .collect();
                        println!(
                            "{home}\t{day}\t{}\t{:.2}\t{}",
                            usage.tokens,
                            usage.cost,
                            models.join(" ")
                        );
                    }
                    for (day, models) in &history.unknown_models {
                        println!("{home}\t{day}\tunknown\t{models:?}");
                    }
                }
            }
        }
    }
}
