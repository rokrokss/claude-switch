//! What each Claude profile's sessions cost, from Claude Code's transcripts:
//! `<config>/projects/**/*.jsonl` (subagents' included) and the sessions
//! claude-switch archived under `<config>/claude-switch-archive/`.
//!
//! The rules are ccusage's, as OpenUsage's `ClaudeLogUsageScanner` ports
//! them (MIT, <https://github.com/robinebers/openusage>):
//! - A usage line carries `"usage":{`, reads as JSON with a valid timestamp,
//!   has no `null` where Claude never writes one, and passes the validity
//!   checks: a semver-like `version`, and ids and model not empty.
//! - Its tokens split into input, 5-minute and 1-hour cache writes, cache
//!   reads and output; a `speed` other than `fast` or `standard` marks a
//!   shape not understood, and the line is skipped.
//! - Advisor iterations count as their own requests, under their own model.
//! - Lines are deduplicated by `(message.id, requestId)`, and by message id
//!   alone against subagent logs that replay a parent message under a new
//!   request id. Of two, the one not from a subagent wins, then the one with
//!   more tokens, then the one recording a speed.
//! - A line's own `costUSD` is its cost; otherwise its tokens are priced.
//!
//! A file belongs to the profile that owns its session, as the Sessions list
//! decides (`sessions::transcript_owners`); a subagent's file to its
//! parent session's. A file no profile claims belongs to the one holding it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use super::daily::{self, UsageHistory};
use super::jsonl::{self, LogFile, Parser, ScanCache};
use super::pricing::{ModelPricing, TokenBreakdown};
use crate::sessions::Home;

/// Bumped when what [`ClaudeParser`] yields changes.
pub const SCHEMA: u32 = 1;

/// The folder under a config dir holding the sessions claude-switch archived.
const ARCHIVE_DIR: &str = "claude-switch-archive";
/// The folder in it holding what moves replaced: backups, not sessions.
const REPLACED_DIR: &str = ".replaced";
/// What every usage line contains, checked before any JSON is read.
const USAGE_MARKER: &[u8] = br#""usage":{"#;

/// One usage line, or one advisor iteration of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Milliseconds since the epoch.
    #[serde(rename = "t")]
    pub timestamp: i64,
    #[serde(rename = "i", default, skip_serializing_if = "is_zero")]
    pub input: u64,
    #[serde(rename = "w5", default, skip_serializing_if = "is_zero")]
    pub cache_write_5m: u64,
    #[serde(rename = "w1", default, skip_serializing_if = "is_zero")]
    pub cache_write_1h: u64,
    #[serde(rename = "r", default, skip_serializing_if = "is_zero")]
    pub cache_read: u64,
    #[serde(rename = "o", default, skip_serializing_if = "is_zero")]
    pub output: u64,
    #[serde(rename = "f", default, skip_serializing_if = "is_false")]
    pub is_fast: bool,
    #[serde(rename = "m", default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(rename = "q", default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(rename = "s", default, skip_serializing_if = "is_false")]
    pub is_sidechain: bool,
    /// The line recorded a `speed` at all; `is_fast` says it was fast.
    #[serde(rename = "h", default, skip_serializing_if = "is_false")]
    pub has_speed: bool,
    #[serde(rename = "c", default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// `None` when the line names no model, or the `<synthetic>` stand-in.
    #[serde(rename = "n", default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Entry {
    pub fn tokens(&self) -> TokenBreakdown {
        TokenBreakdown {
            input: self.input,
            cache_write_5m: self.cache_write_5m,
            cache_write_1h: self.cache_write_1h,
            cache_read: self.cache_read,
            output: self.output,
            is_fast: self.is_fast,
        }
    }
}

/// Claude Code's lines stand alone, so the parser keeps no state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeParser;

impl Parser for ClaudeParser {
    type Item = Entry;

