//! The menu bar item: every account's usage at a glance, kept current for as
//! long as claude-switch runs, with its window open or not.
//!
//! The item shows each Claude and ChatGPT profile that reports usage as its
//! app's mark beside the share used of its two windows ([`strip`]), leaving
//! out those the user hid from the menu bar. A click opens a panel with every
//! profile's meters, hidden or not (the `tray` window, `tray.html`);
//! a right-click offers to open claude-switch or quit it. Closing the main
//! window only hides it, and the Dock icon with it, so the item stays. The
//! item goes at the right end of the menu bar, where a full bar can't hide
//! it, unless the user has moved it.
//!
//! Usage is fetched here, every five minutes and when the panel opens, rather
//! than by the main window, which stops polling while it isn't focused. These
//! fetches never start `claude` to refresh a lapsed sign-in: nobody may be
//! watching. A profile whose fetch fails keeps showing its last good figures.
//!
//! The strip's rules — a mark per account, its two values stacked beside it,
//! nothing drawn for a window without data — follow OpenUsage's menu bar
//! (MIT, <https://github.com/robinebers/openusage>).

pub(crate) mod cache;
mod privacy;
#[cfg(target_os = "macos")]
mod render;
mod strip;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, Rect, WebviewWindow, WindowEvent};

use crate::app_kind::{default_id, AppKind};
use crate::app_state::AppState;
use crate::error::AppResult;
use crate::settings::MenuBarStyle;
use crate::usage::metrics::{self, Metric};
use crate::usage::{backoff, spend};
use crate::usage::{ProfileUsage, QuotaError};
use crate::{app_state, commands, logging, migration, notifications, profiles};

/// Label of the main window.
pub const MAIN_WINDOW: &str = "main";
/// Label of the panel window, declared in tauri.conf.json.
pub const PANEL_WINDOW: &str = "tray";
const TRAY_ID: &str = "usage";
const OPEN_ID: &str = "tray-open";
const SETTINGS_ID: &str = "tray-settings";
const QUIT_ID: &str = "tray-quit";
/// Carries the entries to the panel whenever they change.
const USAGE_EVENT: &str = "tray-usage";
/// Carries the settings to the panel whenever they change.
const SETTINGS_EVENT: &str = "settings-changed";
/// Carries [`TrayStatus`] to the panel whenever it changes.
const STATUS_EVENT: &str = "tray-status";
/// Asks the main window to show its Settings pane.
const OPEN_SETTINGS_EVENT: &str = "open-settings";
/// Asks the main window to show its About dialog (the App menu's own event).
const OPEN_ABOUT_EVENT: &str = "open-about";

const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// After a fetch that failed for a passing reason (no network, an unexpected
/// answer), the next comes this much sooner.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A profile whose fetch takes longer than this is given up on until the next.
pub(crate) const FETCH_DEADLINE: Duration = Duration::from_secs(120);
/// How often the screen is checked for capture while usage hides from it.
const SCREEN_CAPTURE_POLL: Duration = Duration::from_secs(3);
/// How long a refresh waits, after the quota fetches, for the local logs to
/// be read. A slower read shows when it finishes.
const HISTORY_WAIT: Duration = Duration::from_secs(5);
/// Opening the panel fetches again only if the last fetch is older than this.
const REFRESH_ON_OPEN_AFTER: Duration = Duration::from_secs(60);
/// A click on the item this soon after the panel was hidden is the click that
/// hid it (the panel lost focus to the menu bar), not one to open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// Points between the menu bar and the panel, and kept from a screen edge.
const PANEL_MARGIN: f64 = 6.0;
/// The menu bar's height in points, for placing the panel without its item.
const MENU_BAR_HEIGHT: f64 = 24.0;
const PANEL_CORNER_RADIUS: f64 = 12.0;

