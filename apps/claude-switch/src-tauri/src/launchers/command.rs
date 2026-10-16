//! The `claude-switch` command, `~/.local/bin/claude-switch`: a script that
//! runs this app's executable, so `claude-switch usage` works in a terminal.
//! Installed and removed from Settings; run bare, it prints its help rather
//! than opening the app.
//!
//! Like the profiles' wrappers ([`super::cli`]), it carries a marker line,
//! and a file without it is never overwritten or removed. The marker is its
//! own, so a profile wrapper of the same name (slug `switch`) and this
//! command never take each other's place.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::paths::local_bin_dir;

pub const MARKER: &str = "# claude-switch command line";
const NAME: &str = "claude-switch";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    NotInstalled,
    Installed,
    /// Something else has the name.
    Conflict,
}

/// The command as Settings shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandLine {
    /// Where it is, or would be, installed, `~` for the home directory.
    pub path: String,
    pub state: State,
}

#[tauri::command]
pub fn command_line() -> AppResult<CommandLine> {
    describe(&local_bin_dir()?)
}

#[tauri::command]
pub fn install_command_line() -> AppResult<CommandLine> {
    let bin_dir = local_bin_dir()?;
    install_at(&bin_dir, &std::env::current_exe()?)?;
    describe(&bin_dir)
}

#[tauri::command]
pub fn uninstall_command_line() -> AppResult<CommandLine> {
    let bin_dir = local_bin_dir()?;
    uninstall_at(&bin_dir)?;
    describe(&bin_dir)
}

/// Points an installed command at this executable, which may have moved
/// since it was installed. Reports rather than fails: called at startup.
pub fn sync_best_effort() {
    let result = local_bin_dir().and_then(|bin_dir| sync_at(&bin_dir, &std::env::current_exe()?));
    if let Err(err) = result {
        eprintln!(
            "claude-switch: could not update the claude-switch command: {}",
            err.message()
        );
    }
}

/// Pure: the script, running `executable` with the arguments it was given,
/// or with `help` when given none.
pub fn script(executable: &Path) -> String {
    format!(
        "#!/bin/bash\n{MARKER}\nexec {} \"${{@:-help}}\"\n",
        shell_quoted(&executable.to_string_lossy())
    )
}

fn describe(bin_dir: &Path) -> AppResult<CommandLine> {
    let path = bin_dir.join(NAME);
    let shown = match dirs::home_dir() {
        Some(home) => match path.strip_prefix(&home) {
            Ok(relative) => format!("~/{}", relative.display()),
            Err(_) => path.display().to_string(),
        },
        None => path.display().to_string(),
    };
    Ok(CommandLine {
        path: shown,
        state: state_at(&path),
    })
}

fn state_at(path: &Path) -> State {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return State::NotInstalled;
    };
    // A link would have the script written through it, to wherever it points.
    if metadata.file_type().is_symlink() {
        return State::Conflict;
    }
    match fs::read_to_string(path) {
        Ok(text) if text.contains(MARKER) => State::Installed,
        _ => State::Conflict,
    }
}

fn install_at(bin_dir: &Path, executable: &Path) -> AppResult<PathBuf> {
    let path = bin_dir.join(NAME);
    if state_at(&path) == State::Conflict {
        return Err(AppError::Validation(format!(
            "{} already exists and was not created by claude-switch; refusing to overwrite",
            path.display()
        )));
    }
    fs::create_dir_all(bin_dir)?;
    fs::write(&path, script(executable))?;
    let mut permissions = fs::metadata(&path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions)?;
    Ok(path)
}

fn uninstall_at(bin_dir: &Path) -> AppResult<()> {
    let path = bin_dir.join(NAME);
    match state_at(&path) {
        State::NotInstalled => Ok(()),
        State::Installed => Ok(fs::remove_file(&path)?),
        State::Conflict => Err(AppError::Validation(format!(
            "{} was not created by claude-switch; refusing to delete",
            path.display()
        ))),
    }
}

