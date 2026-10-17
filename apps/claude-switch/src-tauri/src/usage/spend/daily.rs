//! A profile's priced usage by local calendar day: tokens, dollars and what
//! each model accounts for, with the models no list could price. Shared by
//! the Claude and Codex scanners, after OpenUsage's
//! `DailyUsageAccumulator` (MIT, <https://github.com/robinebers/openusage>).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

/// Days counted before today: the window is today and the 30 before it.
pub const PREVIOUS_DAYS: i64 = 30;

/// Where the window starts, in milliseconds since the epoch: the start of
/// the local day [`PREVIOUS_DAYS`] before `now`'s.
pub fn window_start<Tz: TimeZone>(now: &DateTime<Tz>) -> i64 {
    let first = now.date_naive() - Duration::days(PREVIOUS_DAYS);
    start_of(&now.timezone(), first)
}

/// The first instant of local day `day`: midnight, or the first hour that
/// exists where a clock change skips midnight.
fn start_of<Tz: TimeZone>(zone: &Tz, day: NaiveDate) -> i64 {
    (0..24)
        .find_map(|hour| {
            let time = NaiveTime::from_hms_opt(hour, 0, 0)?;
            zone.from_local_datetime(&day.and_time(time)).earliest()
        })
        .map_or_else(
            || day.and_time(NaiveTime::MIN).and_utc().timestamp_millis(),
            |start| start.timestamp_millis(),
        )
}

/// A log's ISO 8601 timestamp in milliseconds since the epoch, read the way
/// OpenUsage reads them: a space may stand for the `T`, ` UTC` for `Z`, a
/// time without a zone is UTC, and fractions are cut to milliseconds.
pub fn timestamp_millis(raw: &str) -> Option<i64> {
    let mut text = raw.trim().to_owned();
    if text.is_empty() {
        return None;
    }
    if text.contains(' ') && starts_with_shape(&text, "dddd-dd-dd dd:dd:dd") {
        text.replace_range(10..11, "T");
    }
    if let Some(utc) = text.strip_suffix(" UTC") {
        text = format!("{utc}Z");
    }
    let text = normalized_iso(&text).unwrap_or(text);
    DateTime::parse_from_rfc3339(&text)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// `text` as `<date>T<time>[.mmm]<zone>`, if it is a date and time with an
/// optional fraction and an optional `Z` or `±hh:mm` zone.
fn normalized_iso(text: &str) -> Option<String> {
    let head = text
        .get(..19)
        .filter(|head| has_shape(head, "dddd-dd-ddTdd:dd:dd"))?;
    let mut rest = &text[19..];
    let mut fraction = String::new();
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let kept: String = after_dot[..digits.min(3)].to_owned();
        fraction = format!(".{kept:0<3}");
        rest = &after_dot[digits..];
    }
    let zone = match rest {
        "" | "Z" => "Z",
        _ if has_shape(rest, "+dd:dd") || has_shape(rest, "-dd:dd") => rest,
        _ => return None,
    };
    Some(format!("{head}{fraction}{zone}"))
}

/// Whether `text` is `shape`, where `d` is a digit and anything else itself.
fn has_shape(text: &str, shape: &str) -> bool {
    text.len() == shape.len() && starts_with_shape(text, shape)
}

fn starts_with_shape(text: &str, shape: &str) -> bool {
    text.len() >= shape.len()
        && text.bytes().zip(shape.bytes()).all(|(byte, want)| {
            if want == b'd' {
                byte.is_ascii_digit()
            } else {
                byte == want
            }
        })
}