/// A profile the item reports on, as the panel shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayEntry {
    pub id: String,
    pub app: AppKind,
    pub name: String,
    /// The CLI command the profile signs in with, named in the panel's
    /// messages: `claude`, or a profile's `claude-<slug>` wrapper.
    pub command: String,
    /// The account's plan as its app names it, e.g. `Max 20x`, when known.
    pub plan: Option<String>,
    /// The last usage that came back with a quota, kept through failed
    /// fetches. `None` until one has.
    pub usage: Option<ProfileUsage>,
    /// What that usage and the profile's local history report.
    pub metrics: Vec<Metric>,
    /// Why the latest fetch brought no quota, if it didn't.
    pub error: Option<QuotaError>,
    /// While a rate limit holds the profile back, when it is asked again
    /// ([`crate::usage::backoff`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
}

#[derive(Default)]
pub struct TrayState {
    entries: Mutex<Vec<TrayEntry>>,
    /// Held for the whole of a refresh, so two never run at once.
    refreshing: tokio::sync::Mutex<()>,
    refreshed_at: Mutex<Option<Instant>>,
    panel_hidden_at: Mutex<Option<Instant>>,
    status: Mutex<TrayStatus>,
    /// Usage is hidden because the screen is being captured.
    concealed: AtomicBool,
    /// The shortcut that opens the panel, as registered.
    shortcut: Mutex<Option<String>>,
}

/// Whether a fetch is running and when the next is due, for the panel's footer.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayStatus {
    pub refreshing: bool,
    pub next_refresh_at: Option<String>,
}

/// Puts the item in the menu bar and starts polling. Called once, from setup.
pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    // The last figures shown, drawn at once while the first fetch runs.
    let state = TrayState::default();
    *state.entries.lock().unwrap() = cache::load();
    app.manage(state);

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, OPEN_ID, "Open claude-switch", true, None::<&str>)?,
            &MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT_ID, "Quit claude-switch", true, None::<&str>)?,
        ],
    )?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(fallback_icon())
        .icon_as_template(true)
        .tooltip("claude-switch")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_ID => show_main_window(app),
            SETTINGS_ID => open_settings(app.clone()),
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    #[cfg(target_os = "macos")]
    keep_in_sight(&tray);

    if let Some(panel) = app.get_webview_window(PANEL_WINDOW) {
        #[cfg(target_os = "macos")]
        round_corners(&panel);
        let handle = app.handle().clone();
        panel.on_window_event(move |event| {
            if let WindowEvent::Focused(false) = event {
                hide_panel(&handle);
            }
        });
    }

    redraw(app.handle());
    watch_screen_capture(app.handle().clone());
    if let Ok(state) = app_state::load() {
        logging::set_level(state.settings.advanced.log_level);
        apply_shortcut(app.handle(), state.settings.menu_bar.shortcut.as_deref());
    }
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let passing_failure = refresh(&handle).await;
            let wait = if passing_failure {
                RETRY_AFTER
            } else {
                POLL_INTERVAL
            };
            let next = Utc::now() + chrono::Duration::from_std(wait).unwrap_or_default();
            update_status(&handle, |status| {
                status.next_refresh_at = Some(next.to_rfc3339())
            });
            tokio::time::sleep(wait).await;
        }
    });
    Ok(())
}

/// While "hide while screen sharing" is on, checks every few seconds whether
/// the screen is being captured, and redraws the item when that changes.
fn watch_screen_capture(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let wanted = app_state::load()
                .map(|state| state.settings.privacy.hide_while_screen_sharing)
                .unwrap_or(false);
            let concealed = wanted && privacy::screen_captured();
            let was = app
                .state::<TrayState>()
                .concealed
                .swap(concealed, Ordering::Relaxed);
            if was != concealed {
                redraw(&app);
            }
            tokio::time::sleep(SCREEN_CAPTURE_POLL).await;
        }
    });
}

/// The autosave name macOS files the item's place in the menu bar under.
#[cfg(target_os = "macos")]
const ITEM_NAME: &str = "claude-switch-usage";

