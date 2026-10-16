//! Notifications when a profile's usage runs low: after every fetch of the
//! menu bar's figures, each meter of every profile is checked against the
//! triggers the user turned on in Settings ([`rules`]), and what fires is
//! posted as a macOS notification from claude-switch. A click on one opens
//! the menu bar panel.
//!
//! Profiles hidden from the menu bar are checked too: hiding one only keeps
//! it out of the strip, and the panel a notification opens still lists it.
//! A metric turned off in the profile's layout is not.

#[cfg(target_os = "macos")]
mod macos;
mod rules;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::app_state;
use crate::error::AppResult;
use crate::settings::LayoutSettings;
use crate::tray::TrayEntry;
use crate::usage::metrics::MetricValue;
use crate::usage::pace;
use rules::{Alert, Evaluator, Reading};

/// Whether macOS lets claude-switch post notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    /// The user hasn't been asked yet.
    NotDetermined,
    Denied,
    Granted,
    /// Not from this build: one run outside an app bundle can't post any.
    Unavailable,
}

/// What has been alerted since launch. Behind an async lock: an evaluation
/// waits on deliveries, and the next must not start before it ends.
#[derive(Default)]
pub struct NotifierState(tokio::sync::Mutex<Evaluator>);

/// Called once, from setup.
pub fn setup(app: &tauri::App) {
    app.manage(NotifierState::default());
    #[cfg(target_os = "macos")]
    macos::listen_for_clicks(app.handle());
}

/// Checks `entries`, just fetched, for anything to notify about.
pub fn after_refresh(app: &AppHandle, entries: &[TrayEntry]) {
    // Unreadable settings mean every trigger is off, as by default.
    let settings = app_state::load()
        .map(|state| state.settings)
        .unwrap_or_default();
    let readings = readings(entries, &settings.layout, Utc::now());
    let toggles = settings.notifications;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NotifierState>();
        let mut evaluator = state.0.lock().await;
        evaluator.evaluate(readings, &toggles, deliver).await;
    });
}

async fn deliver(alert: Alert) -> bool {
    #[cfg(target_os = "macos")]
    return macos::post(&alert).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = alert;
        false
    }
}

/// Every meter of `entries` the layout shows, as of `now`.
fn readings(entries: &[TrayEntry], layout: &LayoutSettings, now: DateTime<Utc>) -> Vec<Reading> {
    let mut readings = Vec::new();
    for entry in entries {
        let shown = layout.profiles.get(&entry.id);
        for metric in &entry.metrics {
            let MetricValue::Meter {
                used,
                limit,
                resets_at,
                ..
            } = &metric.value
            else {
                continue;
            };
            if shown.is_some_and(|shown| {
                !shown.always_visible.contains(&metric.id) && !shown.on_demand.contains(&metric.id)
            }) {
                continue;
            }
            let remaining = if *limit > 0.0 {
                ((limit - used) / limit).clamp(0.0, 1.0)
            } else {
                0.0
            };
            readings.push(Reading {
                key: format!("{}.{}", entry.id, metric.id),
                subtitle: format!("{} {}", entry.name, metric.label),
                state: pace::meter_state(&metric.value, now),
                remaining,
                resets_at: resets_at
                    .as_deref()
                    .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.with_timezone(&Utc)),
            });
        }
    }
    readings
}

/// Whether claude-switch may post notifications.
#[tauri::command]
pub async fn notification_permission() -> Permission {
    #[cfg(target_os = "macos")]
    return macos::permission().await;
    #[cfg(not(target_os = "macos"))]
    Permission::Unavailable
}

/// Asks the user to let claude-switch post notifications, if they haven't
/// answered yet, and says what they allow now.
#[tauri::command]
pub async fn request_notification_permission() -> Permission {
    #[cfg(target_os = "macos")]
    return macos::request_permission().await;
    #[cfg(not(target_os = "macos"))]
    Permission::Unavailable
}

