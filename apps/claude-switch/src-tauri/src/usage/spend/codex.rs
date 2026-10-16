//! What each Codex profile's sessions cost, from the rollouts Codex keeps in
//! its home's `sessions/` and `archived_sessions/`.
//!
//! The rules are ccusage's, as OpenUsage's `CodexLogUsageScanner`,
//! `CodexLogFileParser` and `CodexUsagePricing` port them (MIT,
//! <https://github.com/robinebers/openusage>):
//! - A `turn_context` line sets the session's model; an `event_msg` of type
//!   `token_count` carries a turn's usage: its `last_token_usage`, else the
//!   difference between this and the last cumulative `total_token_usage`.
//!   A cumulative total that didn't change is a repeat, not a turn.
//! - A child session (a subagent, a fork) first replays its parent's
//!   history; those lines only set the baseline, until a `task_started` at
//!   or after the child's own creation (or, without that, at or after its
//!   own line's time) opens the session's own turns.
//! - `thread_settings_applied` sets the service tier: `fast` or `priority`
//!   bill at the model's priority multiplier, `ultrafast` at 6x for GPT-6
//!   Astra (its fast multiplier for other models).
//! - A session without model metadata is `gpt-5`. `codex-auto-review`
//!   keeps its name, is priced at the model it ran on before October 6,
//!   2026 (UTC) and free from then; `gpt-reserve` is priced as
//!   `gpt-5.6-luna`.
//! - Supported GPT-5.4 to GPT-6 models bill a request above 272k input
//!   tokens at their long-context rates; cached input bills at the input
//!   rate where no cache discount is published.
//! - An event logged the same in several files (copied rollouts) counts
//!   once, and of a home's `sessions/` and `archived_sessions/` holding the
//!   same relative path, the active copy counts.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::daily::{self, UsageHistory};
use super::jsonl::{self, LogFile, Parser, ScanCache};
use super::pricing::{ModelPricing, ModelRates, TokenBreakdown, CODEX_PROVIDER};
use crate::sessions::Home;

/// Bumped when what [`CodexParser`] yields changes.
pub const SCHEMA: u32 = 1;

const AUTO_REVIEW: &str = "codex-auto-review";
/// When auto-review became free for ChatGPT sign-ins: the start of the day
/// announced, in UTC, as no time was given.
const AUTO_REVIEW_FREE_SINCE: &str = "2026-10-06T00:00:00Z";
/// The model auto-review ran on before then, by the day each took over.
const AUTO_REVIEW_MODELS: [(&str, &str); 8] = [
    ("2026-07-09", "gpt-5.6-luna"),
    ("2026-04-23", "gpt-5.5"),
    ("2026-03-05", "gpt-5.4"),
    ("2026-02-05", "gpt-5.3-codex"),
    ("2025-12-11", "gpt-5.2-codex"),
    ("2025-11-13", "gpt-5.1-codex"),
    ("2025-09-15", "gpt-5-codex"),
    ("2025-08-07", "gpt-5"),
];
/// Luna Reserve keeps its name in the breakdown, priced as Luna.
const RESERVE: &str = "gpt-reserve";
const RESERVE_PRICED_AS: &str = "gpt-5.6-luna";
/// A session with no model metadata.
const DEFAULT_MODEL: &str = "gpt-5";

/// One turn's tokens, the difference to the last cumulative total already
/// taken. `cached` is part of `input`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Milliseconds since the epoch.
    #[serde(rename = "t")]
    pub timestamp: i64,
    #[serde(rename = "m")]
    pub model: String,
    /// The model it is priced as, when not its own.
    #[serde(rename = "p", default, skip_serializing_if = "Option::is_none")]
    pub pricing_model: Option<String>,
    #[serde(rename = "i")]
    pub input: u64,
    #[serde(rename = "c")]
    pub cached: u64,
    #[serde(rename = "o")]
    pub output: u64,
    #[serde(rename = "r")]
    pub reasoning: u64,
    #[serde(rename = "n")]
    pub total: u64,
    #[serde(rename = "f", default, skip_serializing_if = "is_false")]
    pub is_fast: bool,
    #[serde(rename = "u", default, skip_serializing_if = "is_false")]
    pub is_ultrafast: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The token counts of a `token_count` usage object, taking the older
/// spellings ccusage takes too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawUsage {
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
    total: u64,
}

