// SPDX-License-Identifier: MIT

mod accounts;
mod app_kind;
mod app_state;
pub mod cli;
mod codex_rpc;
mod commands;
mod deps;
mod error;
mod haptics;
mod inherited_env;
mod launch;
mod launchers;
mod local_api;
mod logging;
mod login_item;
mod migration;
mod notifications;
mod path_setup;
mod paths;
mod profiles;
mod sessions;
mod settings;
mod shared_config;
mod slug;
#[cfg(test)]
mod test_support;
mod tray;
mod usage;
mod usage_command;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, WindowEvent};

/// Custom menu identifier for the App menu's "About claude-switch" entry.
/// The default macOS About item opens a tiny system panel; we replace it
/// with an item that emits an event the frontend listens for, so the
/// click opens our own Atelier-styled `<AboutDialog>` instead.
const OPEN_ABOUT_ID: &str = "open-about";

/// Set (to anything) to leave the launchers and the clone watch as they are
/// at startup: `CLAUDE_SWITCH_SKIP_STARTUP_SYNC=1 pnpm tauri dev`.
const SKIP_STARTUP_SYNC_ENV: &str = "CLAUDE_SWITCH_SKIP_STARTUP_SYNC";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // One claude-switch at a time: opening it again (from the Finder, say,
    // while it sits in the menu bar) brings back the running one's window.
    // Not in a development build, which should run beside the installed app.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        tray::show_main_window(app);
    }));
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Provides `relaunch()` to the frontend so the updater can restart
        // the app itself after `downloadAndInstall` finishes — Tauri 2's
        // updater plugin does NOT relaunch on its own.
        .plugin(tauri_plugin_process::init())
        // The menu bar panel is placed under the item each time it opens.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_denylist(&[tray::PANEL_WINDOW])
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        // The shortcut that opens the menu bar panel from anywhere (Settings).
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        tray::toggle_panel_from_shortcut(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            // Bring the desktop launchers an earlier version built up to this
            // one, so every profile changes over now rather than whenever it
            // next happens to be rebuilt. Off the main thread: a wrapper takes
            // seconds to build. A development run sets SKIP_STARTUP_SYNC_ENV:
            // otherwise it would point the user's clone watch at itself.
            let skip_sync = std::env::var_os(SKIP_STARTUP_SYNC_ENV).is_some();
            std::thread::spawn(move || {
                if skip_sync {
                    return;
                }
                let Ok(all) = profiles::load() else {
                    return;
                };
                let version = env!("CARGO_PKG_VERSION");
                for (id, result) in launchers::gui::refresh_outdated(&all, version) {
                    match result {
                        Ok(()) => eprintln!("claude-switch: rebuilt the launcher of profile {id}"),
                        Err(err) => {
                            eprintln!("claude-switch: left the launcher of profile {id}: {err}")
                        }
                    }
                }
                // The clone watch runs this executable, which may have moved
                // since it was installed; so does the `claude-switch` command.
                launchers::clone_watch::sync_best_effort();
                launchers::command::sync_best_effort();
            });

            // Build the macOS app menu manually so we can swap the default
            // About panel for a frontend-driven dialog. Everything else here
            // mirrors what Tauri would auto-generate (Services, Hide,
            // Hide Others, Show All, Quit) so the menu stays familiar.
            let about = MenuItem::with_id(
                app,
                OPEN_ABOUT_ID,
                "About claude-switch",
                true,
                None::<&str>,
            )?;
            let app_submenu = Submenu::with_items(
                app,
                "claude-switch",
                true,
                &[
                    &about,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::show_all(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?;
            // Edit submenu so Cmd+C / Cmd+V / Cmd+Z keep working in dialog
            // inputs — these would otherwise be silently dropped because
            // setting a custom menu replaces the default one entirely.
            let edit_submenu = Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?;
            let menu = Menu::with_items(app, &[&app_submenu, &edit_submenu])?;
            app.set_menu(menu)?;

            tray::setup(app)?;
            notifications::setup(app);
            let local_api =
                app_state::load().is_ok_and(|state| state.settings.integrations.local_api);
            local_api::apply(app.handle(), local_api);
            Ok(())
        })
        // Closing the main window leaves claude-switch running in the menu
        // bar; the item, the Dock icon or Quit bring it back or end it.
        .on_window_event(|window, event| {
            if window.label() == tray::MAIN_WINDOW {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == OPEN_ABOUT_ID {
                let _ = app.emit(OPEN_ABOUT_ID, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_profiles,
            commands::create_profile,
            commands::regenerate_launchers,
            commands::update_profile,
            commands::reorder_profiles,
            commands::delete_profile,
            commands::toggle_surface,
            commands::open_profile_in_app,
            commands::touch_profile_last_used,
            commands::open_in_finder,
            commands::open_default_gui,
            commands::profile_paths,
            commands::profile_account,
            commands::detect_existing_install,
            commands::detect_existing_sizes,
            commands::import_existing_install,
            commands::list_migration_backups,
            commands::delete_migration_backup,
            commands::check_dependencies,
            commands::detect_shell,
            commands::install_path_hook,
            commands::load_app_state,
            commands::update_app_state,
            commands::get_app_metadata,
            commands::get_profile_usage,
            commands::use_codex_reset,
            commands::open_external_url,
            commands::open_cli_login,
            commands::list_sessions,
            commands::check_session_action,
            commands::archive_session,
            commands::restore_session,
            commands::plan_session_move,
            commands::move_session,
            commands::check_session_repair,
            commands::repair_sessions,
            tray::tray_usage,
            tray::tray_status,
            tray::refresh_usage,
            tray::open_main_window,
            tray::close_panel,
            tray::open_settings,
            tray::open_about,
            tray::quit_app,
            login_item::login_item,
            login_item::set_login_item,
            logging::log_file,
            haptics::haptic_tick,
            notifications::notification_permission,
            notifications::request_notification_permission,
            notifications::open_notification_settings,
            launchers::command::command_line,
            launchers::command::install_command_line,
            launchers::command::uninstall_command_line,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // A click on the Dock icon brings back a main window that was
            // closed to the menu bar.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show_main_window(app);
                return;
            }
            // claude:// links reach claude-switch only while a profile signs in
            // (see `launchers::link_handler`); they belong to Claude.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = event {
                let scheme = launchers::link_handler::SCHEME;
                for url in urls.iter().filter(|url| url.scheme() == scheme) {
                    if let Err(err) = launchers::link_handler::forward(url.as_str()) {
                        eprintln!(
                            "claude-switch: could not pass a {scheme}:// link on to Claude: {}",
                            err.message()
                        );
                    }
                }
            }
        });
}