/// Opens claude-switch's page of System Settings → Notifications, where a
/// permission once denied can be given back.
#[tauri::command]
pub fn open_notification_settings(app: AppHandle) -> AppResult<()> {
    let url = format!(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={}",
        app.config().identifier
    );
    let status = std::process::Command::new("open").arg(&url).status()?;
    if !status.success() {
        return Err(crate::error::AppError::Validation(format!(
            "`open {url}` exited with status {status}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_kind::AppKind;
    use crate::settings::ProfileLayout;
    use crate::usage::metrics::{Metric, Reading as Amount, Unit};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn meter(id: &str, label: &str, used: f64, limit: f64) -> Metric {
        Metric {
            id: id.into(),
            label: label.into(),
            value: MetricValue::Meter {
                used,
                limit,
                unit: Unit::Percent,
                resets_at: Some("2026-10-17T14:00:00Z".into()),
                period_secs: Some(5 * 60 * 60),
                not_started: false,
            },
            state: None,
        }
    }

    fn entry(metrics: Vec<Metric>) -> TrayEntry {
        TrayEntry {
            id: "p1".into(),
            app: AppKind::Claude,
            name: "Work".into(),
            command: "claude-work".into(),
            plan: None,
            usage: None,
            metrics,
            error: None,
            retry_at: None,
        }
    }

    #[test]
    fn every_meter_is_read_with_whose_it_is_and_what_is_left() {
        let resets = Metric {
            id: "resets".into(),
            label: "Rate limit resets".into(),
            value: MetricValue::Values {
                values: vec![Amount {
                    amount: 1.0,
                    unit: Unit::Count,
                }],
                expires_at: Vec::new(),
                breakdown: None,
            },
            state: None,
        };
        let entries = [entry(vec![
            meter("session", "Session", 92.0, 100.0),
            resets,
            meter("extra", "Extra usage", 5.0, 0.0),
        ])];
        let readings = readings(&entries, &LayoutSettings::default(), now());
        assert_eq!(readings.len(), 2, "values without a limit are not read");
        let session = &readings[0];
        assert_eq!(session.key, "p1.session");
        assert_eq!(session.subtitle, "Work Session");
        assert!((session.remaining - 0.08).abs() < 1e-9);
        assert_eq!(
            session.resets_at,
            Some(
                DateTime::parse_from_rfc3339("2026-10-17T14:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
        assert!(session.state.is_some());
        assert_eq!(readings[1].remaining, 0.0);
    }

    #[test]
    fn a_weekly_limit_used_faster_and_faster_alerts_at_each_step() {
        // A tenth of the week left: the share used is about where it ends.
        let weekly = |used: f64| Metric {
            id: "weekly".into(),
            label: "Weekly".into(),
            value: MetricValue::Meter {
                used,
                limit: 100.0,
                unit: Unit::Percent,
                resets_at: Some((now() + chrono::Duration::hours(7 * 24) / 10).to_rfc3339()),
                period_secs: Some(7 * 24 * 60 * 60),
                not_started: false,
            },
            state: None,
        };
        let toggles = crate::settings::NotificationSettings {
            almost_out: true,
            cutting_it_close: true,
            will_run_out: true,
        };
        let mut evaluator = Evaluator::default();
        let mut check = |used| {
            let readings = readings(
                &[entry(vec![weekly(used)])],
                &LayoutSettings::default(),
                now(),
            );
            let mut fired = Vec::new();
            tauri::async_runtime::block_on(evaluator.evaluate(readings, &toggles, |alert| {
                fired.push(alert.trigger);
                std::future::ready(true)
            }));
            fired
        };
        assert!(check(80.0).is_empty(), "the first reading is a baseline");
        assert_eq!(check(87.0), [rules::Trigger::CuttingItClose]);
        assert!(check(87.0).is_empty());
        assert_eq!(
            check(95.0),
            [rules::Trigger::WillRunOut, rules::Trigger::AlmostOut]
        );
    }

    #[test]
    fn a_metric_turned_off_in_the_layout_is_not_read() {
        let entries = [entry(vec![
            meter("session", "Session", 10.0, 100.0),
            meter("weekly", "Weekly", 10.0, 100.0),
        ])];
        let mut layout = LayoutSettings::default();
        layout.profiles.insert(
            "p1".into(),
            ProfileLayout {
                always_visible: Vec::new(),
                on_demand: vec!["weekly".into()],
            },
        );
        let keys: Vec<_> = readings(&entries, &layout, now())
            .into_iter()
            .map(|reading| reading.key)
            .collect();
        assert_eq!(keys, ["p1.weekly"]);
    }
}
