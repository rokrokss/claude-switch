//! The clone watch: a LaunchAgent that puts strays right.
//!
//! A profile's signed clone loses the profile's arguments and environment
//! whenever macOS, rather than claude-switch, starts it: when the vendor's
//! updater relaunches it after installing into it, when its Dock tile or a
//! Recents entry is clicked, when windows are reopened at login. It then runs
//! on the user's default account under the profile's icon (a
//! [`Stray`](crate::launchers::clone::Stray)). Nothing in the clone can be
//! changed to prevent that without breaking its signature, so it is put right
//! from outside.
//!
//! The agent runs `claude-switch --watch-clones`, which looks every
//! [`POLL`] for:
//!
//! - **Fresh strays** (younger than [`clone::FRESH_STRAY`]): quit, the way the
//!   Quit menu item does, and the profile opened again from its clone.
//! - **Older strays**: never quit unasked — the user may be typing into one. A
//!   notification says what happened, once per process; opening the profile
//!   from claude-switch offers to quit it.
//! - **Updates that left a profile closed**: when the vendor's updater has
//!   installed into a clone and nothing runs from it a while later, a
//!   notification says so. With the default instance already running, a
//!   relaunched clone hands itself over to that one and quits before any look
//!   could catch it, so the profile just disappears. Whether it was meant to
//!   come back can't be told from outside (an update also installs, silently,
//!   when the user quits), so it is reported, not reopened.
//! - **A profile signing in**: while one runs signed out, claude-switch opens
//!   `claude://` links, so that its Google sign-in comes back to it rather than
//!   to the stock app (see [`link_handler`]).
//!
//! The agent is installed while some profile has a signed clone and removed when
//! none has (see [`sync`]). It can also be switched off in System Settings →
//! General → Login Items & Extensions, where macOS lists it.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use plist::{Dictionary, Value};

use crate::app_kind::CLAUDE;
use crate::error::{AppError, AppResult};
use crate::launch;
use crate::launchers::{clone, link_handler, wrapper};
use crate::paths::{app_data_dir, clone_bundle_path, resolve_gui_app};
use crate::profiles::{self, Profile};

/// The agent's launchd label.
pub const LABEL: &str = "app.claude-switch.clone-watch";

/// The flag that runs the watch instead of the GUI.
pub const WATCH_CLONES_FLAG: &str = "--watch-clones";

/// How often the watch looks. A stray is caught well inside
/// [`clone::FRESH_STRAY`].
const POLL: Duration = Duration::from_secs(2);

/// How long after a clone's version changed the watch waits before concluding
/// the profile did not come back: long enough for the updater to finish and for
/// a relaunched stray to be put right.
const UPDATE_SETTLE: Duration = Duration::from_secs(30);

const LAUNCHCTL: &str = "/bin/launchctl";

/// Pure: whether the watch is wanted: some profile opens its desktop app from a
/// signed clone.
pub fn wanted(profiles: &[Profile]) -> bool {
    profiles.iter().any(watched)
}

fn watched(profile: &Profile) -> bool {
    profile.signed_clone && profile.surfaces.gui
}

/// Pure: the agent's plist, running `host` (the claude-switch executable) with
/// [`WATCH_CLONES_FLAG`] for as long as the user is logged in, and logging to
/// `log`.
pub fn plist_body(host: &Path, log: &Path) -> AppResult<Vec<u8>> {
    let mut dict = Dictionary::new();
    dict.insert("Label".into(), Value::String(LABEL.into()));
    dict.insert(
        "ProgramArguments".into(),
        Value::Array(vec![
            Value::String(host.display().to_string()),
            Value::String(WATCH_CLONES_FLAG.into()),
        ]),
    );
    // At login too: windows reopened then start clones without their profile.
    dict.insert("RunAtLoad".into(), Value::Boolean(true));
    dict.insert("KeepAlive".into(), Value::Boolean(true));
    // Nothing reads it. It makes the body of each version differ, so that
    // [`sync`] restarts a watch an earlier version started: replacing the app
    // leaves the old executable running until it exits.
    let mut environment = Dictionary::new();
    environment.insert(
        "CLAUDE_SWITCH_VERSION".into(),
        Value::String(env!("CARGO_PKG_VERSION").into()),
    );
    dict.insert(
        "EnvironmentVariables".into(),
        Value::Dictionary(environment),
    );
    dict.insert(
        "StandardOutPath".into(),
        Value::String(log.display().to_string()),
    );
    dict.insert(
        "StandardErrorPath".into(),
        Value::String(log.display().to_string()),
    );
    let mut bytes = Vec::new();
    ::plist::to_writer_xml(&mut bytes, &Value::Dictionary(dict))
        .map_err(|err| AppError::Validation(format!("plist serialisation failed: {err}")))?;
    Ok(bytes)
}

