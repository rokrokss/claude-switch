use crate::app_kind::AppSpec;

/// Generate the bash script body for `Contents/MacOS/launcher`. Execs
/// `open -n -a "<gui_app_target>"` with `--user-data-dir` pointed at the
/// per-profile gui-data directory (app-neutral path under claude-switch/).
///
/// `gui_app_target` is the resolved stock GUI bundle to open — the caller
/// resolves it (see [`crate::paths::resolve_gui_app`]) so this stays a pure
/// function of its arguments rather than reaching into the filesystem
/// itself.
///
/// The script also exports the app's config-home env var
/// ([`AppSpec::cli_config_env`]) at the profile's `cli-config` dir — the same
/// home the CLI wrapper and usage provider use. Without it the app falls back
/// to the stock home: Codex (`CODEX_HOME`) shows the default account
/// regardless of `--user-data-dir`, and Claude's Code tab (`CLAUDE_CONFIG_DIR`)
/// shares `~/.claude`'s history, settings, plugins and MCP servers. `open`
/// propagates the exported environment to the launched GUI app, so the export
/// reaches it.
pub fn launcher_script(profile_id: &str, spec: &AppSpec, gui_app_target: &str) -> String {
    let profiles_base = "$HOME/Library/Application Support/claude-switch/profiles";
    let config_home_export = format!(
        "CONFIG_DIR=\"{profiles_base}/{profile_id}/cli-config\"\nexport {env}=\"$CONFIG_DIR\"\n",
        env = spec.cli_config_env,
    );
    format!(
        r#"#!/bin/bash
# claude-switch launcher — profile id: {profile_id}
DATA_DIR="{profiles_base}/{profile_id}/gui-data"
{config_home_export}exec open -n -a "{gui_app_target}" --args --user-data-dir="$DATA_DIR"
"#,
    )
}

/// Marks the script of a launcher that hands its launches to claude-switch, in
/// the signed-clone shape: the line that does it.
const HANDOFF_MARKER: &str = "exec \"$HOST\" --open-profile";

/// Generate the script of a profile whose desktop app runs from a signed clone.
///
/// The launcher stays a script launcher — its own bundle id, its badged icon,
/// at `/Applications/<App> (<Name>).app` — but hands the launch to the
/// claude-switch executable at `host_binary` with `--open-profile <id>`, which
/// focuses the profile if it runs and otherwise opens it from its clone,
/// making or refreshing that first (see [`crate::launch::open_profile`]).
///
/// Should claude-switch not be at `host_binary` any more, the script carries on
/// as [`launcher_script`] does, opening the stock app on the profile's data, so
/// the profile never becomes unlaunchable because of where claude-switch went.
pub fn handoff_launcher_script(
    profile_id: &str,
    spec: &AppSpec,
    host_binary: &str,
    gui_app_target: &str,
) -> String {
    let fallback = launcher_script(profile_id, spec, gui_app_target);
    let fallback_body = fallback
        .split_once('\n')
        .map_or(fallback.as_str(), |(_, rest)| rest);
    format!(
        r#"#!/bin/bash
# claude-switch launcher — profile id: {profile_id} — mode: signed clone
# Hands the launch to claude-switch, which opens this profile from its signed
# clone, or focuses it if it is running. Without claude-switch at HOST, the
# profile opens the stock way below.
HOST={host}
if [ -x "$HOST" ]; then
  {HANDOFF_MARKER} "{profile_id}"
fi
{fallback_body}"#,
        host = shell_quote(host_binary),
    )
}

/// Pure: whether `script` is a launcher script in the signed-clone shape.
pub fn hands_off(script: &str) -> bool {
    script.contains(HANDOFF_MARKER)
}

/// Pure: the claude-switch executable a signed-clone launcher script hands its
/// launches to, as [`handoff_launcher_script`] wrote it.
pub fn handoff_host(script: &str) -> Option<String> {
    let quoted = script.lines().find_map(|line| line.strip_prefix("HOST="))?;
    let inner = quoted.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("'\\''", "'"))
}