/// Puts the item at the right end of the menu bar, next to the system's own,
/// unless the user has ⌘-dragged it elsewhere (macOS remembers that place
/// under the item's autosave name). Left to itself, macOS adds an item at the
/// left end, the first place a full menu bar hides behind the notch.
#[cfg(target_os = "macos")]
fn keep_in_sight(tray: &tauri::tray::TrayIcon) {
    use objc2_foundation::{NSString, NSUserDefaults};

    let _ = tray.with_inner_tray_icon(|inner| {
        let Some(item) = inner.ns_status_item() else {
            return;
        };
        let defaults = NSUserDefaults::standardUserDefaults();
        // The key AppKit reads the place from; smaller is further right.
        let place = NSString::from_str(&format!("NSStatusItem Preferred Position {ITEM_NAME}"));
        if defaults.objectForKey(&place).is_none() {
            defaults.setDouble_forKey(0.0, &place);
        }
        item.setAutosaveName(Some(&NSString::from_str(ITEM_NAME)));
    });
}

/// Brings the main window back, e.g. after it was closed to the menu bar.
pub fn show_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_dock_visibility(true);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The entries the panel shows, as of the last fetch or, before it, as last
/// saved; `None` while there are neither.
#[tauri::command]
pub fn tray_usage(state: tauri::State<'_, TrayState>) -> Option<Vec<TrayEntry>> {
    let fetched = state.refreshed_at.lock().unwrap().is_some();
    let entries = state.entries.lock().unwrap();
    (fetched || !entries.is_empty()).then(|| for_panel(&entries))
}

/// The entries as of the last fetch or, before it, as last saved.
pub(crate) fn entries(app: &AppHandle) -> Vec<TrayEntry> {
    app.state::<TrayState>().entries.lock().unwrap().clone()
}

/// From the panel: hides it and shows the main window.
#[tauri::command]
pub fn open_main_window(app: AppHandle) {
    hide_panel(&app);
    show_main_window(&app);
}

/// From the panel (Esc): hides it.
#[tauri::command]
pub fn close_panel(app: AppHandle) {
    hide_panel(&app);
}

/// From the panel: shows the main window on its Settings pane.
#[tauri::command]
pub fn open_settings(app: AppHandle) {
    open_main_window(app.clone());
    let _ = app.emit_to(MAIN_WINDOW, OPEN_SETTINGS_EVENT, ());
}

/// From the panel: shows the main window with its About dialog.
#[tauri::command]
pub fn open_about(app: AppHandle) {
    open_main_window(app.clone());
    let _ = app.emit_to(MAIN_WINDOW, OPEN_ABOUT_EVENT, ());
}

/// From the panel: fetches every profile's usage now, rather than at the
/// next scheduled fetch.
#[tauri::command]
pub fn refresh_usage(app: AppHandle) {
    refresh_now(&app);
}

/// Fetches every profile's usage now, in the background.
pub fn refresh_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { refresh(&app).await });
}

/// Whether a fetch is running and when the next is due.
#[tauri::command]
pub fn tray_status(state: tauri::State<'_, TrayState>) -> TrayStatus {
    state.status.lock().unwrap().clone()
}

/// Changes the status and tells the panel.
fn update_status(app: &AppHandle, change: impl FnOnce(&mut TrayStatus)) {
    let status = {
        let state = app.state::<TrayState>();
        let mut status = state.status.lock().unwrap();
        change(&mut status);
        status.clone()
    };
    let _ = app.emit_to(PANEL_WINDOW, STATUS_EVENT, status);
}

/// From the panel: ends claude-switch, menu bar item and all.
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn toggle_panel(app: &AppHandle, icon: Rect) {
    let Some(panel) = app.get_webview_window(PANEL_WINDOW) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        hide_panel(app);
        return;
    }
    let state = app.state::<TrayState>();
    let just_hidden = state
        .panel_hidden_at
        .lock()
        .unwrap()
        .is_some_and(|at| at.elapsed() < REOPEN_GUARD);
    if just_hidden {
        return;
    }
    open_panel(app, &panel, icon);
}