    fn line(&mut self, line: &[u8], items: &mut Vec<Entry>) {
        if contains(line, USAGE_MARKER) {
            items.extend(parse_line(line));
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A field as written: absent, `null` (`Some(Value::Null)`), or a value.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// The fields of a line read here; the rest, a message's content above all,
/// is skipped without being built.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    timestamp: Option<Value>,
    message: Option<Message>,
    #[serde(default, deserialize_with = "present")]
    cwd: Option<Value>,
    #[serde(default, rename = "costUSD", deserialize_with = "present")]
    cost_usd: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    version: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    session_id: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    request_id: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    is_api_error_message: Option<Value>,
    #[serde(default)]
    is_sidechain: Option<Value>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default, deserialize_with = "present")]
    id: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    model: Option<Value>,
    usage: Option<Value>,
}

/// The entries of one usage line: its own, then one per advisor iteration.
/// Other iterations are already in the line's own totals. Nothing for a
/// line that breaks a rule.
pub fn parse_line(line: &[u8]) -> Vec<Entry> {
    parse(line).unwrap_or_default()
}

fn parse(line: &[u8]) -> Option<Vec<Entry>> {
    let line: Line = serde_json::from_slice(line).ok()?;
    let timestamp = daily::timestamp_millis(line.timestamp.as_ref()?.as_str()?)?;
    let message = line.message?;
    let Some(Value::Object(usage)) = &message.usage else {
        return None;
    };
    // Claude never writes `null` into these; a line that does is some other
    // shape.
    let nulls = [
        &line.cwd,
        &line.cost_usd,
        &line.version,
        &line.session_id,
        &line.request_id,
        &line.is_api_error_message,
        &message.id,
        &message.model,
    ];
    if nulls.iter().any(|field| matches!(field, Some(Value::Null))) {
        return None;
    }
    if [
        "speed",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ]
    .iter()
    .any(|key| matches!(usage.get(*key), Some(Value::Null)))
    {
        return None;
    }
    let (tokens, has_speed) = token_breakdown(usage)?;
    if let Some(Value::String(version)) = &line.version {
        if !is_semver_prefix(version) {
            return None;
        }
    }
    let empty = [
        &line.session_id,
        &line.request_id,
        &message.id,
        &message.model,
    ]
    .iter()
    .any(|field| matches!(field, Some(Value::String(text)) if text.is_empty()));
    if empty {
        return None;
    }

    let text = |field: &Option<Value>| field.as_ref().and_then(Value::as_str).map(str::to_owned);
    let parent = Entry {
        timestamp,
        message_id: text(&message.id),
        request_id: text(&line.request_id),
        is_sidechain: line
            .is_sidechain
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(false),
        has_speed,
        cost_usd: line.cost_usd.as_ref().and_then(Value::as_f64),
        model: text(&message.model).filter(|model| model != "<synthetic>"),
        ..with_tokens(tokens)
    };
    // Every iteration must be an object, or none are read.
    let iterations: Option<Vec<&Map<String, Value>>> = match usage.get("iterations") {
        Some(Value::Array(items)) => items.iter().map(Value::as_object).collect(),
        _ => None,
    };
    let mut entries = Vec::new();
    let mut advisor = 0;
    for iteration in iterations.into_iter().flatten() {
        if iteration.get("type").and_then(Value::as_str) != Some("advisor_message") {
            continue;
        }
        let Some(model) = iteration
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.is_empty())
        else {
            continue;
        };
        let Some((tokens, has_speed)) = token_breakdown(iteration) else {
            continue;
        };
        entries.push(Entry {
            timestamp,
            message_id: parent
                .message_id
                .as_ref()
                .map(|id| format!("{id}:advisor:{advisor}")),
            request_id: parent.request_id.clone(),
            is_sidechain: parent.is_sidechain,
            has_speed,
            cost_usd: None,
            model: Some(model.to_owned()),
            ..with_tokens(tokens)
        });
        advisor += 1;
    }
    entries.insert(0, parent);
    Some(entries)
}