impl RawUsage {
    fn from_json(json: &Map<String, Value>) -> Self {
        let int = |keys: &[&str]| keys.iter().find_map(|key| json.get(*key).and_then(number));
        let input = int(&["input_tokens", "prompt_tokens", "input"]).unwrap_or(0);
        let cached = int(&[
            "cached_input_tokens",
            "cache_read_input_tokens",
            "cached_tokens",
        ])
        .unwrap_or(0);
        let output = int(&["output_tokens", "completion_tokens", "output"]).unwrap_or(0);
        let reasoning = int(&["reasoning_output_tokens", "reasoning_tokens"]).unwrap_or(0);
        let reported = int(&["total_tokens"]).unwrap_or(0);
        let recomputed = input + output + reasoning;
        Self {
            input,
            cached,
            output,
            reasoning,
            total: if reported > 0 || recomputed == 0 {
                reported
            } else {
                recomputed
            },
        }
    }

    /// A turn's usage from cumulative totals.
    fn minus(self, previous: Option<RawUsage>) -> Self {
        let previous = previous.unwrap_or_default();
        Self {
            input: self.input.saturating_sub(previous.input),
            cached: self.cached.saturating_sub(previous.cached),
            output: self.output.saturating_sub(previous.output),
            reasoning: self.reasoning.saturating_sub(previous.reasoning),
            total: self.total.saturating_sub(previous.total),
        }
    }
}

fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|count| count.max(0) as u64))
        .or_else(|| value.as_f64().map(|count| count.max(0.0) as u64))
}

/// Until when a child session's lines replay its parent's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum ReplayGate {
    /// Until a turn starts at or after this many seconds since the epoch:
    /// the child's own creation.
    UntilStartedAt(f64),
    /// Without a creation time, until a turn starts at or after its own
    /// line's time.
    UntilSelfTimed,
}

impl ReplayGate {
    fn opened_by(&self, started_at: f64, line_timestamp: Option<&str>) -> bool {
        match self {
            ReplayGate::UntilStartedAt(gate) => started_at >= *gate,
            ReplayGate::UntilSelfTimed => line_timestamp
                .and_then(|raw| daily::timestamp_millis(raw.trim()))
                .is_some_and(|millis| started_at >= whole_seconds(millis)),
        }
    }
}

fn whole_seconds(millis: i64) -> f64 {
    millis.div_euclid(1000) as f64
}

/// A rollout's context so far: kept per file, across reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexParser {
    previous_totals: Option<RawUsage>,
    current_model: Option<String>,
    fast: bool,
    ultrafast: bool,
    saw_session_meta: bool,
    replay_gate: Option<ReplayGate>,
}

const TURN_CONTEXT: &[u8] = br#""type":"turn_context""#;
const TOKEN_COUNT: &[u8] = br#""type":"token_count""#;
const SESSION_META: &[u8] = br#""type":"session_meta""#;
const TASK_STARTED: &[u8] = br#""type":"task_started""#;
const THREAD_SETTINGS: &[u8] = br#""type":"thread_settings_applied""#;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

impl Parser for CodexParser {
    type Item = Event;

    fn line(&mut self, line: &[u8], events: &mut Vec<Event>) {
        let is_turn_context = contains(line, TURN_CONTEXT);
        let is_session_meta = !self.saw_session_meta && contains(line, SESSION_META);
        let is_task_started = self.replay_gate.is_some() && contains(line, TASK_STARTED);
        let is_thread_settings = contains(line, THREAD_SETTINGS);
        if !(is_turn_context
            || is_session_meta
            || is_task_started
            || is_thread_settings
            || contains(line, TOKEN_COUNT))
        {
            return;
        }
        let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let kind = object.get("type").and_then(Value::as_str);
        let payload = object.get("payload").and_then(Value::as_object);
        let line_timestamp = object.get("timestamp").and_then(Value::as_str);

        if kind == Some("turn_context") {
            if let Some(model) = payload.and_then(model_name) {
                self.current_model = Some(model);
            }
            return;
        }
        // A child rollout replays its parent's metadata too; only the first
        // is its own.
        if kind == Some("session_meta") && !self.saw_session_meta {
            self.saw_session_meta = true;
            if payload.is_some_and(is_child_session) {
                self.replay_gate = Some(
                    match line_timestamp.and_then(|raw| daily::timestamp_millis(raw.trim())) {
                        Some(created) => ReplayGate::UntilStartedAt(whole_seconds(created)),
                        None => ReplayGate::UntilSelfTimed,
                    },
                );
            }
            return;
        }
        let payload_kind = payload
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str);
        if is_thread_settings
            && kind == Some("event_msg")
            && payload_kind == Some("thread_settings_applied")
        {
            if let Some(tier) = payload.and_then(service_tier) {
                self.ultrafast = tier == "ultrafast";
                self.fast = tier == "fast" || tier == "priority";
            }
            return;
        }
        let (Some("event_msg"), Some(payload)) = (kind, payload) else {
            return;
        };
        // Replayed starts keep the parent's older time; the first live turn
        // opens the gate.
        if payload_kind == Some("task_started") {
            if let (Some(gate), Some(started_at)) = (
                &self.replay_gate,
                payload.get("started_at").and_then(Value::as_f64),
            ) {
                if gate.opened_by(started_at, line_timestamp) {
                    self.replay_gate = None;
                }
            }
            return;
        }
        if payload_kind != Some("token_count") {
            return;
        }
        let Some(timestamp) = line_timestamp.and_then(|raw| daily::timestamp_millis(raw.trim()))
        else {
            return;
        };
        let info = payload.get("info").and_then(Value::as_object);
        let totals = info
            .and_then(|info| info.get("total_token_usage"))
            .and_then(Value::as_object)
            .map(RawUsage::from_json);
        // The parent's history sets the baseline, never the child's usage.
        if self.replay_gate.is_some() {
            if totals.is_some() {
                self.previous_totals = totals;
            }
            return;
        }
        if totals.is_some() && totals == self.previous_totals {
            return;
        }
        let last = info
            .and_then(|info| info.get("last_token_usage"))
            .and_then(Value::as_object)
            .map(RawUsage::from_json);
        let usage = match (last, totals) {
            (Some(last), _) => last,
            (None, Some(totals)) => totals.minus(self.previous_totals),
            (None, None) => return,
        };
        if totals.is_some() {
            self.previous_totals = totals;
        }
        if usage.input == 0 && usage.cached == 0 && usage.output == 0 && usage.reasoning == 0 {
            return;
        }
        let parsed = model_name(payload).or_else(|| info.and_then(model_name));
        let model = match parsed {
            Some(model) => {
                self.current_model = Some(model.clone());
                model
            }
            None => self
                .current_model
                .get_or_insert_with(|| DEFAULT_MODEL.to_owned())
                .clone(),
        };
        events.push(Event {
            timestamp,
            pricing_model: (model == RESERVE).then(|| RESERVE_PRICED_AS.to_owned()),
            model,
            input: usage.input,
            cached: usage.cached.min(usage.input),
            output: usage.output,
            reasoning: usage.reasoning,
            total: usage.total,
            is_fast: self.fast,
            is_ultrafast: self.ultrafast,
        });
    }
}