/// Opens the panel under the item, as a click on it does: for a click on a
/// notification. With the item out of sight, as when a full menu bar hides
/// it, the main window opens instead.
pub fn show_panel(app: &AppHandle) {
    let Some(panel) = app.get_webview_window(PANEL_WINDOW) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.set_focus();
        return;
    }
    let icon = app
        .tray_by_id(TRAY_ID)
        .and_then(|tray| tray.rect().ok().flatten())
        .filter(|icon| on_screen(&panel, icon));
    match icon {
        Some(icon) => open_panel(app, &panel, icon),
        None => show_main_window(app),
    }
}

fn on_screen(panel: &WebviewWindow, icon: &Rect) -> bool {
    monitor_under(panel, icon).is_some()
}

/// The screen the item is on, if it's on one. macOS looks the point up in
/// points, not the pixels the rest of the placement works in.
fn monitor_under(panel: &WebviewWindow, icon: &Rect) -> Option<tauri::Monitor> {
    let scale = panel.scale_factor().unwrap_or(1.0);
    let point = icon.position.to_logical::<f64>(scale);
    panel.monitor_from_point(point.x, point.y).ok().flatten()
}

fn open_panel(app: &AppHandle, panel: &WebviewWindow, icon: Rect) {
    let state = app.state::<TrayState>();
    place_panel(panel, icon);
    let _ = panel.show();
    let _ = panel.set_focus();
    // Redraws its "updated … ago" lines at once; a fetch may follow.
    let _ = app.emit_to(
        PANEL_WINDOW,
        USAGE_EVENT,
        for_panel(&state.entries.lock().unwrap()),
    );
    let stale = state
        .refreshed_at
        .lock()
        .unwrap()
        .is_none_or(|at| at.elapsed() >= REFRESH_ON_OPEN_AFTER);
    if stale {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move { refresh(&handle).await });
    }
}

fn hide_panel(app: &AppHandle) {
    let Some(panel) = app.get_webview_window(PANEL_WINDOW) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        *app.state::<TrayState>().panel_hidden_at.lock().unwrap() = Some(Instant::now());
    }
}

/// Centres the panel under the item, kept on the item's screen.
fn place_panel(panel: &WebviewWindow, icon: Rect) {
    let Ok(scale) = panel.scale_factor() else {
        return;
    };
    let Ok(size) = panel.outer_size() else {
        return;
    };
    let position = icon.position.to_physical::<f64>(scale);
    let icon_size = icon.size.to_physical::<f64>(scale);
    let width = f64::from(size.width);
    let margin = PANEL_MARGIN * scale;
    let mut x = position.x + icon_size.width / 2.0 - width / 2.0;
    let mut y = position.y + icon_size.height + margin;
    match monitor_under(panel, &icon) {
        Some(monitor) => {
            let left = f64::from(monitor.position().x) + margin;
            let right = f64::from(monitor.position().x) + f64::from(monitor.size().width) - margin;
            x = x.min(right - width).max(left);
        }
        // The item is out of sight (hidden behind the notch, or opened from
        // the shortcut): the top right of the main screen, under the menu bar.
        _ => {
            if let Ok(Some(monitor)) = panel.primary_monitor() {
                x = f64::from(monitor.position().x) + f64::from(monitor.size().width)
                    - width
                    - margin;
                y = f64::from(monitor.position().y) + MENU_BAR_HEIGHT * scale + margin;
            }
        }
    }
    let _ = panel.set_position(PhysicalPosition::new(x, y));
}

/// A borderless window has square corners, and without macOS's private API
/// the webview can't be see-through; clipping the content view to a rounded
/// rectangle over a clear window gives the panel a popover's corners.
#[cfg(target_os = "macos")]
fn round_corners(panel: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSColor, NSWindow};

    let Ok(pointer) = panel.ns_window() else {
        return;
    };
    // SAFETY: Tauri hands out the NSWindow it owns, alive as long as `panel`.
    let window: &NSWindow = unsafe { &*pointer.cast() };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    let Some(view) = window.contentView() else {
        return;
    };
    view.setWantsLayer(true);
    // SAFETY: a view that wants a layer has one, and these are CALayer's own
    // corner radius and clipping properties.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        if let Some(layer) = layer.as_ref() {
            let _: () = msg_send![layer, setCornerRadius: PANEL_CORNER_RADIUS];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
    window.invalidateShadow();
}

