// SPDX-License-Identifier: MIT

//! The command line, which exists for one caller: a wrapper bundle that found
//! itself out of step with the vendor app it was cloned from.
//!
//! A wrapper cannot rebuild itself. The rebuild deletes and recreates the
//! bundle, and the only process in a position to notice the drift is the shim
//! running out of that very bundle. So it hands the launch back here instead:
//! `claude-switch --open-profile <id>` opens one profile through the usual route
//! — which rebuilds a stale wrapper on the way — and exits, without ever
//! starting Tauri or showing a window.
//!
//! The launcher of a profile with a signed clone hands every launch here the
//! same way, and the clone watch LaunchAgent runs `claude-switch --watch-clones`
//! (see [`crate::launchers::clone_watch`]).
//!
//! It also has commands for people: `claude-switch usage` prints the profiles'
//! usage limits ([`crate::usage_command`]), through the `claude-switch` script
//! Settings installs ([`crate::launchers::command`]).

use crate::error::AppError;
use crate::launch;
use crate::launchers::clone_watch;
use crate::profiles;
use crate::usage_command;

pub use clone_watch::WATCH_CLONES_FLAG;

/// Flag that opens one profile and exits, instead of starting the GUI. Defined
/// with the shim's keys, because the shim is what passes it.
pub use profile_shim::OPEN_PROFILE_FLAG;

/// The command that prints the profiles' usage limits.
pub const USAGE_COMMAND: &str = "usage";

/// What `claude-switch help` prints, and the installed command run bare.
pub const HELP: &str = "\
Usage: claude-switch usage [profile] [--force]

Prints each profile's usage limits as JSON, then exits. Figures fetched in the
last five minutes, by the claude-switch app or an earlier run, are reused; the
rest are fetched now. A lapsed sign-in is never renewed here: open
claude-switch, or run the profile's own command, to renew it.

Arguments:
  profile        A profile's id, slug or name, or `claude` or `codex` for
                 every profile of that app. Every profile when left out.

Options:
  --force        Fetch every profile asked for, however recent its figures
  -h, --help     Show this help
  -v, --version  Show the version

Exit status: 0 on success; 2 for a misuse or an unknown profile; 4 when a
fetch failed (the JSON still holds the last figures) or the profiles can't
be read.
";

/// What the process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation<'a> {
    /// Start the GUI, which is what every ordinary launch wants.
    Gui,
    /// Open this profile's desktop app, then exit.
    OpenProfile(&'a str),
    /// Run the clone watch until stopped.
    WatchClones,
    /// Print the profiles' usage limits, then exit.
    Usage(UsageArgs),
    /// Print [`HELP`].
    Help,
    /// Print the version.
    Version,
    /// The arguments named a mode without giving it what it needs.
    Misuse(String),
}

/// What `claude-switch usage` was asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageArgs {
    /// Which profiles, as given; every profile when `None`.
    pub profile: Option<String>,
    /// Fetch even figures that are recent enough to reuse.
    pub force: bool,
    pub help: bool,
}

/// Pure: what `args` — the whole argument vector, program name included — asks
/// for.
///
/// Anything the flag does not appear in is an ordinary launch: LaunchServices
/// passes arguments of its own (`-psn_0_…`), and an unknown one must never
/// stop the app from opening. A command for people comes first, where
/// LaunchServices never puts one.
pub fn invocation(args: &[String]) -> Invocation<'_> {
    match args.get(1).map(String::as_str) {
        Some(USAGE_COMMAND) => return usage_invocation(&args[2..]),
        Some("help" | "-h" | "--help") => return Invocation::Help,
        Some("-v" | "--version") => return Invocation::Version,
        _ => {}
    }
    let Some(offset) = args.iter().skip(1).position(|arg| arg == OPEN_PROFILE_FLAG) else {
        if args.iter().skip(1).any(|arg| arg == WATCH_CLONES_FLAG) {
            return Invocation::WatchClones;
        }
        return Invocation::Gui;
    };
    match args.get(offset + 2) {
        Some(id) if !id.trim().is_empty() => Invocation::OpenProfile(id),
        _ => Invocation::Misuse(format!("{OPEN_PROFILE_FLAG} needs a profile id")),
    }
}

/// Pure: what `claude-switch usage` was asked for, from the arguments after it.
fn usage_invocation<'a>(args: &[String]) -> Invocation<'a> {
    let misuse = |problem: String| {
        Invocation::Misuse(format!(
            "{problem}\nRun `claude-switch {USAGE_COMMAND} --help` for usage."
        ))
    };
    let mut parsed = UsageArgs::default();
    for arg in args {
        match arg.as_str() {
            "--force" => parsed.force = true,
            "-h" | "--help" => parsed.help = true,
            option if option.starts_with('-') => {
                return misuse(format!("unknown option: {option}"));
            }
            profile => {
                if parsed.profile.is_some() {
                    return misuse("name one profile at most".to_owned());
                }
                parsed.profile = Some(profile.to_owned());
            }
        }
    }
    Invocation::Usage(parsed)
}

/// Print the profiles' usage limits; the process's exit status.
pub fn usage(args: &UsageArgs) -> i32 {
    usage_command::run(args)
}

