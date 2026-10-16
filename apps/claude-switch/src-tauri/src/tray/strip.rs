//! What the menu bar strip says: for each profile with usage, its app's mark
//! and the share used of each of its two windows. Pure, so the rules can be
//! tested apart from the drawing.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::TrayEntry;
use crate::app_kind::AppKind;
use crate::usage::Window;

/// The strip, one group per profile that has a value to show, in the order
/// the profiles were given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    pub groups: Vec<Group>,
}

/// A profile's segment of the strip: its app's mark, then one or two values
/// (`"18%"`), its first window's before its second's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub app: AppKind,
    pub values: Vec<String>,
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

/// The strip for `entries` at `now`. A window without data is left out, and
/// so is a profile none of whose windows has any: the strip never shows a
/// placeholder.
pub fn build(entries: &[TrayEntry], now: DateTime<Utc>) -> Strip {
    let groups = entries
        .iter()
        .filter_map(|entry| {
            let values: Vec<String> = windows(entry)
                .into_iter()
                .filter_map(|(_, window)| used_percent(window, now))
                .map(|percent| format!("{percent}%"))
                .collect();
            (!values.is_empty()).then_some(Group {
                app: entry.app,
                values,
            })
        })
        .collect();
    Strip { groups }
}

/// The item's tooltip: which profile each group belongs to, since two
/// profiles of one app wear the same mark.
pub fn tooltip(entries: &[TrayEntry], now: DateTime<Utc>) -> String {
    let mut lines = vec!["claude-switch".to_owned()];
    for entry in entries {
        let values: Vec<String> = windows(entry)
            .into_iter()
            .filter_map(|(name, window)| Some(format!("{name} {}%", used_percent(window, now)?)))
            .collect();
        if !values.is_empty() {
            lines.push(format!("{}: {}", entry.name, values.join(", ")));
        }
    }
    lines.join("\n")
}

/// The profile's windows with their names, in the order the strip shows
/// them: the primary slot, then the secondary. Claude's slots are the 5-hour
/// and the weekly window; a Codex plan may put either length in either slot,
/// so its windows are named after the length the payload reports.
fn windows(entry: &TrayEntry) -> Vec<(String, &Window)> {
    let Some(quota) = entry.usage.as_ref().and_then(|usage| usage.quota.as_ref()) else {
        return Vec::new();
    };
    let slots = [
        ("5-hour", quota.primary.as_ref()),
        ("weekly", quota.secondary.as_ref()),
    ];
    slots
        .into_iter()
        .filter_map(|(claude_name, window)| {
            let window = window?;
            let name = match (entry.app, window.window_duration_mins) {
                (AppKind::Claude, _) => claude_name.to_owned(),
                (AppKind::Codex, Some(10080)) => "weekly".to_owned(),
                (AppKind::Codex, Some(minutes)) if minutes % 60 == 0 => {
                    format!("{}-hour", minutes / 60)
                }
                (AppKind::Codex, Some(minutes)) => format!("{minutes}-minute"),
                (AppKind::Codex, None) => "usage".to_owned(),
            };
            Some((name, window))
        })
        .collect()
}