/// A profile to report on, before its usage is fetched.
pub(crate) struct Target {
    pub(crate) id: String,
    pub(crate) app: AppKind,
    pub(crate) name: String,
    pub(crate) command: String,
}

/// The profiles the item reports on, in the sidebar's order: by app, each
/// app's stock install before its profiles. Usage is read with the CLI's
/// sign-in, so only those with the CLI set up have any.
pub(crate) fn targets() -> AppResult<Vec<Target>> {
    let profiles = profiles::load()?;
    let custom_names = app_state::load()
        .map(|state| state.default_profile_names)
        .unwrap_or_default();
    let mut targets = Vec::new();
    for app in [AppKind::Claude, AppKind::Codex] {
        let spec = app.spec();
        if !spec.has_usage {
            continue;
        }
        if migration::detect_for(app).is_ok_and(|install| install.cli_path.is_some()) {
            targets.push(Target {
                id: default_id(app),
                app,
                name: custom_names
                    .get(&app)
                    .cloned()
                    .unwrap_or_else(|| spec.display_name.to_owned()),
                command: spec.cli_binary.to_owned(),
            });
        }
        targets.extend(
            profiles
                .iter()
                .filter(|profile| profile.app == app && profile.surfaces.cli)
                .map(|profile| Target {
                    id: profile.id.clone(),
                    app,
                    name: profile.name.clone(),
                    command: format!("{}-{}", spec.cli_wrapper_prefix, profile.slug),
                }),
        );
    }
    Ok(targets)
}

/// Fetches every profile's usage and shows it in the item and the panel.
/// True when a fetch failed for a reason that may pass soon.
async fn refresh(app: &AppHandle) -> bool {
    let state = app.state::<TrayState>();
    let _refreshing = state.refreshing.lock().await;
    update_status(app, |status| status.refreshing = true);
    let passing_failure = fetch_all(app).await;
    update_status(app, |status| status.refreshing = false);
    passing_failure
}

/// The body of [`refresh`], run under its lock.
async fn fetch_all(app: &AppHandle) -> bool {
    let state = app.state::<TrayState>();
    let targets = match targets() {
        Ok(targets) => targets,
        Err(err) => {
            logging::error(
                "menu bar",
                &format!("could not list the profiles: {}", err.message()),
            );
            return true;
        }
    };
    let previous = state.entries.lock().unwrap().clone();
    // Price lists refresh on the side; a read of the logs never waits on
    // them. The read runs while usage is fetched.
    tauri::async_runtime::spawn(spend::pricing::store::refresh());
    let mut history = spend::start_scan();
    let mut entries = Vec::with_capacity(targets.len());
    // One at a time: fetched together, profiles have tripped the usage
    // endpoint's rate limit.
    for target in targets {
        let fetched =
            tokio::time::timeout(FETCH_DEADLINE, commands::profile_usage(&target.id, false)).await;
        let last = previous.iter().find(|entry| entry.id == target.id);
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
        let plan = usage
            .as_ref()
            .and_then(|usage| usage.quota.as_ref()?.plan.clone())
            .or_else(|| commands::profile_plan(&target.id))
            .or_else(|| last.and_then(|entry| entry.plan.clone()));
        let retry_at = retry_at(&target.id, error);
        entries.push(TrayEntry {
            id: target.id,
            app: target.app,
            name: target.name,
            command: target.command,
            plan,
            usage,
            metrics,
            error,
            retry_at,
        });
    }
    let histories = history.within(HISTORY_WAIT).await;
    let now = Local::now();
    for entry in &mut entries {
        let spent = match &histories {
            Some(histories) => spend::metrics_for(histories, &entry.id, &now),
            // Until the read finishes, what the last one found.
            None => previous
                .iter()
                .find(|last| last.id == entry.id)
                .map(|last| spend_metrics(&last.metrics))
                .unwrap_or_default(),
        };
        entry.metrics.extend(spent);
    }
    if histories.is_none() {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(histories) = history.finished().await {
                show_history(&handle, &histories).await;
            }
        });
    }
    after_refresh(app, &previous, &entries);
    if let Err(err) = cache::save(&entries) {
        logging::warn(
            "menu bar",
            &format!("could not save the figures: {}", err.message()),
        );
    }
    for entry in &entries {
        if let Some(error) = entry.error {
            logging::warn(
                "usage",
                &format!(
                    "{} ({}): no usage this time: {error:?}",
                    entry.name, entry.id
                ),
            );
        }
    }
    logging::info("usage", &format!("fetched {} profiles", entries.len()));
    let passing_failure = entries.iter().any(|entry| {
        matches!(
            entry.error,
            Some(QuotaError::Network | QuotaError::Unknown | QuotaError::Forbidden)
        )
    });
    *state.entries.lock().unwrap() = entries.clone();
    *state.refreshed_at.lock().unwrap() = Some(Instant::now());
    show(app, entries);
    passing_failure
}

