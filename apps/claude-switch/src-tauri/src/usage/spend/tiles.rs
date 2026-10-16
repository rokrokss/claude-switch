//! A profile's usage history as metrics: what today, yesterday and the last
//! 30 days cost, with what each model accounts for, and a trend of tokens a
//! day. After OpenUsage's `SpendTileMapper` (MIT,
//! <https://github.com/robinebers/openusage>).
//!
//! A period without usage reports nothing rather than `$0.00 · 0 tokens`:
//! a zero can't be told from logs not yet read, and would contradict a live
//! session meter.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Duration, NaiveDate, TimeZone};

use super::daily::{UsageHistory, PREVIOUS_DAYS};
use crate::usage::metrics::{
    id, ChartPoint, Metric, MetricValue, ModelSpend, Reading, SpendBreakdown, Unit,
};

/// The metrics `history` reports as of `now`, in the panel's order.
pub fn metrics<Tz: TimeZone>(history: &UsageHistory, now: &DateTime<Tz>) -> Vec<Metric> {
    let today = now.date_naive();
    let mut metrics = Vec::new();
    for (metric_id, label, day) in [
        (id::TODAY, "Today", Some(today)),
        (id::YESTERDAY, "Yesterday", today.pred_opt()),
    ] {
        let Some(day) = day else {
            continue;
        };
        if let Some(usage) = history.days.get(&day) {
            metrics.extend(spend(
                metric_id,
                label,
                history,
                &[day],
                usage.tokens,
                usage.cost,
            ));
        }
    }
    let days: Vec<NaiveDate> = history.days.keys().copied().collect();
    let tokens = history.days.values().map(|day| day.tokens).sum();
    let cost = history.days.values().map(|day| day.cost).sum();
    metrics.extend(spend(
        id::LAST_30_DAYS,
        "Last 30 days",
        history,
        &days,
        tokens,
        cost,
    ));
    metrics.extend(trend(history, today));
    metrics
}

