//! What the menu bar strip says: for each profile, its app's mark and the
//! values of the metrics starred for it (session and weekly unless the user
//! chose others). Pure, so the rules can be tested apart from the drawing.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::TrayEntry;
use crate::app_kind::AppKind;
use crate::settings::{MenuBarSettings, MeterStyle};
use crate::usage::metrics::{id, Metric, MetricValue, Unit};

/// At most this many of a profile's starred metrics are drawn: two stack
/// beside its mark.
const MAX_VALUES: usize = 2;
/// The Bars glyph holds this many bars.
const MAX_BARS: usize = 4;

/// The strip, one group per profile that has a value to show, in the order
/// the profiles were given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    pub groups: Vec<Group>,
}

/// A profile's segment of the strip: its app's mark, then one or two values
/// (`"18%"`) in the order they were starred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub app: AppKind,
    pub values: Vec<String>,
}

/// How the strip reads: which metrics are starred, and whether a meter shows
/// what's used or what's left.
pub struct Options<'a> {
    pub menu_bar: &'a MenuBarSettings,
    pub meter_style: MeterStyle,
}

/// The entries the strip and its tooltip draw on: all but those the user
/// took out of the menu bar (`hidden`, by entry id), in the order given.
pub fn in_menu_bar(entries: &[TrayEntry], hidden: &BTreeSet<String>) -> Vec<TrayEntry> {
    entries
        .iter()
        .filter(|entry| !hidden.contains(&entry.id))
        .cloned()
        .collect()
}

/// The strip for `entries` at `now`. A starred metric without data is left
/// out, and so is a profile none of whose starred metrics has any: the strip
/// never shows a placeholder.
pub fn build(entries: &[TrayEntry], options: &Options, now: DateTime<Utc>) -> Strip {
    let groups = entries
        .iter()
        .filter_map(|entry| {
            let values: Vec<String> = starred(entry, options)
                .into_iter()
                .filter_map(|metric| value(metric, options.meter_style, now))
                .collect();
            (!values.is_empty()).then_some(Group {
                app: entry.app,
                values,
            })
        })
        .collect();
    Strip { groups }
}

/// The Bars style's bars: the first four starred meters, across profiles
/// in order, each as the share shown (used or left).
pub fn bars(entries: &[TrayEntry], options: &Options, now: DateTime<Utc>) -> Vec<f64> {
    entries
        .iter()
        .flat_map(|entry| starred(entry, options))
        .filter_map(|metric| bar(metric, options.meter_style, now))
        .take(MAX_BARS)
        .collect()
}

fn bar(metric: &Metric, meter_style: MeterStyle, now: DateTime<Utc>) -> Option<f64> {
    let MetricValue::Meter {
        used,
        limit,
        resets_at,
        ..
    } = &metric.value
    else {
        return None;
    };
    if *limit <= 0.0 {
        return None;
    }
    let used = if has_reset(resets_at.as_deref(), now) {
        0.0
    } else {
        (used / limit).clamp(0.0, 1.0)
    };
    Some(match meter_style {
        MeterStyle::Used => used,
        MeterStyle::Left => 1.0 - used,
    })
}

/// Whether a window resetting at `resets_at` has reset by `now`.
fn has_reset(resets_at: Option<&str>, now: DateTime<Utc>) -> bool {
    resets_at
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| at <= now)
}

/// The item's tooltip: which profile each group belongs to, since two
/// profiles of one app wear the same mark.
pub fn tooltip(entries: &[TrayEntry], options: &Options, now: DateTime<Utc>) -> String {
    let mut lines = vec!["claude-switch".to_owned()];
    for entry in entries {
        let values: Vec<String> = starred(entry, options)
            .into_iter()
            .filter_map(|metric| {
                let value = value(metric, options.meter_style, now)?;
                Some(format!("{} {value}", metric.label))
            })
            .collect();
        if !values.is_empty() {
            lines.push(format!("{}: {}", entry.name, values.join(", ")));
        }
    }
    lines.join("\n")
}

/// The entry's starred metrics that it reports, in the order starred.
fn starred<'a>(entry: &'a TrayEntry, options: &Options) -> Vec<&'a Metric> {
    options
        .menu_bar
        .stars_for(&entry.id)
        .iter()
        .filter_map(|star| entry.metrics.iter().find(|metric| &metric.id == star))
        .take(MAX_VALUES)
        .collect()
}