/// The local day of `millis` since the epoch.
pub fn day_of<Tz: TimeZone>(zone: &Tz, millis: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp_millis(millis).map(|at| at.with_timezone(zone).date_naive())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DayUsage {
    pub tokens: u64,
    pub cost: f64,
    pub models: BTreeMap<String, ModelUsage>,
}

/// Only priced usage is counted: a model no list can price is left out of
/// every figure, as tokens beside dollars that ignore some of them would
/// mislead, and named in `unknown_models` instead.
///
/// Serialized as is into the file iCloud sync keeps per Mac
/// ([`crate::usage::sync::document`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageHistory {
    pub days: BTreeMap<NaiveDate, DayUsage>,
    /// Models used each day that no list could price, whether left out or
    /// estimated at the fallback model's rates.
    pub unknown_models: BTreeMap<NaiveDate, BTreeSet<String>>,
    /// The fallback models whose rates priced some of each day's usage.
    pub fallback_models: BTreeMap<NaiveDate, BTreeSet<String>>,
}

impl UsageHistory {
    /// Counts a priced request of `model` on `day`.
    pub fn add(
        &mut self,
        day: NaiveDate,
        tokens: u64,
        cost: f64,
        model: &str,
        fallback: Option<&str>,
    ) {
        let usage = self.days.entry(day).or_default();
        usage.tokens += tokens;
        usage.cost += cost;
        let by_model = usage.models.entry(model.to_owned()).or_default();
        by_model.tokens += tokens;
        by_model.cost += cost;
        if let Some(fallback) = fallback {
            self.fallback_models
                .entry(day)
                .or_default()
                .insert(fallback.to_owned());
        }
    }

    /// Notes that `model`, used on `day` with tokens, has no known price.
    pub fn add_unknown(&mut self, day: NaiveDate, model: &str) {
        self.unknown_models
            .entry(day)
            .or_default()
            .insert(model.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};

    use super::*;

    #[test]
    fn the_window_starts_at_local_midnight_thirty_days_back() {
        let seoul = FixedOffset::east_opt(9 * 3600).unwrap();
        // 01:00 on the 17th in Seoul is still the 16th in UTC.
        let now = "2026-10-16T16:00:00Z"
            .parse::<DateTime<Utc>>()
            .unwrap()
            .with_timezone(&seoul);
        let start = window_start(&now);
        let expected = "2026-09-16T15:00:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(start, expected.timestamp_millis());
        assert_eq!(day_of(&seoul, start), NaiveDate::from_ymd_opt(2026, 9, 17));
        assert_eq!(
            day_of(&seoul, start - 1),
            NaiveDate::from_ymd_opt(2026, 9, 16)
        );
    }

    #[test]
    fn usage_adds_up_by_day_and_model() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let mut history = UsageHistory::default();
        history.add(day, 10, 1.0, "a", None);
        history.add(day, 5, 0.5, "a", Some("gpt-5.5"));
        history.add(day, 1, 0.25, "b", None);
        history.add_unknown(day, "mystery");
        let usage = &history.days[&day];
        assert_eq!((usage.tokens, usage.cost), (16, 1.75));
        assert_eq!(usage.models["a"].tokens, 15);
        assert!(history.fallback_models[&day].contains("gpt-5.5"));
        assert!(history.unknown_models[&day].contains("mystery"));
    }

    #[test]
    fn timestamps_read_leniently_to_the_millisecond() {
        let at = |text: &str| {
            DateTime::from_timestamp_millis(timestamp_millis(text).unwrap())
                .unwrap()
                .to_rfc3339()
        };
        assert_eq!(
            at("2026-10-17T01:02:03.456789Z"),
            "2026-10-17T01:02:03.456+00:00"
        );
        assert_eq!(
            at(" 2026-10-17T01:02:03.4+09:00 "),
            "2026-10-16T16:02:03.400+00:00"
        );
        assert_eq!(at("2026-10-17 01:02:03 UTC"), "2026-10-17T01:02:03+00:00");
        assert_eq!(at("2026-10-17T01:02:03"), "2026-10-17T01:02:03+00:00");
        assert_eq!(timestamp_millis(""), None);
        assert_eq!(timestamp_millis("yesterday"), None);
        assert_eq!(timestamp_millis("2026-13-17T01:02:03Z"), None);
    }
}