/// `~/Library/LaunchAgents/<LABEL>.plist`.
pub fn plist_path() -> AppResult<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::NotFound("could not determine user home directory".into()))?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// Where the watch writes what it did.
pub fn log_path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("clone-watch.log"))
}

/// Install, refresh or remove the agent so that it runs exactly while
/// `profiles` has a signed clone to watch, from the claude-switch executable
/// running now. Leaves an agent that is already as it should be alone.
pub fn sync(profiles: &[Profile]) -> AppResult<()> {
    let plist = plist_path()?;
    let domain = format!("gui/{}", user_id()?);
    let service = format!("{domain}/{LABEL}");
    let loaded = launchctl(&["print", &service]).is_ok();

    if !wanted(profiles) {
        if loaded {
            launchctl(&["bootout", &service])?;
        }
        if plist.exists() {
            fs::remove_file(&plist)?;
        }
        return Ok(());
    }

    let host = std::env::current_exe()?;
    let body = plist_body(&host, &log_path()?)?;
    if loaded && fs::read(&plist).ok().as_deref() == Some(body.as_slice()) {
        return Ok(());
    }
    if loaded {
        let _ = launchctl(&["bootout", &service]);
    }
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&plist, &body)?;
    launchctl(&["bootstrap", &domain, &plist.display().to_string()])
}

/// [`sync`] against the saved profiles, reporting rather than failing: called
/// after every change to the profiles and when the app starts, none of which
/// should fail over the watch.
pub fn sync_best_effort() {
    if let Err(err) = profiles::load().and_then(|all| sync(&all)) {
        eprintln!(
            "claude-switch: could not update the clone watch: {}",
            err.message()
        );
    }
}

fn user_id() -> AppResult<String> {
    let output = Command::new("/usr/bin/id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if uid.is_empty() {
        return Err(AppError::Validation("could not read the user id".into()));
    }
    Ok(uid)
}