/// A spend metric for `days`, or `None` when they saw no usage.
fn spend(
    metric_id: &str,
    label: &str,
    history: &UsageHistory,
    days: &[NaiveDate],
    tokens: u64,
    cost: f64,
) -> Option<Metric> {
    if tokens == 0 && cost <= 0.0 {
        return None;
    }
    let collect =
        |by_day: &std::collections::BTreeMap<NaiveDate, BTreeSet<String>>| -> Vec<String> {
            days.iter()
                .filter_map(|day| by_day.get(day))
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
    Some(Metric {
        id: metric_id.to_owned(),
        label: label.to_owned(),
        value: MetricValue::Values {
            values: vec![
                Reading {
                    amount: cost,
                    unit: Unit::Money {
                        currency: "USD".to_owned(),
                    },
                },
                Reading {
                    amount: tokens as f64,
                    unit: Unit::Tokens,
                },
            ],
            expires_at: Vec::new(),
            breakdown: Some(SpendBreakdown {
                models: models(history, days),
                unknown_models: collect(&history.unknown_models),
                fallback_models: collect(&history.fallback_models),
            }),
        },
        state: None,
    })
}

/// Tokens a day for today and the [`PREVIOUS_DAYS`] before it, oldest
/// first, idle days as zero so the days stay true to the calendar. `None`
/// when the whole window was idle.
fn trend(history: &UsageHistory, today: NaiveDate) -> Option<Metric> {
    let points: Vec<ChartPoint> = (0..=PREVIOUS_DAYS)
        .rev()
        .map(|back| {
            let day = today - Duration::days(back);
            ChartPoint {
                date: day.format("%Y-%m-%d").to_string(),
                value: history
                    .days
                    .get(&day)
                    .map_or(0.0, |usage| usage.tokens as f64),
            }
        })
        .collect();
    if !points.iter().any(|point| point.value > 0.0) {
        return None;
    }
    Some(Metric {
        id: id::TREND.to_owned(),
        label: "Usage trend".to_owned(),
        value: MetricValue::Chart {
            unit: Unit::Tokens,
            points,
        },
        state: None,
    })
}

const UNATTRIBUTED: &str = "Unattributed";
const OTHER: &str = "Other";
const AUTO_REVIEW: &str = "codex-auto-review";
/// Models named before the rest fold into Other.
const NAMED_MODELS: usize = 5;
/// A model under this share of the period folds into Other.
const MIN_SHARE: f64 = 0.05;

/// What each model used on `days` accounts for. Spellings that differ only
/// in case are one model, named after the spelling with the most tokens.
fn models(history: &UsageHistory, days: &[NaiveDate]) -> Vec<ModelSpend> {
    let mut by_model: HashMap<String, Group> = HashMap::new();
    for usage in days.iter().filter_map(|day| history.days.get(day)) {
        for (model, used) in &usage.models {
            if used.tokens == 0 && used.cost <= 0.0 {
                continue;
            }
            let name = match model.trim() {
                "" => UNATTRIBUTED,
                name => name,
            };
            by_model
                .entry(name.to_lowercase())
                .or_default()
                .add(name, used.tokens, used.cost);
        }
    }
    let mut entries: Vec<ModelSpend> = by_model
        .into_iter()
        .map(|(key, group)| {
            let name = group.name.best().unwrap_or(key);
            group.entry(name)
        })
        .collect();
    entries.sort_by(spend_order);
    fold(entries)
}

/// Keeps auto-review named whatever its share, folds Unattributed and
/// anything under [`MIN_SHARE`] or past the [`NAMED_MODELS`]th into Other.
fn fold(entries: Vec<ModelSpend>) -> Vec<ModelSpend> {
    let cost_total: f64 = entries.iter().map(|entry| entry.cost).sum();
    let token_total: u64 = entries.iter().map(|entry| entry.tokens).sum();
    let share = |entry: &ModelSpend| {
        if cost_total > 0.0 {
            entry.cost / cost_total
        } else if token_total > 0 {
            entry.tokens as f64 / token_total as f64
        } else {
            0.0
        }
    };
    let mut visible = Vec::new();
    let mut other = Group::default();
    let mut named = 0;
    for entry in entries {
        if entry.model == AUTO_REVIEW {
            visible.push(entry);
        } else if entry.model.eq_ignore_ascii_case(UNATTRIBUTED)
            || share(&entry) < MIN_SHARE
            || named >= NAMED_MODELS
        {
            other.fold(&entry);
        } else {
            visible.push(entry);
            named += 1;
        }
    }
    if other.tokens > 0 || other.cost > 0.0 {
        visible.push(other.entry(OTHER.to_owned()));
    }
    visible
}

/// Costliest first, then most tokens, then by name.
fn spend_order(left: &ModelSpend, right: &ModelSpend) -> Ordering {
    right
        .cost
        .partial_cmp(&left.cost)
        .unwrap_or(Ordering::Equal)
        .then(right.tokens.cmp(&left.tokens))
        .then_with(|| left.model.to_lowercase().cmp(&right.model.to_lowercase()))
        .then_with(|| left.model.cmp(&right.model))
}

/// Picks a display spelling among those seen: the one with the most tokens,
/// then the all-lowercase one, then the first by name.
#[derive(Default)]
struct Vote {
    weights: HashMap<String, u64>,
}

impl Vote {
    fn note(&mut self, spelling: &str, tokens: u64) {
        // An entry without tokens still has a say.
        *self.weights.entry(spelling.to_owned()).or_default() += tokens.max(1);
    }

    fn best(&self) -> Option<String> {
        self.weights
            .iter()
            .min_by(|(left, left_weight), (right, right_weight)| {
                right_weight
                    .cmp(left_weight)
                    .then_with(|| {
                        (**right == right.to_lowercase()).cmp(&(**left == left.to_lowercase()))
                    })
                    .then_with(|| left.cmp(right))
            })
            .map(|(spelling, _)| spelling.clone())
    }
}

/// One breakdown row in the making: a model under its spellings, or Other
/// with the models folded into it.
#[derive(Default)]
struct Group {
    tokens: u64,
    cost: f64,
    name: Vote,
    /// By lowercased name: tokens, cost, and how it was spelt.
    variants: HashMap<String, (u64, f64, Vote)>,
}

impl Group {
    fn add(&mut self, spelling: &str, tokens: u64, cost: f64) {
        self.tokens += tokens;
        self.cost += cost;
        self.name.note(spelling, tokens);
        self.variant(spelling, tokens, cost);
    }

    fn fold(&mut self, entry: &ModelSpend) {
        self.tokens += entry.tokens;
        self.cost += entry.cost;
        self.variant(&entry.model, entry.tokens, entry.cost);
    }

    fn variant(&mut self, spelling: &str, tokens: u64, cost: f64) {
        let variant = self.variants.entry(spelling.to_lowercase()).or_default();
        variant.0 += tokens;
        variant.1 += cost;
        variant.2.note(spelling, tokens);
    }

    /// The row, named `model`. A single variant spelt like the row is no
    /// breakdown, and is left out.
    fn entry(self, model: String) -> ModelSpend {
        let mut variants: Vec<ModelSpend> = self
            .variants
            .into_iter()
            .map(|(key, (tokens, cost, vote))| ModelSpend {
                model: vote.best().unwrap_or(key),
                tokens,
                cost: to_cents(cost),
                variants: Vec::new(),
            })
            .collect();
        variants.sort_by(spend_order);
        if variants.len() == 1 && variants[0].model.to_lowercase() == model.to_lowercase() {
            variants.clear();
        }
        ModelSpend {
            model,
            tokens: self.tokens,
            cost: to_cents(self.cost),
            variants,
        }
    }
}

fn to_cents(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};

    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn now() -> DateTime<FixedOffset> {
        "2026-10-17T12:00:00+09:00".parse().unwrap()
    }

    fn values(metric: &Metric) -> (f64, f64, &SpendBreakdown) {
        let MetricValue::Values {
            values, breakdown, ..
        } = &metric.value
        else {
            panic!("{} is a reading", metric.id);
        };
        (
            values[0].amount,
            values[1].amount,
            breakdown.as_ref().unwrap(),
        )
    }

    #[test]
    fn each_period_with_usage_reports_cost_and_tokens() {
        let mut history = UsageHistory::default();
        history.add(day("2026-10-17"), 1_000, 1.5, "claude-opus-4-8", None);
        history.add(day("2026-10-15"), 3_000, 2.0, "claude-sonnet-4-6", None);
        history.add_unknown(day("2026-10-15"), "mystery-1");
        let metrics = metrics(&history, &now());
        let ids: Vec<&str> = metrics.iter().map(|metric| metric.id.as_str()).collect();
        // Yesterday saw nothing, so says nothing.
        assert_eq!(ids, ["today", "last30", "trend"]);
        let (cost, tokens, breakdown) = values(&metrics[0]);
        assert_eq!((cost, tokens), (1.5, 1_000.0));
        assert!(breakdown.unknown_models.is_empty());
        let (cost, tokens, breakdown) = values(&metrics[1]);
        assert_eq!((cost, tokens), (3.5, 4_000.0));
        assert_eq!(breakdown.unknown_models, ["mystery-1"]);
        assert_eq!(metrics[1].label, "Last 30 days");
    }

    #[test]
    fn the_trend_covers_31_days_oldest_first() {
        let mut history = UsageHistory::default();
        history.add(day("2026-10-16"), 500, 0.1, "a", None);
        let metrics = metrics(&history, &now());
        let MetricValue::Chart { points, unit } = &metrics.last().unwrap().value else {
            panic!("the trend is a chart");
        };
        assert_eq!(*unit, Unit::Tokens);
        assert_eq!(points.len(), 31);
        assert_eq!(points[0].date, "2026-09-17");
        assert_eq!(points[30].date, "2026-10-17");
        assert_eq!((points[29].value, points[30].value), (500.0, 0.0));
        assert!(super::metrics(&UsageHistory::default(), &now()).is_empty());
    }

    #[test]
    fn today_is_the_local_day() {
        let mut history = UsageHistory::default();
        history.add(day("2026-10-16"), 10, 0.1, "a", None);
        // 23:30 UTC on the 16th is already the 17th in Seoul.
        let late = "2026-10-16T23:30:00Z".parse::<DateTime<Utc>>().unwrap();
        let in_seoul = metrics(
            &history,
            &late.with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap()),
        );
        assert_eq!(in_seoul[0].id, "yesterday");
        assert_eq!(metrics(&history, &late)[0].id, "today");
    }

    #[test]
    fn small_models_and_those_past_the_fifth_fold_into_other() {
        let mut history = UsageHistory::default();
        let today = day("2026-10-17");
        for (index, model) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            history.add(today, 100, 10.0 - index as f64, model, None);
        }
        history.add(today, 100, 0.5, "tiny", None);
        history.add(today, 50, 3.0, "Unattributed", None);
        history.add(today, 10, 0.0, AUTO_REVIEW, None);
        let metrics = metrics(&history, &now());
        let (_, _, breakdown) = values(&metrics[0]);
        let names: Vec<&str> = breakdown
            .models
            .iter()
            .map(|model| model.model.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c", "d", "e", AUTO_REVIEW, "Other"]);
        let other = breakdown.models.last().unwrap();
        let folded: Vec<&str> = other
            .variants
            .iter()
            .map(|model| model.model.as_str())
            .collect();
        assert_eq!(folded, ["f", "Unattributed", "tiny"]);
        assert_eq!(other.cost, 8.5);
    }

    #[test]
    fn spellings_differing_in_case_are_one_model() {
        let mut history = UsageHistory::default();
        history.add(day("2026-10-17"), 100, 1.0, "GLM-5.2", None);
        history.add(day("2026-10-16"), 300, 1.0, "glm-5.2", None);
        let metrics = metrics(&history, &now());
        assert_eq!(metrics[2].id, "last30");
        let (_, _, breakdown) = values(&metrics[2]);
        // One row, spelt as the spelling with more tokens.
        assert_eq!(breakdown.models.len(), 1);
        assert_eq!(breakdown.models[0].model, "glm-5.2");
        assert_eq!(breakdown.models[0].tokens, 400);
        assert!(breakdown.models[0].variants.is_empty());
    }

    #[test]
    fn fallback_models_are_named_for_their_period() {
        let mut history = UsageHistory::default();
        history.add(day("2026-10-16"), 100, 1.0, "gpt-9", Some("gpt-5.5"));
        history.add_unknown(day("2026-10-16"), "gpt-9");
        let metrics = metrics(&history, &now());
        let (_, _, breakdown) = values(&metrics[0]);
        assert_eq!(metrics[0].id, "yesterday");
        assert_eq!(breakdown.fallback_models, ["gpt-5.5"]);
        assert_eq!(breakdown.unknown_models, ["gpt-9"]);
    }

    #[test]
    fn spend_metrics_round_trip_through_json() {
        let mut history = UsageHistory::default();
        history.add(day("2026-10-17"), 1_000, 1.5, "claude-opus-4-8", None);
        history.add(day("2026-10-17"), 10, 0.01, "gpt-9", Some("gpt-5.5"));
        history.add_unknown(day("2026-10-17"), "gpt-9");
        let metrics = metrics(&history, &now());
        let json = serde_json::to_string(&metrics).unwrap();
        for part in [
            r#""kind":"chart""#,
            r#""type":"tokens""#,
            r#""unknownModels":["gpt-9"]"#,
            r#""fallbackModels""#,
        ] {
            assert!(json.contains(part), "{part} in {json}");
        }
        let back: Vec<Metric> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, metrics);
    }
}