/// Whole percent of `window` used at `now`. A window whose reset has passed
/// since it was fetched has started over, so it shows 0 until the next
/// fetch rather than a figure that no longer holds.
fn used_percent(window: &Window, now: DateTime<Utc>) -> Option<i64> {
    let utilization = window.utilization?;
    if !utilization.is_finite() {
        return None;
    }
    let reset = window
        .resets_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok());
    if reset.is_some_and(|at| at <= now) {
        return Some(0);
    }
    Some(utilization.round().max(0.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{ProfileUsage, QuotaUsage};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn window(utilization: Option<f32>, resets_at: Option<&str>) -> Window {
        Window {
            window_duration_mins: None,
            label: None,
            utilization,
            resets_at: resets_at.map(str::to_owned),
        }
    }

    fn entry(app: AppKind, primary: Option<Window>, secondary: Option<Window>) -> TrayEntry {
        TrayEntry {
            id: "id".into(),
            app,
            name: "Work".into(),
            command: "claude-work".into(),
            usage: Some(ProfileUsage {
                quota: Some(QuotaUsage {
                    primary,
                    secondary,
                    scoped_weekly: Vec::new(),
                    spend: None,
                    rate_limit_reset_credits: None,
                }),
                quota_error: None,
                fetched_at: "2026-10-16T11:58:00Z".into(),
            }),
            error: None,
        }
    }

    fn values(strip: &Strip) -> Vec<Vec<&str>> {
        strip
            .groups
            .iter()
            .map(|group| group.values.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn shows_both_windows_rounded() {
        let entries = [entry(
            AppKind::Claude,
            Some(window(Some(17.6), Some("2026-10-16T15:00:00Z"))),
            Some(window(Some(24.0), Some("2026-10-22T15:00:00Z"))),
        )];
        assert_eq!(values(&build(&entries, now())), [["18%", "24%"]]);
    }

    #[test]
    fn leaves_out_a_window_without_data() {
        let entries = [entry(
            AppKind::Codex,
            Some(window(None, None)),
            Some(window(Some(2.0), None)),
        )];
        let strip = build(&entries, now());
        assert_eq!(values(&strip), [["2%"]]);
        assert_eq!(strip.groups[0].app, AppKind::Codex);
    }

    #[test]
    fn leaves_out_a_profile_with_nothing_to_show() {
        let mut signed_out = entry(AppKind::Claude, None, None);
        signed_out.usage = None;
        let entries = [
            signed_out,
            entry(AppKind::Claude, Some(window(None, None)), None),
            entry(AppKind::Codex, Some(window(Some(5.0), None)), None),
        ];
        let strip = build(&entries, now());
        assert_eq!(values(&strip), [["5%"]]);
        assert_eq!(strip.groups[0].app, AppKind::Codex);
    }

    #[test]
    fn keeps_the_order_of_the_profiles() {
        let entries = [
            entry(AppKind::Claude, Some(window(Some(1.0), None)), None),
            entry(AppKind::Claude, Some(window(Some(2.0), None)), None),
            entry(AppKind::Codex, Some(window(Some(3.0), None)), None),
        ];
        assert_eq!(values(&build(&entries, now())), [["1%"], ["2%"], ["3%"]]);
    }

    #[test]
    fn a_window_past_its_reset_shows_zero() {
        let entries = [entry(
            AppKind::Claude,
            Some(window(Some(80.0), Some("2026-10-16T11:00:00Z"))),
            Some(window(Some(40.0), Some("2026-10-20T00:00:00Z"))),
        )];
        assert_eq!(values(&build(&entries, now())), [["0%", "40%"]]);
    }

    #[test]
    fn over_the_limit_shows_as_reported() {
        let entries = [entry(
            AppKind::Claude,
            Some(window(Some(103.4), None)),
            None,
        )];
        assert_eq!(values(&build(&entries, now())), [["103%"]]);
    }

    fn hidden(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn with_id(id: &str, mut entry: TrayEntry) -> TrayEntry {
        entry.id = id.into();
        entry.name = id.into();
        entry
    }

    #[test]
    fn leaves_out_the_profiles_hidden_from_the_menu_bar() {
        let entries = [
            with_id(
                "personal",
                entry(AppKind::Claude, Some(window(Some(1.0), None)), None),
            ),
            with_id(
                "work",
                entry(AppKind::Claude, Some(window(Some(2.0), None)), None),
            ),
            with_id(
                "default:codex",
                entry(AppKind::Codex, Some(window(Some(3.0), None)), None),
            ),
        ];
        let shown = in_menu_bar(&entries, &hidden(&["work"]));
        assert_eq!(values(&build(&shown, now())), [["1%"], ["3%"]]);
        assert_eq!(
            tooltip(&shown, now()),
            "claude-switch\npersonal: 5-hour 1%\ndefault:codex: usage 3%"
        );
    }

    #[test]
    fn hiding_an_unknown_profile_changes_nothing() {
        let entries = [entry(AppKind::Claude, Some(window(Some(1.0), None)), None)];
        let shown = in_menu_bar(&entries, &hidden(&["gone"]));
        assert_eq!(values(&build(&shown, now())), [["1%"]]);
    }

    #[test]
    fn with_every_profile_hidden_the_strip_is_empty() {
        let entries = [
            with_id(
                "a",
                entry(AppKind::Claude, Some(window(Some(1.0), None)), None),
            ),
            with_id(
                "b",
                entry(AppKind::Codex, Some(window(Some(2.0), None)), None),
            ),
        ];
        let shown = in_menu_bar(&entries, &hidden(&["a", "b"]));
        assert!(build(&shown, now()).groups.is_empty());
        assert_eq!(tooltip(&shown, now()), "claude-switch");
    }

    #[test]
    fn tooltip_names_each_profile() {
        let mut codex = entry(
            AppKind::Codex,
            Some(Window {
                window_duration_mins: Some(10080),
                ..window(Some(2.0), None)
            }),
            None,
        );
        codex.name = "ChatGPT".into();
        let entries = [codex];
        assert_eq!(
            tooltip(&entries, now()),
            "claude-switch\nChatGPT: weekly 2%"
        );
    }
}