fn model_name(json: &Map<String, Value>) -> Option<String> {
    [
        json.get("model"),
        json.get("model_name"),
        json.get("metadata")
            .and_then(|metadata| metadata.get("model")),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|model| !model.is_empty())
    .map(str::to_owned)
}

fn service_tier(payload: &Map<String, Value>) -> Option<&str> {
    [
        payload
            .get("thread_settings")
            .and_then(|settings| settings.get("service_tier")),
        payload.get("service_tier"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|tier| !tier.is_empty())
}

/// Whether a `session_meta` payload is a child session's, whose first
/// turns replay its parent's. `null` and blank text count as absent.
fn is_child_session(payload: &Map<String, Value>) -> bool {
    let set = |value: Option<&Value>| match value {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    };
    set(payload.get("forked_from_id"))
        || set(payload.get("parent_thread_id"))
        || payload.get("thread_source").and_then(Value::as_str) == Some("subagent")
        || payload
            .get("source")
            .and_then(Value::as_object)
            .is_some_and(|source| set(source.get("subagent")))
}

/// How a Codex model's rates were found, before Codex's own rules apply.
struct Resolution {
    rates: Option<ModelRates>,
    /// The base model, without `-fast`, whose name selects those rules.
    rate_model: String,
    is_fast_alias: bool,
    /// The base model has rates of its own, so the tier multiplier applies
    /// to them once.
    has_base_rates: bool,
}

fn millis(iso: &str) -> i64 {
    daily::timestamp_millis(iso).unwrap_or(i64::MAX)
}

fn is_free_auto_review(model: &str, at: i64) -> bool {
    model == AUTO_REVIEW && at >= millis(AUTO_REVIEW_FREE_SINCE)
}

/// The model `model` is priced as at `at`: auto-review's model of the time
/// before it became free, else itself.
fn dated_pricing_model(model: &str, at: i64) -> &str {
    if model != AUTO_REVIEW || at >= millis(AUTO_REVIEW_FREE_SINCE) {
        return model;
    }
    AUTO_REVIEW_MODELS
        .iter()
        .find(|(since, _)| at >= millis(&format!("{since}T00:00:00Z")))
        .map_or(DEFAULT_MODEL, |(_, model)| model)
}

/// Codex's speed is a tier, not a `-fast` price: a fast alias resolves
/// through its base rates so the tier multiplier applies once.
fn resolve(pricing: &ModelPricing, model: &str, at: i64) -> Resolution {
    if is_free_auto_review(model, at) {
        return Resolution {
            rates: Some(ModelRates::new(0.0, 0.0, 0.0, 0.0)),
            rate_model: model.to_owned(),
            is_fast_alias: false,
            has_base_rates: true,
        };
    }
    let effective = dated_pricing_model(model, at);
    let canonical = pricing.canonical_name(effective);
    let (rate_model, is_fast_alias) = match canonical.strip_suffix("-fast") {
        Some(base) => (base.to_owned(), true),
        None => (canonical, false),
    };
    let base = pricing.resolve(&rate_model);
    Resolution {
        has_base_rates: base.is_some(),
        rates: base.or_else(|| pricing.resolve(effective)),
        rate_model,
        is_fast_alias,
    }
}

/// `model` without a `-YYYY-MM-DD` or `-YYYYMMDD` snapshot suffix.
fn dated_base(model: &str) -> &str {
    for shape in ["-dddd-dd-dd", "-dddddddd"] {
        let Some(at) = model.len().checked_sub(shape.len()).filter(|at| *at > 0) else {
            continue;
        };
        if !model.is_char_boundary(at) {
            continue;
        }
        let (base, suffix) = model.split_at(at);
        let matches = suffix.bytes().zip(shape.bytes()).all(|(byte, want)| {
            if want == b'd' {
                byte.is_ascii_digit()
            } else {
                byte == want
            }
        });
        if matches {
            return base;
        }
    }
    model
}

fn long_context_rates(base: &str) -> Option<(f64, f64, f64)> {
    Some(match base {
        "gpt-5.4" => (5.0, 22.5, 0.5),
        "gpt-5.4-pro" => (60.0, 270.0, 60.0),
        "gpt-5.5" => (10.0, 45.0, 1.0),
        "gpt-5.5-pro" => (60.0, 270.0, 60.0),
        "gpt-5.6-sol" => (8.0, 30.0, 0.8),
        "gpt-5.6-terra" => (4.0, 18.0, 0.4),
        "gpt-5.6-luna" => (0.4, 1.8, 0.04),
        // Above 272k: 2x input and cache, 1.5x output.
        "gpt-6-astra" => (20.0, 75.0, 2.0),
        "gpt-6.1-sol" => (4.0, 15.0, 0.2),
        "gpt-6-sol" => (4.0, 15.0, 0.4),
        "gpt-6-luna" => (0.2, 0.75, 0.02),
        _ => return None,
    })
}

fn priority_multiplier(base: &str, rates: &ModelRates) -> f64 {
    match base {
        "gpt-5.5" | "gpt-5.5-pro" => 2.5,
        "gpt-5.4" | "gpt-5.4-pro" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna"
        | "gpt-6-astra" | "gpt-6.1-sol" | "gpt-6-sol" | "gpt-6-luna" => 2.0,
        _ if rates.fast_multiplier == 1.0 => 2.0,
        _ => rates.fast_multiplier,
    }
}

/// `rates` with Codex's rules for `model` applied: its long-context tier,
/// cached input at the input rate without a published discount, and its
/// priority multiplier as the fast one.
fn adjusted(rates: &ModelRates, model: &str) -> ModelRates {
    let base = dated_base(model);
    let mut effective = rates.clone();
    if let Some((input, output, cache_read)) = long_context_rates(base) {
        effective.input_long = Some(input);
        effective.output_long = Some(output);
        effective.cache_read_long = Some(cache_read);
        effective.long_context_threshold = 272_000;
    }
    let no_discount = matches!(base, "gpt-5.4-pro" | "gpt-5.5-pro");
    if no_discount || !rates.cache_read_explicit {
        effective.cache_read = effective.input;
        effective.cache_read_long = effective.input_long;
    }
    effective.fast_multiplier = priority_multiplier(base, rates);
    effective
}

/// What `event` costs at `rates` under Codex's rules for `model`.
fn cost(rates: &ModelRates, event: &Event, model: &str, fast: bool, ultrafast: bool) -> f64 {
    let mut rates = adjusted(rates, model);
    if ultrafast && dated_base(model) == "gpt-6-astra" {
        rates.fast_multiplier = 6.0;
    }
    let tokens = TokenBreakdown {
        input: event.input.saturating_sub(event.cached),
        cache_read: event.cached,
        output: event.output,
        is_fast: fast || ultrafast,
        ..TokenBreakdown::default()
    };
    rates.cost(&tokens, true)
}

/// `events` counted once each, priced and counted by local day from
/// `since` (milliseconds since the epoch). Usage no list prices is left out,
/// unless `fallback` names a model whose rates it may be estimated at; its
/// model is named as unknown either way.
pub fn aggregate<Tz: TimeZone>(
    events: &[Event],
    since: i64,
    zone: &Tz,
    pricing: &ModelPricing,
    fallback: Option<&str>,
) -> UsageHistory {
    let fallback_rates = fallback.and_then(|model| pricing.fallback_rates(model, CODEX_PROVIDER));
    if fallback.is_some() && fallback_rates.is_none() {
        eprintln!("claude-switch: the Codex fallback model has no price; unpriced usage stays out");
    }
    let mut seen = HashSet::new();
    let mut history = UsageHistory::default();
    for event in events.iter().filter(|event| event.timestamp >= since) {
        let key = (
            event.timestamp,
            &event.model,
            &event.pricing_model,
            event.input,
            event.cached,
            event.output,
            event.reasoning,
            event.total,
        );
        if !seen.insert(key) {
            continue;
        }
        let Some(day) = daily::day_of(zone, event.timestamp) else {
            continue;
        };
        let model = event.model.trim();
        if model.is_empty() {
            continue;
        }
        let pricing_model = match &event.pricing_model {
            Some(priced_as) if model != AUTO_REVIEW => priced_as.as_str(),
            _ => model,
        };
        let resolution = resolve(pricing, pricing_model, event.timestamp);
        let mut rate_model = resolution.rate_model;
        let mut rates = resolution.rates;
        let mut fast = if resolution.is_fast_alias {
            resolution.has_base_rates
        } else {
            event.is_fast
        };
        let mut ultrafast =
            event.is_ultrafast && (!resolution.is_fast_alias || resolution.has_base_rates);
        let mut used_fallback = None;
        if rates.is_none() && event.total > 0 {
            history.add_unknown(day, model);
        }
        if rates.is_none() {
            if let (Some(fallback), Some(fallback_rates)) = (fallback, &fallback_rates) {
                rates = Some(fallback_rates.clone());
                rate_model = fallback.to_owned();
                fast = resolution.is_fast_alias || event.is_fast;
                ultrafast = event.is_ultrafast;
                used_fallback = Some(fallback);
            }
        }
        let Some(rates) = rates else {
            continue;
        };
        let event_cost = cost(&rates, event, &rate_model, fast, ultrafast);
        history.add(day, event.total, event_cost, model, used_fallback);
    }
    history
}

/// The session dirs physically inside `home`, resolved.
fn session_dirs(home: &Home) -> Vec<PathBuf> {
    ["sessions", "archived_sessions"]
        .iter()
        .map(|name| home.config_dir.join(name))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Each home's rollouts, by home id: every `*.jsonl` under its `sessions/`
/// and `archived_sessions/` (the home itself when it has neither). Of the
/// two holding the same relative path, `sessions/` counts; a dir another
/// home already reads (by its real path) isn't read again.
pub fn files_by_home(homes: &[Home]) -> HashMap<String, Vec<LogFile>> {
    let mut seen_dirs = HashSet::new();
    let mut by_home = HashMap::new();
    for home in homes {
        let mut dirs = session_dirs(home);
        if dirs.is_empty() {
            dirs.push(home.config_dir.clone());
        }
        let mut seen_relative = HashSet::new();
        let mut files = Vec::new();
        for dir in dirs {
            let dir = fs::canonicalize(&dir).unwrap_or(dir);
            if !seen_dirs.insert(dir.clone()) {
                continue;
            }
            for file in jsonl::jsonl_files(&dir) {
                let relative = file
                    .path
                    .strip_prefix(&dir)
                    .map(PathBuf::from)
                    .unwrap_or_default();
                if seen_relative.insert(relative) {
                    files.push(file);
                }
            }
        }
        by_home.insert(home.id.clone(), files);
    }
    by_home
}

/// Each home's usage history, by home id, as of `now`.
pub fn scan<Tz: TimeZone>(
    homes: &[Home],
    now: &DateTime<Tz>,
    pricing: &ModelPricing,
    fallback: Option<&str>,
    cache: &ScanCache<CodexParser>,
) -> HashMap<String, UsageHistory> {
    let since = daily::window_start(now);
    let by_home = files_by_home(homes);
    let all: Vec<LogFile> = by_home.values().flatten().cloned().collect();
    let parsed = cache.parsed(&all, i128::from(since) * 1_000_000);
    by_home
        .into_iter()
        .map(|(home, files)| {
            let events: Vec<Event> = files
                .iter()
                .filter_map(|file| parsed.get(&file.path))
                .flat_map(|items| items.iter().cloned())
                .collect();
            let history = aggregate(&events, since, &now.timezone(), pricing, fallback);
            (home, history)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};
    use serde_json::json;

    use super::*;
    use crate::test_support::claude_home;
    use crate::usage::spend::pricing::tests::pricing;

    fn test_pricing() -> ModelPricing {
        pricing(
            r#"{"pricing":{},"alias_rules":[],"fallback_models":{"codex":["gpt-5.5"]}}"#,
            r#"{"models":{
                "gpt-5":{"i":1.25,"o":10,"cw":1.25,"cr":0.125},
                "gpt-5.5":{"i":5,"o":30,"cw":5,"cr":0.5},
                "gpt-5.6-luna":{"i":0.2,"o":1.2,"cw":0.25,"cr":0.02},
                "gpt-6-astra":{"i":10,"o":50,"cw":12.5,"cr":1},
                "gpt-nocache":{"i":1,"o":2,"cw":1,"cr":0.1,"cre":false}}}"#,
            r#"{"models":{}}"#,
        )
    }

    fn events(lines: &[Value]) -> Vec<Event> {
        let mut parser = CodexParser::default();
        let mut events = Vec::new();
        for line in lines {
            parser.line(line.to_string().as_bytes(), &mut events);
        }
        events
    }

    fn turn_context(model: &str) -> Value {
        json!({ "timestamp": "2026-10-17T01:00:00Z", "type": "turn_context", "payload": { "model": model } })
    }

    fn token_count(timestamp: &str, last: Option<Value>, total: Option<Value>) -> Value {
        let mut info = serde_json::Map::new();
        if let Some(last) = last {
            info.insert("last_token_usage".into(), last);
        }
        if let Some(total) = total {
            info.insert("total_token_usage".into(), total);
        }
        json!({ "timestamp": timestamp, "type": "event_msg", "payload": { "type": "token_count", "info": info } })
    }

    fn tokens(input: u64, cached: u64, output: u64) -> Value {
        json!({ "input_tokens": input, "cached_input_tokens": cached, "output_tokens": output,
                "reasoning_output_tokens": 0, "total_tokens": input + output })
    }

    fn task_started(timestamp: &str, started_at: i64) -> Value {
        json!({ "timestamp": timestamp, "type": "event_msg", "payload": { "type": "task_started", "started_at": started_at } })
    }

    fn secs(timestamp: &str) -> i64 {
        timestamp.parse::<DateTime<Utc>>().unwrap().timestamp()
    }

    #[test]
    fn a_turn_is_its_last_usage_or_the_difference_in_totals() {
        let found = events(&[
            turn_context("gpt-5.5"),
            token_count(
                "2026-10-17T01:00:01Z",
                Some(tokens(100, 300, 10)),
                Some(tokens(100, 0, 10)),
            ),
            // The same cumulative total again is a repeat, not a turn.
            token_count(
                "2026-10-17T01:00:02Z",
                Some(tokens(100, 0, 10)),
                Some(tokens(100, 0, 10)),
            ),
            token_count("2026-10-17T01:00:03Z", None, Some(tokens(250, 50, 30))),
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].model, "gpt-5.5");
        // Cached input never exceeds input.
        assert_eq!(
            (found[0].input, found[0].cached, found[0].output),
            (100, 100, 10)
        );
        assert_eq!(
            (
                found[1].input,
                found[1].cached,
                found[1].output,
                found[1].total
            ),
            (150, 50, 20, 170)
        );
    }

    #[test]
    fn a_session_without_model_metadata_is_gpt_5() {
        let found = events(&[token_count(
            "2026-10-17T01:00:01Z",
            Some(tokens(1, 0, 1)),
            None,
        )]);
        assert_eq!(found[0].model, "gpt-5");
        let named = events(&[
            json!({ "timestamp": "2026-10-17T01:00:01Z", "type": "event_msg",
            "payload": { "type": "token_count", "model": " gpt-6-astra ", "info": { "last_token_usage": tokens(1, 0, 1) } } }),
        ]);
        assert_eq!(named[0].model, "gpt-6-astra");
    }

    #[test]
    fn a_child_session_skips_its_parent_s_replayed_history() {
        let created = "2026-10-17T02:00:00.500Z";
        let found = events(&[
            json!({ "timestamp": created, "type": "session_meta", "payload": { "forked_from_id": "parent" } }),
            // The parent's history, replayed with new timestamps.
            task_started("2026-10-17T02:00:00Z", secs("2026-10-17T01:00:00Z")),
            token_count(
                "2026-10-17T02:00:00Z",
                Some(tokens(1000, 0, 100)),
                Some(tokens(1000, 0, 100)),
            ),
            json!({ "timestamp": "2026-10-17T02:00:01Z", "type": "session_meta", "payload": { "forked_from_id": "grandparent" } }),
            // The child's own first turn.
            task_started("2026-10-17T02:00:05Z", secs("2026-10-17T02:00:00Z")),
            token_count("2026-10-17T02:00:09Z", None, Some(tokens(1200, 0, 130))),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].input, found[0].output), (200, 30));
    }

    #[test]
    fn without_a_creation_time_a_child_waits_for_a_turn_started_at_its_own_time() {
        let found = events(&[
            json!({ "type": "session_meta", "payload": { "source": { "subagent": { "kind": "review" } } } }),
            task_started("2026-10-17T02:00:00Z", secs("2026-10-17T01:00:00Z")),
            token_count(
                "2026-10-17T02:00:00Z",
                Some(tokens(1000, 0, 100)),
                Some(tokens(1000, 0, 100)),
            ),
            task_started("2026-10-17T02:00:05Z", secs("2026-10-17T02:00:05Z")),
            token_count(
                "2026-10-17T02:00:09Z",
                Some(tokens(5, 0, 5)),
                Some(tokens(1005, 0, 105)),
            ),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].input, 5);
        // A root session that names no parent is not a child.
        let root = events(&[
            json!({ "timestamp": "2026-10-17T02:00:00Z", "type": "session_meta", "payload": { "forked_from_id": null, "parent_thread_id": " " } }),
            token_count("2026-10-17T02:00:01Z", Some(tokens(5, 0, 5)), None),
        ]);
        assert_eq!(root.len(), 1);
    }

    #[test]
    fn the_service_tier_follows_the_session_s_settings() {
        let settings = |tier: &str| {
            json!({ "timestamp": "2026-10-17T01:00:00Z", "type": "event_msg",
                    "payload": { "type": "thread_settings_applied", "thread_settings": { "service_tier": tier } } })
        };
        let found = events(&[
            token_count("2026-10-17T01:00:01Z", Some(tokens(1, 0, 1)), None),
            settings("priority"),
            token_count("2026-10-17T01:00:02Z", Some(tokens(2, 0, 2)), None),
            settings("ultrafast"),
            token_count("2026-10-17T01:00:03Z", Some(tokens(3, 0, 3)), None),
            settings("standard"),
            token_count("2026-10-17T01:00:04Z", Some(tokens(4, 0, 4)), None),
        ]);
        let tiers: Vec<(bool, bool)> = found
            .iter()
            .map(|event| (event.is_fast, event.is_ultrafast))
            .collect();
        assert_eq!(
            tiers,
            [(false, false), (true, false), (false, true), (false, false)]
        );
    }

    fn event(timestamp: &str, model: &str, input: u64, cached: u64, output: u64) -> Event {
        Event {
            timestamp: timestamp
                .parse::<DateTime<Utc>>()
                .unwrap()
                .timestamp_millis(),
            model: model.to_owned(),
            pricing_model: (model == RESERVE).then(|| RESERVE_PRICED_AS.to_owned()),
            input,
            cached,
            output,
            reasoning: 0,
            total: input + output,
            is_fast: false,
            is_ultrafast: false,
        }
    }

    fn cost_of(events: &[Event], fallback: Option<&str>) -> (f64, UsageHistory) {
        let since = "2026-09-17T00:00:00Z"
            .parse::<DateTime<Utc>>()
            .unwrap()
            .timestamp_millis();
        let history = aggregate(events, since, &Utc, &test_pricing(), fallback);
        (history.days.values().map(|day| day.cost).sum(), history)
    }

    fn approx(left: f64, right: f64) {
        assert!((left - right).abs() < 1e-9, "{left} != {right}");
    }

    #[test]
    fn cached_input_is_priced_at_the_cache_rate_and_the_tier_multiplies() {
        let plain = event("2026-10-17T01:00:00Z", "gpt-5", 1_000_000, 200_000, 100_000);
        approx(
            cost_of(std::slice::from_ref(&plain), None).0,
            0.8 * 1.25 + 0.2 * 0.125 + 1.0,
        );
        let fast = Event {
            is_fast: true,
            ..plain.clone()
        };
        approx(
            cost_of(&[fast], None).0,
            2.0 * (0.8 * 1.25 + 0.2 * 0.125 + 1.0),
        );
        // No published discount: cached input at the input rate.
        approx(
            cost_of(
                &[event(
                    "2026-10-17T01:00:00Z",
                    "gpt-nocache",
                    1_000_000,
                    1_000_000,
                    0,
                )],
                None,
            )
            .0,
            1.0,
        );
        // Ultrafast is 6x for GPT-6 Astra.
        let astra = Event {
            is_ultrafast: true,
            ..event("2026-10-17T01:00:00Z", "gpt-6-astra", 200_000, 0, 0)
        };
        approx(cost_of(&[astra], None).0, 0.2 * 10.0 * 6.0);
    }

    #[test]
    fn a_long_request_bills_at_the_long_context_rates() {
        approx(
            cost_of(
                &[event("2026-10-17T01:00:00Z", "gpt-5.5", 272_000, 0, 0)],
                None,
            )
            .0,
            0.272 * 5.0,
        );
        approx(
            cost_of(
                &[event("2026-10-17T01:00:00Z", "gpt-5.5", 300_000, 0, 0)],
                None,
            )
            .0,
            0.3 * 10.0,
        );
    }

    #[test]
    fn auto_review_is_priced_by_its_date_and_reserve_as_luna() {
        // Luna's rates, the model auto-review ran on then.
        let (paid, _) = cost_of(
            &[event("2026-10-05T23:59:59Z", AUTO_REVIEW, 200_000, 0, 0)],
            None,
        );
        approx(paid, 0.2 * 0.2);
        let (free, history) = cost_of(
            &[event("2026-10-06T00:00:00Z", AUTO_REVIEW, 200_000, 0, 0)],
            None,
        );
        approx(free, 0.0);
        let day = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        assert_eq!(history.days[&day].models[AUTO_REVIEW].tokens, 200_000);
        let (reserve, history) = cost_of(
            &[event("2026-10-17T01:00:00Z", RESERVE, 200_000, 0, 0)],
            None,
        );
        approx(reserve, 0.2 * 0.2);
        let day = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        assert!(history.days[&day].models.contains_key(RESERVE));
    }

    #[test]
    fn unknown_models_are_left_out_unless_a_fallback_prices_them() {
        let mystery = [event("2026-10-17T01:00:00Z", "gpt-mystery", 200_000, 0, 0)];
        let day = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let (cost, history) = cost_of(&mystery, None);
        approx(cost, 0.0);
        assert!(history.days.is_empty());
        assert!(history.unknown_models[&day].contains("gpt-mystery"));
        let (cost, history) = cost_of(&mystery, Some("gpt-5.5"));
        approx(cost, 0.2 * 5.0);
        assert!(history.unknown_models[&day].contains("gpt-mystery"));
        assert!(history.fallback_models[&day].contains("gpt-5.5"));
        // A fallback the supplement doesn't offer prices nothing.
        assert!(cost_of(&mystery, Some("gpt-5")).1.days.is_empty());
    }

    #[test]
    fn the_same_event_in_two_files_counts_once() {
        let copy = event("2026-10-17T01:00:00Z", "gpt-5", 1_000_000, 0, 0);
        let (cost, _) = cost_of(&[copy.clone(), copy], None);
        approx(cost, 1.25);
    }

    fn write(path: &std::path::Path, lines: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn each_home_reads_its_rollouts_with_the_active_copy_first() {
        let root = tempfile::tempdir().unwrap();
        let stock = Home {
            app: crate::app_kind::AppKind::Codex,
            ..claude_home(root.path(), "default:codex")
        };
        let work = Home {
            app: crate::app_kind::AppKind::Codex,
            ..claude_home(root.path(), "work")
        };
        let turn =
            |input: u64| token_count("2026-10-17T01:00:00Z", Some(tokens(input, 0, 0)), None);
        write(
            &stock.config_dir.join("sessions/2026/10/17/a.jsonl"),
            &[turn(1_000_000)],
        );
        write(
            &stock
                .config_dir
                .join("archived_sessions/2026/10/17/a.jsonl"),
            &[turn(9_000_000)],
        );
        write(
            &stock.config_dir.join("archived_sessions/b.jsonl"),
            &[turn(2_000_000)],
        );
        // A home with neither folder is read whole.
        write(
            &work.config_dir.join("elsewhere/c.jsonl"),
            &[turn(4_000_000)],
        );
        let homes = [stock, work];
        let now = "2026-10-17T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let cache = ScanCache::new(None, SCHEMA);
        let histories = scan(&homes, &now, &test_pricing(), None, &cache);
        let tokens = |home: &str| {
            histories[home]
                .days
                .values()
                .map(|day| day.tokens)
                .sum::<u64>()
        };
        assert_eq!(tokens("default:codex"), 3_000_000);
        assert_eq!(tokens("work"), 4_000_000);
    }

    #[test]
    fn a_rollout_read_on_keeps_its_model_and_totals() {
        let root = tempfile::tempdir().unwrap();
        let home = Home {
            app: crate::app_kind::AppKind::Codex,
            ..claude_home(root.path(), "work")
        };
        let path = home.config_dir.join("sessions/a.jsonl");
        write(
            &path,
            &[
                turn_context("gpt-5.5"),
                token_count("2026-10-17T01:00:00Z", None, Some(tokens(100, 0, 0))),
            ],
        );
        let homes = [home];
        let now = "2026-10-17T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let cache = ScanCache::new(None, SCHEMA);
        scan(&homes, &now, &test_pricing(), None, &cache);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(
            &mut file,
            format!(
                "{}\n",
                token_count("2026-10-17T01:00:05Z", None, Some(tokens(300, 0, 0)))
            )
            .as_bytes(),
        )
        .unwrap();
        let histories = scan(&homes, &now, &test_pricing(), None, &cache);
        let day = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let usage = &histories["work"].days[&day];
        // 100 then 200 more, both gpt-5.5, not 300 more of gpt-5.
        assert_eq!(usage.tokens, 300);
        assert_eq!(usage.models.keys().collect::<Vec<_>>(), ["gpt-5.5"]);
    }
}