/// A metric as the strip prints it, or `None` when it has nothing to print.
fn value(metric: &Metric, meter_style: MeterStyle, now: DateTime<Utc>) -> Option<String> {
    match &metric.value {
        MetricValue::Meter {
            used,
            limit,
            unit,
            resets_at,
            ..
        } => {
            // A window whose reset has passed since it was fetched has started
            // over: nothing used until the next fetch says otherwise.
            let used = if has_reset(resets_at.as_deref(), now) {
                0.0
            } else {
                *used
            };
            let shown = match meter_style {
                MeterStyle::Used => used,
                MeterStyle::Left => limit - used,
            };
            match unit {
                Unit::Percent => {
                    let percent = (shown / limit * 100.0).clamp(0.0, 100.0);
                    Some(format!("{}%", percent.round() as i64))
                }
                Unit::Money { currency } => Some(money(shown.max(0.0), currency)),
                Unit::Count | Unit::Tokens => Some(compact(shown.max(0.0))),
            }
        }
        MetricValue::Values { values, .. } => {
            let reading = values.first()?;
            let text = match &reading.unit {
                Unit::Money { currency } => money(reading.amount, currency),
                Unit::Count if metric.id == id::RESETS => {
                    format!("{} resets", compact(reading.amount))
                }
                Unit::Count | Unit::Tokens => compact(reading.amount),
                Unit::Percent => format!("{}%", reading.amount.round() as i64),
            };
            Some(text)
        }
        MetricValue::Chart { .. } => None,
    }
}

/// `$130` below a thousand, `$2.1K` above.
fn money(amount: f64, currency: &str) -> String {
    let symbol = match currency {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        _ => "",
    };
    let number = compact(amount);
    if symbol.is_empty() {
        format!("{number} {currency}")
    } else {
        format!("{symbol}{number}")
    }
}