fn with_tokens(tokens: TokenBreakdown) -> Entry {
    Entry {
        input: tokens.input,
        cache_write_5m: tokens.cache_write_5m,
        cache_write_1h: tokens.cache_write_1h,
        cache_read: tokens.cache_read,
        output: tokens.output,
        is_fast: tokens.is_fast,
        ..Entry::default()
    }
}

/// A count as Claude writes it: a number, a fraction cut to a whole one.
fn count(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|count| count.max(0) as u64))
        .or_else(|| value.as_f64().map(|count| count.max(0.0) as u64))
}

/// The tokens of a `usage` object, and whether it recorded a speed. Input
/// and output must be numbers. Cache writes are the 5-minute and 1-hour
/// split when given, else the older total read as all 5-minute.
fn token_breakdown(usage: &Map<String, Value>) -> Option<(TokenBreakdown, bool)> {
    let input = count(usage.get("input_tokens"))?;
    let output = count(usage.get("output_tokens"))?;
    let speed = usage.get("speed").and_then(Value::as_str);
    if speed.is_some_and(|speed| speed != "fast" && speed != "standard") {
        return None;
    }
    let (cache_write_5m, cache_write_1h) = match usage.get("cache_creation") {
        Some(Value::Object(split)) => (
            count(split.get("ephemeral_5m_input_tokens")).unwrap_or(0),
            count(split.get("ephemeral_1h_input_tokens")).unwrap_or(0),
        ),
        _ => (
            count(usage.get("cache_creation_input_tokens")).unwrap_or(0),
            0,
        ),
    };
    let tokens = TokenBreakdown {
        input,
        cache_write_5m,
        cache_write_1h,
        cache_read: count(usage.get("cache_read_input_tokens")).unwrap_or(0),
        output,
        is_fast: speed == Some("fast"),
    };
    Some((tokens, speed.is_some()))
}

/// `digits.digits.digit…`: takes `1.0.24` and pre-release suffixes, not
/// `unknown`.
fn is_semver_prefix(version: &str) -> bool {
    let bytes = version.as_bytes();
    let mut at = 0;
    let digits = |at: &mut usize| {
        let start = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        *at > start
    };
    for _ in 0..2 {
        if !digits(&mut at) || bytes.get(at) != Some(&b'.') {
            return false;
        }
        at += 1;
    }
    bytes.get(at).is_some_and(u8::is_ascii_digit)
}

/// `entries` without the lines logged twice (see the module docs). Entries
/// without a message id are all kept; of duplicates, the first is kept
/// unless a later one wins [`should_replace`].
pub fn dedup(entries: Vec<Entry>) -> Vec<Entry> {
    let mut kept: Vec<Entry> = Vec::with_capacity(entries.len());
    let mut exact: HashMap<(String, Option<String>), usize> = HashMap::new();
    let mut by_message: HashMap<String, Vec<usize>> = HashMap::new();
    for entry in entries {
        let Some(message_id) = entry.message_id.clone() else {
            kept.push(entry);
            continue;
        };
        let key = (message_id.clone(), entry.request_id.clone());
        let collision = exact.get(&key).copied().or_else(|| {
            by_message.get(&message_id).and_then(|indices| {
                indices
                    .iter()
                    .copied()
                    .find(|&index| entry.is_sidechain || kept[index].is_sidechain)
            })
        });
        if let Some(index) = collision {
            if should_replace(&entry, &kept[index]) {
                let old = &kept[index];
                if let Some(old_id) = &old.message_id {
                    exact.remove(&(old_id.clone(), old.request_id.clone()));
                }
                kept[index] = entry;
                exact.insert(key, index);
            }
            continue;
        }
        let index = kept.len();
        kept.push(entry);
        exact.insert(key, index);
        by_message.entry(message_id).or_default().push(index);
    }
    kept
}