/// When a profile whose fetch was refused for a rate limit is asked again,
/// while a hold keeps it back.
fn retry_at(profile_id: &str, error: Option<QuotaError>) -> Option<String> {
    if !matches!(error, Some(QuotaError::RateLimited)) {
        return None;
    }
    let until = backoff::until(&backoff::load(), profile_id, Utc::now())?;
    Some(until.to_rfc3339())
}

/// The metrics of `metrics` read from the local logs.
fn spend_metrics(metrics: &[Metric]) -> Vec<Metric> {
    metrics
        .iter()
        .filter(|metric| spend::is_spend_metric(&metric.id))
        .cloned()
        .collect()
}

/// Shows a read of the local logs that finished after its refresh did.
async fn show_history(app: &AppHandle, histories: &spend::Histories) {
    let state = app.state::<TrayState>();
    let _refreshing = state.refreshing.lock().await;
    let now = Local::now();
    let entries: Vec<TrayEntry> = state
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|entry| {
            let mut metrics: Vec<Metric> = entry
                .metrics
                .iter()
                .filter(|metric| !spend::is_spend_metric(&metric.id))
                .cloned()
                .collect();
            metrics.extend(spend::metrics_for(histories, &entry.id, &now));
            TrayEntry {
                metrics,
                ..entry.clone()
            }
        })
        .collect();
    if let Err(err) = cache::save(&entries) {
        logging::warn(
            "menu bar",
            &format!("could not save the figures: {}", err.message()),
        );
    }
    *state.entries.lock().unwrap() = entries.clone();
    show(app, entries);
}

/// Runs after every fetch with the entries before and after it: where what
/// reacts to a change in usage (notifications) hooks in.
fn after_refresh(app: &AppHandle, _previous: &[TrayEntry], next: &[TrayEntry]) {
    notifications::after_refresh(app, next);
}

/// `entries` as sent to the panel: each meter with its state as of now.
fn for_panel(entries: &[TrayEntry]) -> Vec<TrayEntry> {
    let now = Utc::now();
    entries
        .iter()
        .map(|entry| TrayEntry {
            metrics: metrics::with_states(&entry.metrics, now),
            ..entry.clone()
        })
        .collect()
}

/// The user changed what the item or the panel shows: redraws the one and
/// tells the other, fetching nothing. Starts or stops the local API too.
pub fn settings_changed(app: &AppHandle, state: &AppState) {
    let _ = app.emit_to(PANEL_WINDOW, SETTINGS_EVENT, &state.settings);
    logging::set_level(state.settings.advanced.log_level);
    apply_shortcut(app, state.settings.menu_bar.shortcut.as_deref());
    redraw(app);
    crate::local_api::apply(app, state.settings.integrations.local_api);
}

/// Registers `shortcut` (e.g. `CommandOrControl+Shift+U`) to open the panel
/// from anywhere, in place of the one registered before. One macOS or
/// another app already holds is reported and left unregistered.
fn apply_shortcut(app: &AppHandle, shortcut: Option<&str>) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let state = app.state::<TrayState>();
    let mut registered = state.shortcut.lock().unwrap();
    if registered.as_deref() == shortcut {
        return;
    }
    if let Some(old) = registered.take() {
        let _ = app.global_shortcut().unregister(old.as_str());
    }
    if let Some(new) = shortcut.filter(|new| !new.is_empty()) {
        match app.global_shortcut().register(new) {
            Ok(()) => *registered = Some(new.to_owned()),
            Err(err) => logging::warn("shortcut", &format!("could not register {new}: {err}")),
        }
    }
}