/// Whole numbers below a thousand; one decimal of thousands or millions above.
fn compact(amount: f64) -> String {
    if amount >= 1_000_000.0 {
        format!("{:.1}M", amount / 1_000_000.0)
    } else if amount >= 1_000.0 {
        format!("{:.1}K", amount / 1_000.0)
    } else {
        format!("{}", amount.round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::metrics::Reading;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn percent(metric_id: &str, label: &str, used: f64, resets_at: Option<&str>) -> Metric {
        Metric {
            id: metric_id.into(),
            label: label.into(),
            value: MetricValue::Meter {
                used,
                limit: 100.0,
                unit: Unit::Percent,
                resets_at: resets_at.map(str::to_owned),
                period_secs: None,
                not_started: false,
            },
            state: None,
        }
    }

    fn session(used: f64) -> Metric {
        percent(id::SESSION, "Session", used, None)
    }

    fn weekly(used: f64) -> Metric {
        percent(id::WEEKLY, "Weekly", used, None)
    }

    fn entry(app: AppKind, metrics: Vec<Metric>) -> TrayEntry {
        TrayEntry {
            id: "id".into(),
            app,
            name: "Work".into(),
            command: "claude-work".into(),
            plan: None,
            usage: None,
            metrics,
            error: None,
            retry_at: None,
        }
    }

    fn with_id(id: &str, mut entry: TrayEntry) -> TrayEntry {
        entry.id = id.into();
        entry.name = id.into();
        entry
    }

    fn strip(
        entries: &[TrayEntry],
        menu_bar: &MenuBarSettings,
        style: MeterStyle,
    ) -> Vec<Vec<String>> {
        let options = Options {
            menu_bar,
            meter_style: style,
        };
        build(entries, &options, now())
            .groups
            .into_iter()
            .map(|group| group.values)
            .collect()
    }

    fn defaults(entries: &[TrayEntry]) -> Vec<Vec<String>> {
        strip(entries, &MenuBarSettings::default(), MeterStyle::Used)
    }

    #[test]
    fn shows_session_and_weekly_rounded() {
        let entries = [entry(AppKind::Claude, vec![session(17.6), weekly(24.0)])];
        assert_eq!(defaults(&entries), [["18%", "24%"]]);
    }

    #[test]
    fn leaves_out_a_starred_metric_without_data() {
        let entries = [entry(AppKind::Codex, vec![weekly(2.0)])];
        let built = build(
            &entries,
            &Options {
                menu_bar: &MenuBarSettings::default(),
                meter_style: MeterStyle::Used,
            },
            now(),
        );
        assert_eq!(built.groups[0].values, ["2%"]);
        assert_eq!(built.groups[0].app, AppKind::Codex);
    }

    #[test]
    fn leaves_out_a_profile_with_nothing_to_show() {
        let entries = [
            entry(AppKind::Claude, Vec::new()),
            entry(AppKind::Codex, vec![session(5.0)]),
        ];
        assert_eq!(defaults(&entries), [["5%"]]);
    }

    #[test]
    fn keeps_the_order_of_the_profiles() {
        let entries = [
            entry(AppKind::Claude, vec![session(1.0)]),
            entry(AppKind::Claude, vec![session(2.0)]),
            entry(AppKind::Codex, vec![session(3.0)]),
        ];
        assert_eq!(defaults(&entries), [["1%"], ["2%"], ["3%"]]);
    }

    #[test]
    fn a_window_past_its_reset_shows_zero() {
        let entries = [entry(
            AppKind::Claude,
            vec![
                percent(id::SESSION, "Session", 80.0, Some("2026-10-16T11:00:00Z")),
                percent(id::WEEKLY, "Weekly", 40.0, Some("2026-10-20T00:00:00Z")),
            ],
        )];
        assert_eq!(defaults(&entries), [["0%", "40%"]]);
    }

    #[test]
    fn a_percentage_stays_between_nothing_and_all() {
        let entries = [entry(AppKind::Claude, vec![session(103.4)])];
        assert_eq!(defaults(&entries), [["100%"]]);
    }

    #[test]
    fn left_shows_what_remains() {
        let entries = [entry(AppKind::Claude, vec![session(18.0), weekly(24.0)])];
        assert_eq!(
            strip(&entries, &MenuBarSettings::default(), MeterStyle::Left),
            [["82%", "76%"]]
        );
    }

    #[test]
    fn shows_the_starred_metrics_in_the_order_starred() {
        let mut menu_bar = MenuBarSettings::default();
        menu_bar
            .stars
            .insert("id".into(), vec![id::WEEKLY.into(), "weekly.fable".into()]);
        let entries = [entry(
            AppKind::Claude,
            vec![
                session(10.0),
                weekly(20.0),
                percent("weekly.fable", "Weekly · Fable", 30.0, None),
            ],
        )];
        assert_eq!(
            strip(&entries, &menu_bar, MeterStyle::Used),
            [["20%", "30%"]]
        );
    }

    #[test]
    fn bars_are_the_first_four_starred_meters() {
        let entries = [
            entry(AppKind::Claude, vec![session(20.0), weekly(40.0)]),
            entry(AppKind::Claude, vec![session(60.0), weekly(80.0)]),
            entry(AppKind::Codex, vec![weekly(10.0)]),
        ];
        let options = Options {
            menu_bar: &MenuBarSettings::default(),
            meter_style: MeterStyle::Used,
        };
        assert_eq!(bars(&entries, &options, now()), [0.2, 0.4, 0.6, 0.8]);
        let left = Options {
            meter_style: MeterStyle::Left,
            ..options
        };
        assert_eq!(bars(&entries[2..], &left, now()), [0.9]);
    }

    #[test]
    fn a_profile_with_nothing_starred_is_left_out() {
        let mut menu_bar = MenuBarSettings::default();
        menu_bar.stars.insert("id".into(), Vec::new());
        let entries = [entry(AppKind::Claude, vec![session(10.0)])];
        assert!(strip(&entries, &menu_bar, MeterStyle::Used).is_empty());
    }

    #[test]
    fn money_and_counts_read_compactly() {
        let mut menu_bar = MenuBarSettings::default();
        menu_bar
            .stars
            .insert("id".into(), vec![id::EXTRA.into(), id::RESETS.into()]);
        let entries = [entry(
            AppKind::Codex,
            vec![
                Metric {
                    id: id::EXTRA.into(),
                    label: "Extra usage".into(),
                    value: MetricValue::Values {
                        values: vec![Reading {
                            amount: 2140.0,
                            unit: Unit::Money {
                                currency: "USD".into(),
                            },
                        }],
                        expires_at: Vec::new(),
                        breakdown: None,
                    },
                    state: None,
                },
                Metric {
                    id: id::RESETS.into(),
                    label: "Rate limit resets".into(),
                    value: MetricValue::Values {
                        values: vec![Reading {
                            amount: 2.0,
                            unit: Unit::Count,
                        }],
                        expires_at: Vec::new(),
                        breakdown: None,
                    },
                    state: None,
                },
            ],
        )];
        assert_eq!(
            strip(&entries, &menu_bar, MeterStyle::Used),
            [["$2.1K", "2 resets"]]
        );
    }

    fn hidden(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn leaves_out_the_profiles_hidden_from_the_menu_bar() {
        let entries = [
            with_id("personal", entry(AppKind::Claude, vec![session(1.0)])),
            with_id("work", entry(AppKind::Claude, vec![session(2.0)])),
            with_id("default:codex", entry(AppKind::Codex, vec![session(3.0)])),
        ];
        let shown = in_menu_bar(&entries, &hidden(&["work"]));
        assert_eq!(defaults(&shown), [["1%"], ["3%"]]);
        let options = Options {
            menu_bar: &MenuBarSettings::default(),
            meter_style: MeterStyle::Used,
        };
        assert_eq!(
            tooltip(&shown, &options, now()),
            "claude-switch\npersonal: Session 1%\ndefault:codex: Session 3%"
        );
    }

    #[test]
    fn hiding_an_unknown_profile_changes_nothing() {
        let entries = [entry(AppKind::Claude, vec![session(1.0)])];
        let shown = in_menu_bar(&entries, &hidden(&["gone"]));
        assert_eq!(defaults(&shown), [["1%"]]);
    }

    #[test]
    fn with_every_profile_hidden_the_strip_is_empty() {
        let entries = [
            with_id("a", entry(AppKind::Claude, vec![session(1.0)])),
            with_id("b", entry(AppKind::Codex, vec![session(2.0)])),
        ];
        let shown = in_menu_bar(&entries, &hidden(&["a", "b"]));
        assert!(defaults(&shown).is_empty());
    }

    #[test]
    fn tooltip_names_each_profile() {
        let mut codex = entry(AppKind::Codex, vec![weekly(2.0)]);
        codex.name = "ChatGPT".into();
        let options = Options {
            menu_bar: &MenuBarSettings::default(),
            meter_style: MeterStyle::Used,
        };
        assert_eq!(
            tooltip(&[codex], &options, now()),
            "claude-switch\nChatGPT: Weekly 2%"
        );
    }
}
