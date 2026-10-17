//! The file each Mac keeps in iCloud: its own usage history, by account, as
//! of when it wrote it. Read back from every Mac's file, the histories of an
//! account add up across Macs. After OpenUsage's `UsageHistoryDocument` and
//! `UsageHistoryAggregator` (MIT, <https://github.com/robinebers/openusage>).
//!
//! An account is named by a hash of its key ([`account_id`]), never by an
//! email or an id the file would give away, and a history is only ever added
//! to the same account's.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::usage::spend::daily::{UsageHistory, PREVIOUS_DAYS};
use crate::usage::spend::Histories;

pub const SCHEMA: &str = "claude-switch.history.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDocument {
    pub schema: String,
    pub device_id: String,
    pub device_name: String,
    pub updated_at: DateTime<Utc>,
    /// This Mac's history of each account, by [`account_id`].
    pub accounts: BTreeMap<String, UsageHistory>,
}

/// What names an account in the file: the SHA-256 of its
/// [`crate::accounts::ProfileAccount::key`], in hex.
pub fn account_id(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

/// This Mac's document: the history of every home in `homes` (home id and
/// account id, in order) that has one, by account. Homes signed in to one
/// account add up; a home with no known account is left out, as it can't be
/// matched to any other.
pub fn local(
    device_id: &str,
    device_name: &str,
    histories: &Histories,
    homes: &[(String, Option<String>)],
    now: DateTime<Utc>,
) -> HistoryDocument {
    let mut accounts: BTreeMap<String, UsageHistory> = BTreeMap::new();
    for (home, account) in homes {
        if let (Some(account), Some(history)) = (account, histories.get(home)) {
            add(accounts.entry(account.clone()).or_default(), history, None);
        }
    }
    HistoryDocument {
        schema: SCHEMA.to_owned(),
        device_id: device_id.to_owned(),
        device_name: device_name.to_owned(),
        updated_at: now,
        accounts,
    }
}

/// `histories` with each account's history from `peers` added, for the
/// window ending `today`, to the first of `homes` (home id and account id,
/// in the panel's order) signed in to it. Added to two homes, the total
/// spend would count it twice.
pub fn combined(
    histories: &Histories,
    homes: &[(String, Option<String>)],
    peers: &[HistoryDocument],
    today: NaiveDate,
) -> Histories {
    let mut combined = histories.clone();
    let window = (today - Duration::days(PREVIOUS_DAYS), today);
    let mut claimed = Vec::new();
    for (home, account) in homes {
        let Some(account) = account else {
            continue;
        };
        if claimed.contains(&account) {
            continue;
        }
        claimed.push(account);
        for peer in peers.iter().filter_map(|peer| peer.accounts.get(account)) {
            add(
                combined.entry(home.clone()).or_default(),
                peer,
                Some(window),
            );
        }
    }
    combined.retain(|_, history| !history.days.is_empty() || !history.unknown_models.is_empty());
    combined
}

/// Adds `from`'s days, those within `window` (first and last day) if given,
/// to `into`.
fn add(into: &mut UsageHistory, from: &UsageHistory, window: Option<(NaiveDate, NaiveDate)>) {
    let within = |day: &NaiveDate| window.is_none_or(|(first, last)| (first..=last).contains(day));
    for (day, usage) in from.days.iter().filter(|(day, _)| within(day)) {
        let total = into.days.entry(*day).or_default();
        total.tokens += usage.tokens;
        total.cost += usage.cost;
        for (model, used) in &usage.models {
            let by_model = total.models.entry(model.clone()).or_default();
            by_model.tokens += used.tokens;
            by_model.cost += used.cost;
        }
    }
    for (day, models) in from.unknown_models.iter().filter(|(day, _)| within(day)) {
        into.unknown_models
            .entry(*day)
            .or_default()
            .extend(models.iter().cloned());
    }
    for (day, models) in from.fallback_models.iter().filter(|(day, _)| within(day)) {
        into.fallback_models
            .entry(*day)
            .or_default()
            .extend(models.iter().cloned());
    }
}

/// The newest document of each device in `documents`: a device whose file
/// iCloud kept two versions of shows once.
pub fn newest_by_device(documents: Vec<HistoryDocument>) -> Vec<HistoryDocument> {
    let mut newest: HashMap<String, HistoryDocument> = HashMap::new();
    for document in documents {
        match newest.get(&document.device_id) {
            Some(kept) if kept.updated_at >= document.updated_at => {}
            _ => {
                newest.insert(document.device_id.clone(), document);
            }
        }
    }
    let mut documents: Vec<HistoryDocument> = newest.into_values().collect();
    documents.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    documents
}

/// Reads one Mac's file, refusing what claude-switch wouldn't have written.
pub fn parse(bytes: &[u8]) -> Result<HistoryDocument, String> {
    #[derive(Deserialize)]
    struct Schema {
        schema: String,
    }
    let schema: Schema = serde_json::from_slice(bytes).map_err(|err| err.to_string())?;
    if schema.schema != SCHEMA {
        return Err(format!(
            "written as {}, which a newer claude-switch writes",
            schema.schema
        ));
    }
    let document: HistoryDocument = serde_json::from_slice(bytes).map_err(|err| err.to_string())?;
    if uuid::Uuid::parse_str(&document.device_id).is_err() {
        return Err("its device id isn't one".into());
    }
    if document.device_name.trim().is_empty() {
        return Err("it names no Mac".into());
    }
    for (account, history) in &document.accounts {
        if account.len() != 64 || !account.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("an account in it isn't named by a hash".into());
        }
        let mut costs = history.days.values().flat_map(|day| {
            std::iter::once(day.cost).chain(day.models.values().map(|model| model.cost))
        });
        if costs.any(|cost| !cost.is_finite() || cost < 0.0) {
            return Err("it holds a cost that can't be".into());
        }
        if history
            .days
            .values()
            .any(|day| day.models.keys().any(|model| model.trim().is_empty()))
        {
            return Err("it holds a model without a name".into());
        }
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn history(entries: &[(&str, u64, f64, &str)]) -> UsageHistory {
        let mut history = UsageHistory::default();
        for (date, tokens, cost, model) in entries {
            history.add(day(date), *tokens, *cost, model, None);
        }
        history
    }

    fn document(device: &str, at: &str, accounts: &[(&str, UsageHistory)]) -> HistoryDocument {
        HistoryDocument {
            schema: SCHEMA.into(),
            device_id: device.into(),
            device_name: "Other Mac".into(),
            updated_at: at.parse().unwrap(),
            accounts: accounts
                .iter()
                .map(|(key, history)| (account_id(key), history.clone()))
                .collect(),
        }
    }

    const DEVICE: &str = "6f1d2c1e-8a4b-4c9e-9d7e-3b2a1f0e9d8c";

    #[test]
    fn accounts_are_named_by_a_hash() {
        let id = account_id("claude|a1|o1");
        assert_eq!(id.len(), 64);
        assert_eq!(id, account_id("claude|a1|o1"));
        assert_ne!(id, account_id("claude|a1|o2"));
        assert!(!id.contains("a1"));
    }

    #[test]
    fn the_local_document_adds_up_homes_of_one_account_and_leaves_out_the_unknown() {
        let mut histories = Histories::new();
        histories.insert("stock".into(), history(&[("2026-10-17", 10, 1.0, "opus")]));
        histories.insert("work".into(), history(&[("2026-10-17", 5, 0.5, "opus")]));
        histories.insert("anon".into(), history(&[("2026-10-17", 7, 0.7, "opus")]));
        let a = account_id("claude|a");
        let homes = vec![
            ("stock".to_owned(), Some(a.clone())),
            ("work".to_owned(), Some(a.clone())),
            ("anon".to_owned(), None),
            ("idle".to_owned(), Some(account_id("claude|b"))),
        ];
        let document = local(DEVICE, "This Mac", &histories, &homes, Utc::now());
        assert_eq!(document.accounts.len(), 1);
        let usage = &document.accounts[&a].days[&day("2026-10-17")];
        assert_eq!((usage.tokens, usage.cost), (15, 1.5));
        assert_eq!(usage.models["opus"].tokens, 15);
    }

    #[test]
    fn peers_add_to_the_first_home_of_the_same_account_within_the_window() {
        let mut histories = Histories::new();
        histories.insert("home".into(), history(&[("2026-10-17", 10, 1.0, "opus")]));
        let a = account_id("claude|a");
        let homes = vec![
            ("home".to_owned(), Some(a.clone())),
            ("second".to_owned(), Some(a.clone())),
            ("other".to_owned(), Some(account_id("claude|b"))),
            ("anon".to_owned(), None),
        ];
        let mut peer = history(&[
            ("2026-10-17", 4, 0.4, "opus"),
            ("2026-10-16", 2, 0.2, "sonnet"),
            // Before the window, and a day ahead: neither counts.
            ("2026-09-16", 100, 10.0, "opus"),
            ("2026-10-18", 100, 10.0, "opus"),
        ]);
        peer.add_unknown(day("2026-10-16"), "mystery");
        let peers = vec![
            document(DEVICE, "2026-10-17T00:00:00Z", &[("claude|a", peer)]),
            document(
                "0e8e7a59-2d6c-4f41-8b0c-0f6f8f0b1a2c",
                "2026-10-17T00:00:00Z",
                &[("claude|c", history(&[("2026-10-17", 1, 0.1, "opus")]))],
            ),
        ];
        let combined = combined(&histories, &homes, &peers, day("2026-10-17"));
        let home = &combined["home"];
        assert_eq!(home.days[&day("2026-10-17")].tokens, 14);
        assert_eq!(home.days[&day("2026-10-17")].models["opus"].tokens, 14);
        assert_eq!(home.days[&day("2026-10-16")].models["sonnet"].tokens, 2);
        assert_eq!(home.days.len(), 2);
        assert!(home.unknown_models[&day("2026-10-16")].contains("mystery"));
        // Once only, and never to another account or to a home with none.
        assert!(!combined.contains_key("second"));
        assert!(!combined.contains_key("other"));
        assert!(!combined.contains_key("anon"));
        // Local history is left as it was.
        assert_eq!(histories["home"].days[&day("2026-10-17")].tokens, 10);
    }

    #[test]
    fn a_peer_can_bring_a_home_its_only_history() {
        let a = account_id("codex|w|ada@example.com");
        let homes = vec![("home".to_owned(), Some(a))];
        let peers = vec![document(
            DEVICE,
            "2026-10-17T00:00:00Z",
            &[(
                "codex|w|ada@example.com",
                history(&[("2026-10-17", 3, 0.3, "gpt-5")]),
            )],
        )];
        let combined = combined(&Histories::new(), &homes, &peers, day("2026-10-17"));
        assert_eq!(combined["home"].days[&day("2026-10-17")].tokens, 3);
    }

    #[test]
    fn the_newest_file_of_a_device_wins() {
        let older = document(DEVICE, "2026-10-16T00:00:00Z", &[]);
        let newer = document(DEVICE, "2026-10-17T00:00:00Z", &[]);
        let kept = newest_by_device(vec![newer.clone(), older]);
        assert_eq!(kept, vec![newer]);
    }

    #[test]
    fn a_document_reads_back_as_written() {
        let written = document(
            DEVICE,
            "2026-10-17T01:02:03Z",
            &[("claude|a", history(&[("2026-10-17", 10, 1.25, "opus")]))],
        );
        let bytes = serde_json::to_vec_pretty(&written).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(r#""schema": "claude-switch.history.v1""#));
        assert!(text.contains(r#""2026-10-17": {"#));
        assert_eq!(parse(&bytes).unwrap(), written);
    }

    #[test]
    fn what_claude_switch_would_not_write_is_refused() {
        let good = document(DEVICE, "2026-10-17T00:00:00Z", &[]);
        let refused = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut value = serde_json::to_value(&good).unwrap();
            change(&mut value);
            parse(&serde_json::to_vec(&value).unwrap()).is_err()
        };
        assert!(parse(&serde_json::to_vec(&good).unwrap()).is_ok());
        assert!(refused(
            &|value| value["schema"] = "claude-switch.history.v2".into()
        ));
        assert!(refused(&|value| value["deviceId"] = "../../escape".into()));
        assert!(refused(&|value| value["deviceName"] = " ".into()));
        assert!(refused(&|value| {
            value["accounts"]["ada@example.com"] = serde_json::json!({});
        }));
        assert!(refused(&|value| {
            value["accounts"][account_id("claude|a")] = serde_json::json!({
                "days": { "2026-10-17": { "tokens": 1, "cost": -1.0, "models": {} } }
            });
        }));
        assert!(refused(&|value| {
            value["accounts"][account_id("claude|a")] = serde_json::json!({
                "days": { "2026-13-17": { "tokens": 1, "cost": 1.0, "models": {} } }
            });
        }));
        assert!(parse(b"not json").is_err());
    }
}
