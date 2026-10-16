//! The menu bar item: every account's usage at a glance, kept current for as
//! long as claude-switch runs, with its window open or not.
//!
//! The item shows each Claude and ChatGPT profile that reports usage as its
//! app's mark beside the share used of its two windows ([`strip`]). A click
//! opens a panel with each profile's meters (the `tray` window, `tray.html`);
//! a right-click offers to open claude-switch or quit it. Closing the main
//! window only hides it, so the item stays.
//!
//! Usage is fetched here, every five minutes and when the panel opens, rather
//! than by the main window, which stops polling while it isn't focused. These
//! fetches never start `claude` to refresh a lapsed sign-in: nobody may be
//! watching. A profile whose fetch fails keeps showing its last good figures.
//!
//! The strip's rules — a mark per account, its two values stacked beside it,
//! nothing drawn for a window without data — follow OpenUsage's menu bar
//! (MIT, <https://github.com/robinebers/openusage>).

#[cfg(target_os = "macos")]
mod render;
mod strip;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, Rect, WebviewWindow, WindowEvent};

use crate::app_kind::{default_id, AppKind};
use crate::error::AppResult;
use crate::usage::{ProfileUsage, QuotaError};
use crate::{app_state, commands, migration, profiles};

/// Label of the main window.
pub const MAIN_WINDOW: &str = "main";
/// Label of the panel window, declared in tauri.conf.json.
pub const PANEL_WINDOW: &str = "tray";
const TRAY_ID: &str = "usage";
const OPEN_ID: &str = "tray-open";
const QUIT_ID: &str = "tray-quit";
/// Carries the entries to the panel whenever they change.
const USAGE_EVENT: &str = "tray-usage";

const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Opening the panel fetches again only if the last fetch is older than this.
const REFRESH_ON_OPEN_AFTER: Duration = Duration::from_secs(60);
/// A click on the item this soon after the panel was hidden is the click that
/// hid it (the panel lost focus to the menu bar), not one to open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// Points between the menu bar and the panel, and kept from a screen edge.
const PANEL_MARGIN: f64 = 6.0;
const PANEL_CORNER_RADIUS: f64 = 12.0;

/// A profile the item reports on, as the panel shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayEntry {
    pub id: String,
    pub app: AppKind,
    pub name: String,
    /// The CLI command the profile signs in with, named in the panel's
    /// messages: `claude`, or a profile's `claude-<slug>` wrapper.
    pub command: String,
    /// The last usage that came back with a quota, kept through failed
    /// fetches. `None` until one has.
    pub usage: Option<ProfileUsage>,
    /// Why the latest fetch brought no quota, if it didn't.
    pub error: Option<QuotaError>,
}

#[derive(Default)]
pub struct TrayState {
    entries: Mutex<Vec<TrayEntry>>,
    /// Held for the whole of a refresh, so two never run at once.
    refreshing: tokio::sync::Mutex<()>,
    refreshed_at: Mutex<Option<Instant>>,
    panel_hidden_at: Mutex<Option<Instant>>,
}

/// Puts the item in the menu bar and starts polling. Called once, from setup.
pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    app.manage(TrayState::default());

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, OPEN_ID, "Open claude-switch", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT_ID, "Quit claude-switch", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(fallback_icon())
        .icon_as_template(true)
        .tooltip("claude-switch")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_ID => show_main_window(app),
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

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            refresh(&handle).await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
    Ok(())
}

/// Brings the main window back, e.g. after it was closed to the menu bar.
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The entries the panel shows, as of the last fetch; `None` until the
/// first has finished.
#[tauri::command]
pub fn tray_usage(state: tauri::State<'_, TrayState>) -> Option<Vec<TrayEntry>> {
    let fetched = state.refreshed_at.lock().unwrap().is_some();
    fetched.then(|| state.entries.lock().unwrap().clone())
}

/// From the panel: hides it and shows the main window.
#[tauri::command]
pub fn open_main_window(app: AppHandle) {
    hide_panel(&app);
    show_main_window(&app);
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
    place_panel(&panel, icon);
    let _ = panel.show();
    let _ = panel.set_focus();
    // Redraws its "updated … ago" lines at once; a fetch may follow.
    let _ = app.emit_to(PANEL_WINDOW, USAGE_EVENT, &*state.entries.lock().unwrap());
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
    if let Ok(Some(monitor)) = panel.monitor_from_point(position.x, position.y) {
        let left = f64::from(monitor.position().x) + margin;
        let right = f64::from(monitor.position().x) + f64::from(monitor.size().width) - margin;
        x = x.min(right - width).max(left);
    }
    let y = position.y + icon_size.height + margin;
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
struct Target {
    id: String,
    app: AppKind,
    name: String,
    command: String,
}

/// The profiles the item reports on, in the sidebar's order: by app, each
/// app's stock install before its profiles. Usage is read with the CLI's
/// sign-in, so only those with the CLI set up have any.
fn targets() -> AppResult<Vec<Target>> {
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
async fn refresh(app: &AppHandle) {
    let state = app.state::<TrayState>();
    let _refreshing = state.refreshing.lock().await;
    let targets = match targets() {
        Ok(targets) => targets,
        Err(err) => {
            eprintln!(
                "claude-switch: could not list the profiles for the menu bar: {}",
                err.message()
            );
            return;
        }
    };
    let previous = state.entries.lock().unwrap().clone();
    let mut entries = Vec::with_capacity(targets.len());
    // One at a time: fetched together, profiles have tripped the usage
    // endpoint's rate limit.
    for target in targets {
        let fetched = commands::profile_usage(&target.id, false).await;
        let last = previous
            .iter()
            .find(|entry| entry.id == target.id)
            .and_then(|entry| entry.usage.clone());
        let (usage, error) = match fetched {
            Ok(usage) if usage.quota.is_some() => (Some(usage), None),
            Ok(usage) => (last, Some(usage.quota_error.unwrap_or(QuotaError::Unknown))),
            Err(_) => (last, Some(QuotaError::Unknown)),
        };
        entries.push(TrayEntry {
            id: target.id,
            app: target.app,
            name: target.name,
            command: target.command,
            usage,
            error,
        });
    }
    *state.entries.lock().unwrap() = entries.clone();
    *state.refreshed_at.lock().unwrap() = Some(Instant::now());
    show(app, entries);
}

fn show(app: &AppHandle, entries: Vec<TrayEntry>) {
    let _ = app.emit_to(PANEL_WINDOW, USAGE_EVENT, &entries);
    let now = Utc::now();
    let strip = strip::build(&entries, now);
    let tooltip = strip::tooltip(&entries, now);
    let handle = app.clone();
    // AppKit draws on the main thread.
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        let _ = tray.set_icon(Some(strip_icon(&strip)));
        let _ = tray.set_icon_as_template(true);
        let _ = tray.set_tooltip(Some(tooltip));
    });
}

/// The strip drawn, or claude-switch's mark when it has nothing to show.
#[cfg(target_os = "macos")]
fn strip_icon(strip: &strip::Strip) -> Image<'static> {
    render::strip(strip)
        .map(|drawn| Image::new_owned(drawn.rgba, drawn.width, drawn.height))
        .unwrap_or_else(fallback_icon)
}

#[cfg(not(target_os = "macos"))]
fn strip_icon(_strip: &strip::Strip) -> Image<'static> {
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