/// Open the desktop app of the profile `id`, rebuilding its wrapper first if
/// that has fallen behind the vendor app.
///
/// Returns once the app is up, which can take a minute in the worst case: a
/// rebuild takes seconds, and macOS holds the first run of a bundle it has not
/// assessed before. The caller is a detached process nobody is waiting on, so
/// that is fine here.
pub fn open_profile(id: &str) -> Result<(), String> {
    let all = profiles::load().map_err(|err| err.message())?;
    let profile = all
        .iter()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| format!("no profile with id {id}"))?;
    if !profile.surfaces.gui {
        return Err(format!("profile {id} has no desktop surface"));
    }

    let version = env!("CARGO_PKG_VERSION");
    let bypass = match launch::open_profile(profile, version, launch::focus_pid) {
        // The profile's clone has been running for a while without the profile.
        // Nobody is at a window of ours to ask, so ask in a dialog.
        Err(AppError::StrayRunning(question)) => {
            if !confirm(&format!("Claude ({})", profile.name), &question) {
                return Ok(());
            }
            launch::open_profile_with(profile, version, launch::focus_pid, true)
        }
        other => other,
    }
    .map_err(|err| err.message())?;
    // Not a failure: the profile is open, just not through its wrapper. Worth
    // saying, because this is the only trace such a launch leaves anywhere.
    if let Some(bypass) = bypass {
        eprintln!("claude-switch: {bypass}");
    }
    Ok(())
}

/// Run the clone watch; never returns.
pub fn watch_clones() -> ! {
    clone_watch::run()
}

/// Ask `question` in a dialog titled `title`, and say whether the user went
/// ahead.
fn confirm(title: &str, question: &str) -> bool {
    std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display dialog (item 1 of argv) with title (item 2 of argv) buttons {\"Cancel\", \"Quit and Reopen\"} default button \"Quit and Reopen\" cancel button \"Cancel\" with icon caution",
            "-e",
            "end run",
            question,
            title,
        ])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        std::iter::once("/Applications/claude-switch.app/Contents/MacOS/claude-switch")
            .chain(values.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn no_arguments_starts_the_gui() {
        assert_eq!(invocation(&args(&[])), Invocation::Gui);
    }

    #[test]
    fn arguments_macos_adds_of_its_own_start_the_gui() {
        assert_eq!(invocation(&args(&["-psn_0_1466781"])), Invocation::Gui);
        assert_eq!(invocation(&args(&["--some-day"])), Invocation::Gui);
    }

    #[test]
    fn the_flag_takes_the_argument_after_it_as_the_profile_id() {
        assert_eq!(
            invocation(&args(&[OPEN_PROFILE_FLAG, "abc-123"])),
            Invocation::OpenProfile("abc-123")
        );
    }

    #[test]
    fn the_flag_is_found_wherever_it_sits() {
        assert_eq!(
            invocation(&args(&["-psn_0_1466781", OPEN_PROFILE_FLAG, "abc-123"])),
            Invocation::OpenProfile("abc-123")
        );
    }

    #[test]
    fn the_flag_without_a_usable_id_is_a_misuse_rather_than_a_silent_gui_launch() {
        for tail in [vec![OPEN_PROFILE_FLAG], vec![OPEN_PROFILE_FLAG, "   "]] {
            let argv = args(&tail);
            let parsed = invocation(&argv);
            assert!(
                matches!(parsed, Invocation::Misuse(_)),
                "{tail:?} parsed as {parsed:?}"
            );
        }
    }

    #[test]
    fn the_watch_flag_runs_the_clone_watch() {
        assert_eq!(
            invocation(&args(&[WATCH_CLONES_FLAG])),
            Invocation::WatchClones
        );
        assert_eq!(
            invocation(&args(&[OPEN_PROFILE_FLAG, "abc", WATCH_CLONES_FLAG])),
            Invocation::OpenProfile("abc"),
            "opening a profile wins"
        );
    }

    fn usage(profile: Option<&str>, force: bool) -> Invocation<'static> {
        Invocation::Usage(UsageArgs {
            profile: profile.map(str::to_owned),
            force,
            help: false,
        })
    }

    #[test]
    fn usage_takes_an_optional_profile_and_force_in_any_order() {
        assert_eq!(invocation(&args(&["usage"])), usage(None, false));
        assert_eq!(
            invocation(&args(&["usage", "work"])),
            usage(Some("work"), false)
        );
        assert_eq!(
            invocation(&args(&["usage", "--force", "My Work"])),
            usage(Some("My Work"), true)
        );
        assert_eq!(
            invocation(&args(&["usage", "codex", "--force"])),
            usage(Some("codex"), true)
        );
        assert_eq!(
            invocation(&args(&["usage", "--help"])),
            Invocation::Usage(UsageArgs {
                help: true,
                ..UsageArgs::default()
            })
        );
    }

    #[test]
    fn usage_refuses_an_unknown_option_or_a_second_profile() {
        for tail in [
            vec!["usage", "--forse"],
            vec!["usage", "-f"],
            vec!["usage", "work", "home"],
        ] {
            let argv = args(&tail);
            let parsed = invocation(&argv);
            assert!(
                matches!(&parsed, Invocation::Misuse(message) if message.contains("--help")),
                "{tail:?} parsed as {parsed:?}"
            );
        }
    }

    #[test]
    fn help_and_version_come_first() {
        for flag in ["help", "-h", "--help"] {
            assert_eq!(invocation(&args(&[flag])), Invocation::Help);
        }
        for flag in ["-v", "--version"] {
            assert_eq!(invocation(&args(&[flag])), Invocation::Version);
        }
    }

    #[test]
    fn a_command_anywhere_but_first_is_an_ordinary_launch() {
        assert_eq!(invocation(&args(&["-psn_0_1", "usage"])), Invocation::Gui);
        assert_eq!(
            invocation(&args(&[OPEN_PROFILE_FLAG, "usage"])),
            Invocation::OpenProfile("usage")
        );
    }

    #[test]
    fn a_program_path_that_looks_like_the_flag_is_not_the_flag() {
        let only_argv0 = vec![OPEN_PROFILE_FLAG.to_owned()];
        assert_eq!(invocation(&only_argv0), Invocation::Gui);
    }
}