fn launchctl(args: &[&str]) -> AppResult<()> {
    let output = Command::new(LAUNCHCTL).args(args).output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "launchctl {} failed ({}): {}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// Quit every fresh stray of a profile with a signed clone and open that
/// profile again from its clone. Strays of profiles that have gone, or no longer
/// have a clone, are left alone: they are not ours to put anywhere. Returns the
/// strays left running.
pub fn rehome_fresh_strays(version: &str) -> AppResult<Vec<clone::Stray>> {
    let Some(vendor) = resolve_gui_app(&CLAUDE) else {
        return Ok(Vec::new());
    };
    let exec = clone::vendor_executable(&vendor);
    let strays = clone::running_strays(&exec)?;
    if strays.is_empty() {
        return Ok(strays);
    }
    let profiles = profiles::load()?;
    let mut left = Vec::new();
    for stray in strays {
        let profile = profiles
            .iter()
            .find(|profile| profile.id == stray.profile_id && watched(profile));
        match profile {
            Some(profile) if stray.is_fresh() => {
                eprintln!(
                    "claude-switch: {} was started without its profile (pid {}); reopening it",
                    profile.name, stray.pid
                );
                clone::quit(stray.pid)?;
                launch::open_profile(profile, version, launch::focus_pid)?;
            }
            _ => left.push(stray),
        }
    }
    Ok(left)
}

/// Run the watch until the process is stopped. What one look finds wrong is
/// logged and the next look goes on.
pub fn run() -> ! {
    let version = env!("CARGO_PKG_VERSION");
    let mut watch = Watch::default();
    loop {
        if let Err(err) = watch.look(version) {
            eprintln!("claude-switch: clone watch: {}", err.message());
        }
        thread::sleep(POLL);
    }
}

/// What the watch remembers between looks.
#[derive(Default)]
struct Watch {
    /// Older strays already reported, by pid.
    reported: HashSet<i32>,
    /// Each watched clone's version as last seen, by profile id.
    versions: HashMap<String, Option<String>>,
    /// When each clone whose version changed while watched did so.
    updated: HashMap<String, Instant>,
}

impl Watch {
    fn look(&mut self, version: &str) -> AppResult<()> {
        let profiles: Vec<Profile> = profiles::load()?.into_iter().filter(watched).collect();
        // Before the early return, so that the scheme goes back to Claude even
        // when the last such profile has just gone.
        if let Err(err) = link_handler::sync(&profiles) {
            eprintln!("claude-switch: clone watch: {}", err.message());
        }
        if profiles.is_empty() {
            return Ok(());
        }

        let left = rehome_fresh_strays(version)?;
        self.reported
            .retain(|pid| left.iter().any(|stray| stray.pid == *pid));
        for stray in left {
            let Some(profile) = profiles.iter().find(|p| p.id == stray.profile_id) else {
                continue;
            };
            if self.reported.insert(stray.pid) {
                notify(
                    &format!("Claude ({})", profile.name),
                    &format!(
                        "Running without its profile, on your default Claude account. Open {} \
                         from claude-switch to quit it and reopen it as the profile.",
                        profile.name
                    ),
                );
            }
        }

        self.look_for_updates(&profiles)
    }

    fn look_for_updates(&mut self, profiles: &[Profile]) -> AppResult<()> {
        let processes = launch::process_list()?;
        for profile in profiles {
            let bundle = clone_bundle_path(&profile.id, &profile.name, profile.app.spec())?;
            let now = wrapper::bundle_version(&bundle);
            let previous = self.versions.insert(profile.id.clone(), now.clone());
            if version_changed(previous.as_ref(), now.as_ref()) {
                self.updated.insert(profile.id.clone(), Instant::now());
            }
            let Some(since) = self.updated.get(&profile.id).copied() else {
                continue;
            };
            if since.elapsed() < UPDATE_SETTLE {
                continue;
            }
            self.updated.remove(&profile.id);
            // A version we made the clone at is one of our own refreshes.
            let ours = clone::read_record(&profile.id).cloned_from_version == now;
            if !ours && !clone::in_use(&processes, &bundle) {
                notify(
                    &format!("Claude ({})", profile.name),
                    "Claude updated itself in this profile. If it didn't reopen, open it again \
                     from claude-switch or Spotlight.",
                );
            }
        }
        Ok(())
    }
}

/// Pure: whether a clone seen at `previous` and now at `now` changed version
/// while watched. The first look, and a clone that isn't there, are no change.
fn version_changed(previous: Option<&Option<String>>, now: Option<&String>) -> bool {
    matches!((previous, now), (Some(Some(before)), Some(after)) if before != after)
}

/// Post a user notification. Best effort.
fn notify(title: &str, message: &str) {
    let _ = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title (item 2 of argv)",
            "-e",
            "end run",
            message,
            title,
        ])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_kind::AppKind;
    use crate::profiles::Surfaces;

    fn profile(signed_clone: bool, gui: bool) -> Profile {
        Profile {
            id: "a".into(),
            app: AppKind::Claude,
            name: "Work".into(),
            slug: "work".into(),
            color: "#7C3AED".into(),
            created_at: "2026-10-14T00:00:00Z".into(),
            surfaces: Surfaces { gui, cli: false },
            distinct_dock_icon: false,
            signed_clone,
            last_used_at: None,
        }
    }

    #[test]
    fn the_watch_runs_only_while_a_desktop_profile_has_a_clone() {
        assert!(!wanted(&[]));
        assert!(!wanted(&[profile(false, true)]));
        assert!(!wanted(&[profile(true, false)]), "no desktop app to watch");
        assert!(wanted(&[profile(false, true), profile(true, true)]));
    }

    #[test]
    fn the_agent_runs_the_watch_from_this_executable_at_login_and_keeps_it_up() {
        let body = plist_body(
            Path::new("/Applications/claude-switch.app/Contents/MacOS/claude-switch"),
            Path::new("/tmp/clone-watch.log"),
        )
        .unwrap();
        let dict = Value::from_reader_xml(body.as_slice())
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(dict.get("Label").and_then(Value::as_string), Some(LABEL));
        let args: Vec<&str> = dict
            .get("ProgramArguments")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_string)
            .collect();
        assert_eq!(
            args,
            vec![
                "/Applications/claude-switch.app/Contents/MacOS/claude-switch",
                WATCH_CLONES_FLAG
            ]
        );
        assert_eq!(
            dict.get("RunAtLoad").and_then(Value::as_boolean),
            Some(true)
        );
        assert_eq!(
            dict.get("KeepAlive").and_then(Value::as_boolean),
            Some(true)
        );
        assert_eq!(
            dict.get("EnvironmentVariables")
                .and_then(Value::as_dictionary)
                .and_then(|environment| environment.get("CLAUDE_SWITCH_VERSION"))
                .and_then(Value::as_string),
            Some(env!("CARGO_PKG_VERSION")),
            "a new version restarts the watch"
        );
    }

    #[test]
    fn only_a_change_between_two_seen_versions_counts() {
        let seen = |version: &str| Some(version.to_owned());
        let v1 = "2.1".to_owned();
        let v2 = "2.2".to_owned();
        assert!(!version_changed(None, Some(&v1)), "first look");
        assert!(!version_changed(Some(&None), Some(&v1)), "clone just made");
        assert!(!version_changed(Some(&seen("2.1")), Some(&v1)));
        assert!(!version_changed(Some(&seen("2.1")), None), "clone gone");
        assert!(version_changed(Some(&seen("2.1")), Some(&v2)));
    }
}