/// Of two copies of a line: the one not from a subagent, then the one with
/// more tokens, then the one recording a speed.
fn should_replace(candidate: &Entry, existing: &Entry) -> bool {
    if candidate.is_sidechain != existing.is_sidechain {
        return existing.is_sidechain;
    }
    let (new_total, old_total) = (
        candidate.tokens().total_tokens(),
        existing.tokens().total_tokens(),
    );
    if new_total != old_total {
        return new_total > old_total;
    }
    candidate.has_speed && !existing.has_speed
}

/// `entries`, deduplicated, priced and counted by local day from `since`
/// (milliseconds since the epoch). A line's own `costUSD` comes first.
pub fn aggregate<Tz: TimeZone>(
    entries: &[Entry],
    since: i64,
    zone: &Tz,
    pricing: &ModelPricing,
) -> UsageHistory {
    const UNATTRIBUTED: &str = "Unattributed";
    let mut history = UsageHistory::default();
    for entry in entries.iter().filter(|entry| entry.timestamp >= since) {
        let Some(day) = daily::day_of(zone, entry.timestamp) else {
            continue;
        };
        // One spelling for the price, the warning and the breakdown alike.
        let model = entry
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty());
        let tokens = entry.tokens();
        let cost = match (entry.cost_usd, model) {
            (Some(cost), _) => cost,
            (None, Some(model)) => match pricing.estimated_cost(model, &tokens) {
                Some(cost) => cost,
                None => {
                    if tokens.total_tokens() > 0 {
                        history.add_unknown(day, model);
                    }
                    continue;
                }
            },
            (None, None) => continue,
        };
        history.add(
            day,
            tokens.total_tokens(),
            cost,
            model.unwrap_or(UNATTRIBUTED),
            None,
        );
    }
    history
}

/// The session transcript `file` belongs to: for a subagent's, under
/// `<slug>/<id>/subagents/`, the parent's `<slug>/<id>.jsonl`; for any
/// other, itself.
fn owning_session(file: &Path) -> PathBuf {
    let mut dir = file.parent();
    while let Some(current) = dir {
        match current.file_name().and_then(|name| name.to_str()) {
            None | Some("projects") => break,
            Some("subagents") => {
                if let Some(session_dir) = current.parent() {
                    if let Some(name) = session_dir.file_name() {
                        let mut file_name = name.to_os_string();
                        file_name.push(".jsonl");
                        return session_dir.with_file_name(file_name);
                    }
                }
                break;
            }
            Some(_) => dir = current.parent(),
        }
    }
    file.to_path_buf()
}

/// Each home's log files, by home id, path-sorted: what its config dir's
/// `projects/` holds, less what other homes own, plus what they hold that
/// it owns (`owners`: session transcript → owning home id), plus its
/// archived sessions.
pub fn files_by_home(
    homes: &[Home],
    owners: &HashMap<PathBuf, String>,
) -> HashMap<String, Vec<LogFile>> {
    let known: HashSet<&str> = homes.iter().map(|home| home.id.as_str()).collect();
    let mut by_home: HashMap<String, Vec<LogFile>> = HashMap::new();
    for home in homes {
        let mut found = Vec::new();
        jsonl::walk(&home.config_dir.join("projects"), &mut |path| {
            if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                found.push(path.to_path_buf());
            }
        });
        for path in found {
            let owner = owners
                .get(&owning_session(&path))
                .filter(|owner| known.contains(owner.as_str()))
                .cloned()
                .unwrap_or_else(|| home.id.clone());
            if let Some(file) = LogFile::at(&path) {
                by_home.entry(owner).or_default().push(file);
            }
        }
        let archive = home.config_dir.join(ARCHIVE_DIR);
        let Ok(sessions) = std::fs::read_dir(&archive) else {
            continue;
        };
        for session in sessions.flatten() {
            if session.file_name() == REPLACED_DIR
                || !session.file_type().is_ok_and(|kind| kind.is_dir())
            {
                continue;
            }
            jsonl::walk(&session.path(), &mut |path| {
                if path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                {
                    if let Some(file) = LogFile::at(path) {
                        by_home.entry(home.id.clone()).or_default().push(file);
                    }
                }
            });
        }
    }
    for files in by_home.values_mut() {
        files.sort_by(|a, b| a.path.cmp(&b.path));
    }
    by_home
}