/// Rewrites an installed command that runs another executable; leaves
/// anything else as it is.
fn sync_at(bin_dir: &Path, executable: &Path) -> AppResult<()> {
    let path = bin_dir.join(NAME);
    if state_at(&path) != State::Installed {
        return Ok(());
    }
    if fs::read_to_string(&path)? != script(executable) {
        install_at(bin_dir, executable)?;
    }
    Ok(())
}

/// `value` as one single-quoted shell word.
fn shell_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launchers::cli;
    use std::process::Command;

    const APP: &str = "/Applications/claude-switch.app/Contents/MacOS/claude-switch";

    #[test]
    fn the_script_runs_the_executable_or_its_help() {
        assert_eq!(
            script(Path::new(APP)),
            format!("#!/bin/bash\n{MARKER}\nexec '{APP}' \"${{@:-help}}\"\n")
        );
    }

    #[test]
    fn the_script_passes_arguments_through_and_help_when_there_are_none() {
        let dir = tempfile::tempdir().unwrap();
        // An executable with a quote in its path that prints its arguments.
        let executable = dir.path().join("it's here");
        fs::write(&executable, "#!/bin/bash\nprintf '[%s]' \"$@\"\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let bin_dir = dir.path().join("bin");
        let installed = install_at(&bin_dir, &executable).unwrap();
        let run = |args: &[&str]| {
            let output = Command::new(&installed).args(args).output().unwrap();
            String::from_utf8(output.stdout).unwrap()
        };
        assert_eq!(run(&[]), "[help]");
        assert_eq!(
            run(&["usage", "my work", "--force"]),
            "[usage][my work][--force]"
        );
    }

    #[test]
    fn installs_and_uninstalls() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("bin");
        assert_eq!(state_at(&bin_dir.join(NAME)), State::NotInstalled);
        let path = install_at(&bin_dir, Path::new(APP)).unwrap();
        assert_eq!(state_at(&path), State::Installed);
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // Installing again is fine.
        install_at(&bin_dir, Path::new(APP)).unwrap();
        uninstall_at(&bin_dir).unwrap();
        assert_eq!(state_at(&path), State::NotInstalled);
        uninstall_at(&bin_dir).unwrap();
    }

    #[test]
    fn a_file_of_someone_else_s_is_neither_overwritten_nor_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        fs::write(&path, "#!/bin/sh\necho mine\n").unwrap();
        assert_eq!(state_at(&path), State::Conflict);
        assert!(install_at(dir.path(), Path::new(APP)).is_err());
        assert!(uninstall_at(dir.path()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "#!/bin/sh\necho mine\n");
    }

    #[test]
    fn a_link_is_someone_else_s() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, format!("{MARKER}\n")).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(NAME)).unwrap();
        assert_eq!(state_at(&dir.path().join(NAME)), State::Conflict);
        assert!(install_at(dir.path(), Path::new(APP)).is_err());
    }

    #[test]
    fn a_profile_wrapper_of_the_same_name_and_the_command_keep_apart() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = format!(
            "#!/bin/bash\n{} — profile: abc\nexec claude \"$@\"\n",
            cli::MARKER
        );
        fs::write(dir.path().join(NAME), wrapper).unwrap();
        assert_eq!(state_at(&dir.path().join(NAME)), State::Conflict);

        let other = tempfile::tempdir().unwrap();
        install_at(other.path(), Path::new(APP)).unwrap();
        let spec = crate::app_kind::AppKind::Claude.spec();
        assert!(cli::remove_at("switch", spec, other.path()).is_err());
        assert_eq!(state_at(&other.path().join(NAME)), State::Installed);
    }

    #[test]
    fn sync_points_an_installed_command_at_the_executable_and_leaves_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let moved =
            Path::new("/Users/me/Applications/claude-switch.app/Contents/MacOS/claude-switch");
        sync_at(dir.path(), moved).unwrap();
        assert_eq!(state_at(&dir.path().join(NAME)), State::NotInstalled);

        install_at(dir.path(), Path::new(APP)).unwrap();
        sync_at(dir.path(), moved).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(NAME)).unwrap(),
            script(moved)
        );

        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join(NAME), "mine").unwrap();
        sync_at(other.path(), moved).unwrap();
        assert_eq!(fs::read_to_string(other.path().join(NAME)).unwrap(), "mine");
    }
}