/// `text` as one single-quoted shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_kind::{CLAUDE, CODEX};

    #[test]
    fn script_uses_open_n_with_the_given_gui_app_target() {
        assert!(
            launcher_script("abc", &CLAUDE, "/Applications/Claude.app").contains(
                r#"exec open -n -a "/Applications/Claude.app" --args --user-data-dir="$DATA_DIR""#
            )
        );
        assert!(
            launcher_script("abc", &CODEX, "/Applications/ChatGPT.app").contains(
                r#"exec open -n -a "/Applications/ChatGPT.app" --args --user-data-dir="$DATA_DIR""#
            )
        );
    }

    #[test]
    fn script_includes_profile_id_in_data_dir() {
        assert!(
            launcher_script("id-1", &CLAUDE, "/Applications/Claude.app").contains(
                "DATA_DIR=\"$HOME/Library/Application Support/claude-switch/profiles/id-1/gui-data\""
            )
        );
    }

    #[test]
    fn script_starts_with_bash_shebang() {
        assert!(launcher_script("abc", &CLAUDE, "/Applications/Claude.app")
            .starts_with("#!/bin/bash\n"));
    }

    #[test]
    fn script_has_marker_comment_for_safe_overwrite_detection() {
        assert!(launcher_script("abc", &CLAUDE, "/Applications/Claude.app")
            .contains("# claude-switch launcher"));
    }

    #[test]
    fn codex_script_exports_config_home_at_profile_cli_config() {
        let script = launcher_script("abc", &CODEX, "/Applications/ChatGPT.app");
        assert!(script.contains(
            "CONFIG_DIR=\"$HOME/Library/Application Support/claude-switch/profiles/abc/cli-config\""
        ));
        assert!(script.contains(r#"export CODEX_HOME="$CONFIG_DIR""#));
    }

    const HOST: &str = "/Applications/claude-switch.app/Contents/MacOS/claude-switch";

    #[test]
    fn a_handoff_script_hands_the_launch_to_claude_switch_first() {
        let script = handoff_launcher_script("abc", &CLAUDE, HOST, "/Applications/Claude.app");
        assert!(script.starts_with("#!/bin/bash\n"));
        assert!(script.contains("# claude-switch launcher — profile id: abc — mode: signed clone"));
        assert!(script.contains(&format!("HOST='{HOST}'")));
        assert!(script.contains(r#"exec "$HOST" --open-profile "abc""#));
        let handoff = script.find("--open-profile").unwrap();
        let fallback = script.find("exec open -n -a").unwrap();
        assert!(
            handoff < fallback,
            "claude-switch is tried before the stock app"
        );
    }

    #[test]
    fn a_handoff_script_falls_back_to_the_stock_launch_with_the_config_home() {
        let script = handoff_launcher_script("abc", &CLAUDE, HOST, "/Applications/Claude.app");
        assert!(script.contains(
            r#"exec open -n -a "/Applications/Claude.app" --args --user-data-dir="$DATA_DIR""#
        ));
        assert!(script.contains(r#"export CLAUDE_CONFIG_DIR="$CONFIG_DIR""#));
        assert_eq!(script.matches("#!/bin/bash").count(), 1);
    }

    #[test]
    fn the_handoff_host_is_read_back_however_it_is_quoted() {
        let script = handoff_launcher_script("abc", &CLAUDE, HOST, "/Applications/Claude.app");
        assert!(hands_off(&script));
        assert_eq!(handoff_host(&script).as_deref(), Some(HOST));

        let odd = "/Users/o'neil/Apps/claude switch.app/Contents/MacOS/claude-switch";
        let script = handoff_launcher_script("abc", &CLAUDE, odd, "/Applications/Claude.app");
        assert_eq!(handoff_host(&script).as_deref(), Some(odd));

        let plain = launcher_script("abc", &CLAUDE, "/Applications/Claude.app");
        assert!(!hands_off(&plain));
        assert_eq!(handoff_host(&plain), None);
    }

    #[test]
    fn claude_script_exports_config_dir_at_profile_cli_config() {
        // The desktop app's Code tab resolves its config home from
        // CLAUDE_CONFIG_DIR; without it every profile shared ~/.claude.
        let script = launcher_script("abc", &CLAUDE, "/Applications/Claude.app");
        assert!(script.contains(
            "CONFIG_DIR=\"$HOME/Library/Application Support/claude-switch/profiles/abc/cli-config\""
        ));
        assert!(script.contains(r#"export CLAUDE_CONFIG_DIR="$CONFIG_DIR""#));
    }
}