/// Opens or closes the panel from the global shortcut: under the item, or at
/// the top right of the main screen when the item is out of sight.
pub fn toggle_panel_from_shortcut(app: &AppHandle) {
    let rect = app
        .tray_by_id(TRAY_ID)
        .and_then(|tray| tray.rect().ok().flatten());
    match rect {
        Some(rect) => toggle_panel(app, rect),
        None => toggle_panel(
            app,
            Rect {
                position: tauri::Position::Physical(PhysicalPosition::new(-1, -1)),
                size: tauri::Size::Physical(tauri::PhysicalSize::new(0, 0)),
            },
        ),
    }
}

/// Redraws the item from the figures it has, fetching nothing. With none yet
/// there is nothing to draw but the mark already there.
fn redraw(app: &AppHandle) {
    let entries = app.state::<TrayState>().entries.lock().unwrap().clone();
    if !entries.is_empty() {
        draw(app, &entries);
    }
}

fn show(app: &AppHandle, entries: Vec<TrayEntry>) {
    let _ = app.emit_to(PANEL_WINDOW, USAGE_EVENT, for_panel(&entries));
    draw(app, &entries);
}

/// Draws the item for `entries`, as the user's settings say: less the
/// profiles hidden from the menu bar, with the metrics starred for the rest.
fn draw(app: &AppHandle, entries: &[TrayEntry]) {
    // Unreadable settings draw as the defaults did before there were any.
    let state = app_state::load().unwrap_or_default();
    let entries = strip::in_menu_bar(entries, &state.hidden_from_menu_bar);
    let options = strip::Options {
        menu_bar: &state.settings.menu_bar,
        meter_style: state.settings.display.meter_style,
    };
    let now = Utc::now();
    let look = if app.state::<TrayState>().concealed.load(Ordering::Relaxed) {
        Look::Concealed
    } else {
        match state.settings.menu_bar.style {
            MenuBarStyle::Text => Look::Text(strip::build(&entries, &options, now)),
            MenuBarStyle::Bars => Look::Bars(strip::bars(&entries, &options, now)),
        }
    };
    let tooltip = strip::tooltip(&entries, &options, now);
    let handle = app.clone();
    // AppKit draws on the main thread.
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        let _ = tray.set_icon(Some(icon(&look)));
        let _ = tray.set_icon_as_template(true);
        let _ = tray.set_tooltip(Some(tooltip));
    });
}

/// What the item shows.
enum Look {
    Text(strip::Strip),
    Bars(Vec<f64>),
    /// The app's mark and name instead of usage, while the screen is captured.
    Concealed,
}

/// `look` drawn, or claude-switch's mark when there is nothing to show.
#[cfg(target_os = "macos")]
fn icon(look: &Look) -> Image<'static> {
    let drawn = match look {
        Look::Text(strip) => render::strip(strip),
        Look::Bars(fractions) => render::bars(fractions),
        Look::Concealed => render::concealed(),
    };
    drawn
        .map(|drawn| Image::new_owned(drawn.rgba, drawn.width, drawn.height))
        .unwrap_or_else(fallback_icon)
}

#[cfg(not(target_os = "macos"))]
fn icon(_look: &Look) -> Image<'static> {
    fallback_icon()
}

/// claude-switch's own mark, for when no profile has usage to show.
#[cfg(target_os = "macos")]
fn fallback_icon() -> Image<'static> {
    let mark = render::app_mark();
    Image::new_owned(mark.rgba, mark.width, mark.height)
}

/// The menu bar strip is drawn with AppKit; elsewhere the item is blank.
#[cfg(not(target_os = "macos"))]
fn fallback_icon() -> Image<'static> {
    Image::new_owned(vec![0; 4], 1, 1)
}