/// Each home's usage history, by home id, as of `now`.
pub fn scan<Tz: TimeZone>(
    homes: &[Home],
    owners: &HashMap<PathBuf, String>,
    now: &DateTime<Tz>,
    pricing: &ModelPricing,
    cache: &ScanCache<ClaudeParser>,
) -> HashMap<String, UsageHistory> {
    let since = daily::window_start(now);
    let by_home = files_by_home(homes, owners);
    // Every home's files in one call, so the cache forgets only what's gone.
    let all: Vec<LogFile> = by_home.values().flatten().cloned().collect();
    let parsed = cache.parsed(&all, i128::from(since) * 1_000_000);
    by_home
        .into_iter()
        .map(|(home, files)| {
            let entries: Vec<Entry> = files
                .iter()
                .filter_map(|file| parsed.get(&file.path))
                .flat_map(|items| items.iter().cloned())
                .collect();
            let history = aggregate(&dedup(entries), since, &now.timezone(), pricing);
            (home, history)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{FixedOffset, Utc};
    use serde_json::json;

    use super::*;
    use crate::test_support::claude_home;
    use crate::usage::spend::pricing::tests::pricing;

    /// Prices claude-test at $1/$2 per million (cache write $1.25, read
    /// $0.10), and claude-advisor at ten times that.
    fn test_pricing() -> ModelPricing {
        pricing(
            r#"{"pricing":{},"alias_rules":[]}"#,
            r#"{"models":{"claude-test":{"i":1,"o":2,"cw":1.25,"cr":0.1},
                          "claude-advisor":{"i":10,"o":20,"cw":12.5,"cr":1,"fast":2}}}"#,
            r#"{"models":{}}"#,
        )
    }

    /// An assistant line of message `id`, request `request` at `timestamp`,
    /// with `usage`.
    fn line(id: &str, request: &str, timestamp: &str, usage: Value) -> Value {
        json!({
            "type": "assistant",
            "sessionId": "s1",
            "requestId": request,
            "timestamp": timestamp,
            "version": "2.1.270",
            "cwd": "/work",
            "isSidechain": false,
            "message": { "id": id, "model": "claude-test", "usage": usage, "content": [{"type": "text", "text": "hi"}] },
        })
    }

    fn usage(input: u64, output: u64) -> Value {
        json!({ "input_tokens": input, "output_tokens": output })
    }

    fn parse_value(value: &Value) -> Vec<Entry> {
        parse_line(value.to_string().as_bytes())
    }

    #[test]
    fn a_usage_line_reads_its_tokens_ids_and_model() {
        let mut value = line(
            "m1",
            "r1",
            "2026-10-17T01:00:00.123Z",
            json!({
                "input_tokens": 10, "output_tokens": 20, "cache_read_input_tokens": 30,
                "cache_creation": { "ephemeral_5m_input_tokens": 40, "ephemeral_1h_input_tokens": 50 },
                "cache_creation_input_tokens": 999,
                "speed": "fast",
            }),
        );
        value["costUSD"] = json!(0.5);
        let entries = parse_value(&value);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(
            (
                entry.input,
                entry.output,
                entry.cache_read,
                entry.cache_write_5m,
                entry.cache_write_1h
            ),
            (10, 20, 30, 40, 50)
        );
        assert!(entry.is_fast && entry.has_speed);
        assert_eq!(entry.message_id.as_deref(), Some("m1"));
        assert_eq!(entry.request_id.as_deref(), Some("r1"));
        assert_eq!(entry.model.as_deref(), Some("claude-test"));
        assert_eq!(entry.cost_usd, Some(0.5));
        assert_eq!(
            entry.timestamp,
            "2026-10-17T01:00:00.123Z"
                .parse::<DateTime<Utc>>()
                .unwrap()
                .timestamp_millis()
        );
    }

    #[test]
    fn the_older_cache_write_total_counts_as_five_minute_writes() {
        let value = line(
            "m1",
            "r1",
            "2026-10-17T01:00:00Z",
            json!({
                "input_tokens": 1, "output_tokens": 2, "cache_creation_input_tokens": 7, "speed": "standard",
            }),
        );
        let entry = &parse_value(&value)[0];
        assert_eq!((entry.cache_write_5m, entry.cache_write_1h), (7, 0));
        assert!(!entry.is_fast && entry.has_speed);
    }

    #[test]
    fn lines_that_break_a_rule_are_skipped() {
        let valid = line("m1", "r1", "2026-10-17T01:00:00Z", usage(1, 2));
        assert_eq!(parse_value(&valid).len(), 1);
        let broken = |change: &dyn Fn(&mut Value)| {
            let mut value = valid.clone();
            change(&mut value);
            parse_value(&value).is_empty()
        };
        assert!(broken(&|value| value["costUSD"] = Value::Null));
        assert!(broken(&|value| value["cwd"] = Value::Null));
        assert!(broken(&|value| value["message"]["model"] = Value::Null));
        assert!(broken(
            &|value| value["message"]["usage"]["speed"] = Value::Null
        ));
        assert!(broken(
            &|value| value["message"]["usage"]["speed"] = json!("turbo")
        ));
        assert!(broken(&|value| value["message"]["usage"]
            ["cache_read_input_tokens"] =
            Value::Null));
        assert!(broken(
            &|value| value["message"]["usage"]["output_tokens"] = json!("2")
        ));
        assert!(broken(&|value| value["version"] = json!("unknown")));
        assert!(broken(&|value| value["requestId"] = json!("")));
        assert!(broken(&|value| value["message"]["id"] = json!("")));
        assert!(broken(&|value| value["timestamp"] = json!("soon")));
        assert!(broken(&|value| value["message"] = json!("text")));
        // What Claude does write is fine.
        assert!(!broken(&|value| value["version"] = json!("2.1.3-beta.1")));
        assert!(!broken(&|value| value["isSidechain"] = json!("yes")));
        assert!(!broken(&|value| value["message"]["usage"]["iterations"] =
            json!([{"type": "message", "model": null}])));
        // Without the marker the line isn't even read.
        let mut items = Vec::new();
        ClaudeParser.line(
            br#"{"message":{"usage": {"input_tokens":1,"output_tokens":1}}}"#,
            &mut items,
        );
        assert!(items.is_empty());
    }

    #[test]
    fn advisor_iterations_count_under_their_own_model() {
        let mut value = line("m1", "r1", "2026-10-17T01:00:00Z", usage(100, 200));
        value["message"]["usage"]["iterations"] = json!([
            { "type": "message", "input_tokens": 100, "output_tokens": 200 },
            { "type": "advisor_message", "model": "claude-advisor", "input_tokens": 5, "output_tokens": 6, "speed": "fast" },
            { "type": "advisor_message", "model": "", "input_tokens": 5, "output_tokens": 6 },
            { "type": "advisor_message", "model": "claude-advisor", "input_tokens": 7, "output_tokens": 8 },
        ]);
        let entries = parse_value(&value);
        let ids: Vec<&str> = entries
            .iter()
            .filter_map(|entry| entry.message_id.as_deref())
            .collect();
        assert_eq!(ids, ["m1", "m1:advisor:0", "m1:advisor:1"]);
        assert_eq!(entries[1].model.as_deref(), Some("claude-advisor"));
        assert!(entries[1].is_fast && entries[1].cost_usd.is_none());
        assert_eq!((entries[2].input, entries[2].output), (7, 8));
        // A list with anything but objects in it is left alone.
        value["message"]["usage"]["iterations"] = json!([1, { "type": "advisor_message", "model": "claude-advisor", "input_tokens": 5, "output_tokens": 6 }]);
        assert_eq!(parse_value(&value).len(), 1);
    }

    #[test]
    fn the_synthetic_model_is_no_model() {
        let mut value = line("m1", "r1", "2026-10-17T01:00:00Z", usage(1, 2));
        value["message"]["model"] = json!("<synthetic>");
        assert_eq!(parse_value(&value)[0].model, None);
    }

    fn entry(
        message: Option<&str>,
        request: &str,
        sidechain: bool,
        output: u64,
        has_speed: bool,
    ) -> Entry {
        Entry {
            message_id: message.map(str::to_owned),
            request_id: Some(request.to_owned()),
            is_sidechain: sidechain,
            output,
            has_speed,
            ..Entry::default()
        }
    }

    #[test]
    fn a_line_logged_twice_counts_once_keeping_the_better_copy() {
        let kept = dedup(vec![
            entry(Some("m1"), "r1", false, 10, false),
            entry(Some("m1"), "r1", false, 10, false),
            entry(Some("m1"), "r1", false, 10, true),
            entry(Some("m2"), "r1", false, 5, false),
            entry(Some("m2"), "r1", false, 9, false),
            entry(None, "r1", false, 1, false),
            entry(None, "r1", false, 1, false),
        ]);
        let summary: Vec<(Option<&str>, u64, bool)> = kept
            .iter()
            .map(|entry| (entry.message_id.as_deref(), entry.output, entry.has_speed))
            .collect();
        assert_eq!(
            summary,
            [
                (Some("m1"), 10, true),
                (Some("m2"), 9, false),
                (None, 1, false),
                (None, 1, false)
            ]
        );
    }

    #[test]
    fn a_subagent_replaying_a_message_under_a_new_request_counts_once() {
        // The parent's copy wins over the subagent's, either way round.
        let kept = dedup(vec![
            entry(Some("m1"), "r-sub", true, 50, false),
            entry(Some("m1"), "r-main", false, 10, false),
        ]);
        assert_eq!(kept.len(), 1);
        assert!(!kept[0].is_sidechain);
        let kept = dedup(vec![
            entry(Some("m1"), "r-main", false, 10, false),
            entry(Some("m1"), "r-sub", true, 50, false),
        ]);
        assert_eq!((kept.len(), kept[0].output), (1, 10));
        // Outside subagents, two requests of one message are two requests.
        let kept = dedup(vec![
            entry(Some("m1"), "r1", false, 10, false),
            entry(Some("m1"), "r2", false, 10, false),
        ]);
        assert_eq!(kept.len(), 2);
    }

    fn at(timestamp: &str) -> i64 {
        timestamp
            .parse::<DateTime<Utc>>()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn a_line_s_own_cost_comes_first_and_unknown_models_are_left_out() {
        let priced = |model: Option<&str>, cost: Option<f64>, timestamp: &str| Entry {
            timestamp: at(timestamp),
            input: 1_000_000,
            output: 1_000_000,
            model: model.map(str::to_owned),
            cost_usd: cost,
            ..Entry::default()
        };
        let entries = [
            priced(Some("claude-test"), None, "2026-10-17T01:00:00Z"),
            priced(Some("claude-test"), Some(0.25), "2026-10-17T02:00:00Z"),
            priced(Some("mystery-model"), None, "2026-10-17T03:00:00Z"),
            priced(None, Some(1.0), "2026-10-17T04:00:00Z"),
            priced(None, None, "2026-10-17T05:00:00Z"),
            priced(Some("claude-test"), None, "2026-09-01T00:00:00Z"),
        ];
        let history = aggregate(&entries, at("2026-09-17T00:00:00Z"), &Utc, &test_pricing());
        let day: chrono::NaiveDate = "2026-10-17".parse().unwrap();
        let usage = &history.days[&day];
        assert!((usage.cost - (3.0 + 0.25 + 1.0)).abs() < 1e-9);
        assert_eq!(usage.tokens, 6_000_000);
        assert_eq!(usage.models["Unattributed"].tokens, 2_000_000);
        assert!(history.unknown_models[&day].contains("mystery-model"));
        assert_eq!(history.days.len(), 1);
    }

    /// Writes `lines` as transcript `path` under `home`'s config dir.
    fn write(home: &Home, path: &str, lines: &[Value]) {
        let file = home.config_dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(file, body).unwrap();
    }

    #[test]
    fn each_profile_counts_the_sessions_it_owns() {
        let root = tempfile::tempdir().unwrap();
        let stock = Home {
            stock: true,
            ..claude_home(root.path(), "default:claude")
        };
        let work = claude_home(root.path(), "work");
        let today = "2026-10-17T03:00:00Z";
        // A plain CLI session of the stock install.
        write(
            &stock,
            "projects/-a/cli.jsonl",
            &[line("m1", "r1", today, usage(1_000_000, 0))],
        );
        // A session of work's desktop app whose transcript sits in the stock
        // config dir, with a subagent beside it.
        write(
            &stock,
            "projects/-a/desk.jsonl",
            &[line("m2", "r2", today, usage(0, 1_000_000))],
        );
        write(
            &stock,
            "projects/-a/desk/subagents/agent-1.jsonl",
            &[line("m3", "r3", today, usage(2_000_000, 0))],
        );
        let records = work.gui_data_dir.join("claude-code-sessions/acct/org");
        fs::create_dir_all(&records).unwrap();
        fs::write(
            records.join("local_1.json"),
            json!({ "cliSessionId": "desk" }).to_string(),
        )
        .unwrap();
        // A session work archived, and a backup that isn't a session.
        write(
            &work,
            "claude-switch-archive/old/2026-10-17T00-00-00Z/projects/-a/old.jsonl",
            &[line("m4", "r4", today, usage(4_000_000, 0))],
        );
        write(
            &work,
            "claude-switch-archive/.replaced/old/projects/-a/old.jsonl",
            &[line("m5", "r5", today, usage(8_000_000, 0))],
        );

        let homes = [stock, work];
        let owners = crate::sessions::transcript_owners(&homes);
        let now = "2026-10-17T12:00:00+09:00"
            .parse::<DateTime<FixedOffset>>()
            .unwrap();
        let cache = ScanCache::new(None, SCHEMA);
        let histories = scan(&homes, &owners, &now, &test_pricing(), &cache);
        let tokens = |home: &str| {
            histories[home]
                .days
                .values()
                .map(|day| day.tokens)
                .sum::<u64>()
        };
        let cost = |home: &str| {
            histories[home]
                .days
                .values()
                .map(|day| day.cost)
                .sum::<f64>()
        };
        assert_eq!(tokens("default:claude"), 1_000_000);
        assert_eq!(tokens("work"), 1_000_000 + 2_000_000 + 4_000_000);
        assert!((cost("work") - (2.0 + 2.0 + 4.0)).abs() < 1e-9);
    }

    #[test]
    fn a_subagent_file_belongs_to_the_session_above_it() {
        let path = Path::new("/c/projects/-a/s1/subagents/workflows/w/agent.jsonl");
        assert_eq!(owning_session(path), Path::new("/c/projects/-a/s1.jsonl"));
        let path = Path::new("/c/projects/-a/s1.jsonl");
        assert_eq!(owning_session(path), path);
    }
}
