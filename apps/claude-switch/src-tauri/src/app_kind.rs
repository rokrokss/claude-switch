//! Per-app registry: the single source of truth describing each managed app
//! (Claude, ChatGPT, Gemini, Antigravity) and every Claude-specific seam generalised behind it.
//!
//! Every path, launcher, and launch incantation that used to hardcode
//! "Claude" reads from the `AppSpec` returned by [`spec`]. Adding a new app
//! becomes a const here plus a variant on [`AppKind`], not a copy of call sites.

use serde::{Deserialize, Serialize};

/// Which managed application a profile (or synthetic default entry) belongs to.
///
/// Serialises lowercase (`"claude"`, `"codex"`, `"gemini"`, `"antigravity"`) to match the TypeScript
/// `AppId` union and the `default:<app>` synthetic-id convention. Defaults to
/// [`AppKind::Claude`] so `profiles.json` documents predating the field
/// deserialise as Claude.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppKind {
    #[default]
    Claude,
    Codex,
    Gemini,
    Antigravity,
}

/// One possible on-disk identity for this app's stock GUI bundle.
///
/// A vendor can rename their app bundle without changing what it does (see
/// `CODEX`'s doc comment) — a spec lists every name claude-switch should
/// recognise, newest first, instead of hardcoding a single string that goes
/// stale the moment that happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiBundleCandidate {
    /// Stock bundle file name under `/Applications`, e.g. `"Claude.app"`.
    pub bundle_name: &'static str,
    /// Executable inside `Contents/MacOS`, used both to launch the bundle by
    /// absolute path and to single out its main process in `ps` output, e.g.
    /// `"Claude"`.
    pub macos_exec: &'static str,
}

/// Static description of one managed app. All fields are `&'static str` (plus
/// `has_usage`) so a spec lives in a `const` and is referenced without alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSpec {
    /// Human-facing product name for the GUI/profile-kind identity — shown in
    /// the profile list, create-profile dialog, and any message about the
    /// desktop bundle. E.g. `"Claude"`, or `"ChatGPT"` for ChatGPT profiles: the
    /// unified desktop app Codex now ships inside (see [`GuiBundleCandidate`]).
    pub display_name: &'static str,
    /// Human-facing name for the CLI specifically, e.g. `"Claude"`. Diverges
    /// from `display_name` only for Codex (`"Codex"`), whose desktop app was
    /// folded into ChatGPT while the CLI kept its own identity.
    pub cli_display_name: &'static str,
    /// Possible on-disk identities of the stock GUI bundle, newest first. See
    /// [`GuiBundleCandidate`].
    pub gui_bundle_candidates: &'static [GuiBundleCandidate],
    /// Directory name under `~/Library/Application Support` holding the stock
    /// GUI data, e.g. `"Claude"`.
    pub gui_support_dir_name: &'static str,
    /// Prefix for generated launcher bundles: `"<prefix> (<name>).app"`.
    pub launcher_prefix: &'static str,
    /// Prefixes this app's generated launcher bundles used before a rename of
    /// `launcher_prefix`, so a bundle generated under the old prefix can still
    /// be found and cleaned up on the next `generate()`. Empty unless
    /// `launcher_prefix` has changed historically.
    pub legacy_launcher_prefixes: &'static [&'static str],
    /// Real CLI binary the wrapper execs, e.g. `"claude"`.
    pub cli_binary: &'static str,
    /// Prefix for generated CLI wrappers: `"<prefix>-<slug>"`.
    pub cli_wrapper_prefix: &'static str,
    /// Env var pointing the app at the profile's config home (its `cli-config`
    /// dir), e.g. `"CLAUDE_CONFIG_DIR"`. The CLI wrapper exports it, and so
    /// does the desktop launcher, so the desktop app and the CLI share one
    /// config home.
    ///
    /// Codex needs it for auth: ChatGPT.app reads its login from `CODEX_HOME`,
    /// and `--user-data-dir` alone only isolates the browser layer. Claude keeps
    /// its GUI auth in the `--user-data-dir`, but the Claude Code agent it runs
    /// (the Code tab) resolves its config home from `CLAUDE_CONFIG_DIR`. Without
    /// the export every Claude desktop profile shared the stock `~/.claude`:
    /// one session history, and one set of hooks, permissions, plugins and MCP
    /// servers across accounts.
    pub cli_config_env: &'static str,
    /// Directory, inside the one [`cli_config_env`] points at, that the CLI
    /// keeps its config in — `None` when the variable names the config dir
    /// itself, as `CLAUDE_CONFIG_DIR` and `CODEX_HOME` do. Gemini's
    /// `GEMINI_CLI_HOME` stands in for `$HOME`, so a profile's Gemini config
    /// lives in `cli-config/.gemini`, as the stock one lives in `~/.gemini`.
    ///
    /// [`cli_config_env`]: AppSpec::cli_config_env
    pub cli_config_subdir: Option<&'static str>,
    /// Stock CLI config directory name under `$HOME`, e.g. `".claude"`.
    pub cli_stock_config_dir_name: &'static str,
    /// Entries of the stock config dir that belong to another app, which
    /// importing the stock install copies nothing of and leaves where they
    /// are. Google's Antigravity keeps its data in `~/.gemini` beside Gemini
    /// CLI's, and moving it into a Gemini profile would take it from
    /// Antigravity.
    pub cli_import_leaves: &'static [&'static str],
    /// Further variables the CLI wrapper exports beside
    /// [`cli_config_env`](AppSpec::cli_config_env), and usage reads set too.
    pub cli_extra_env: &'static [(&'static str, &'static str)],
    /// Set for an app whose [`cli_config_env`](AppSpec::cli_config_env) is
    /// `HOME` itself: every entry of the real home but these is linked into
    /// the profile's, so what the CLI runs (git, ssh, toolchains) finds the
    /// user's usual setup. The entries named stay the profile's own.
    pub cli_home_links_except: Option<&'static [&'static str]>,
    /// Whether this app exposes account usage/quota stats.
    pub has_usage: bool,
    /// Config-dir entries a profile *inherits* from the stock install
    /// (`~/.claude`, `~/.codex`) instead of isolating, by way of a symlink
    /// created in the profile's `cli-config` dir. Named from the directory
    /// [`cli_config_env`] points at, so for an app with a
    /// [`cli_config_subdir`] they are named from `$HOME` (`.gemini/skills`).
    ///
    /// The isolation boundary is identity, not craft: profiles exist to
    /// separate account, auth, quota and history. These entries are inert
    /// content describing *how* the user works — skills, subagents, slash
    /// commands, rules, global instructions — so duplicating them per profile
    /// is upkeep with no security benefit. Desktop launchers link them too,
    /// since they point the desktop app's agent at the same `cli-config` dir.
    ///
    /// Deliberately excluded: `settings.json` (carries `hooks`, which run
    /// arbitrary shell, and `permissions` allowlists), `hooks/`, `plugins/`
    /// (absolute install paths, project-scoped entries, and mutable state that
    /// concurrently-running profiles would race on) and anything holding
    /// credentials or session history.
    ///
    /// [`cli_config_env`]: AppSpec::cli_config_env
    /// [`cli_config_subdir`]: AppSpec::cli_config_subdir
    pub shared_surfaces: &'static [&'static str],
    /// Whether a profile of this app can run its desktop app from a signed
    /// clone: an APFS copy of the vendor bundle that keeps the vendor's
    /// signature, told apart from the original by a Finder custom icon (see
    /// [`crate::launchers::clone`]). Only Claude, and only on macOS.
    pub supports_signed_clone: bool,
}

pub const CLAUDE: AppSpec = AppSpec {
    display_name: "Claude",
    cli_display_name: "Claude",
    gui_bundle_candidates: &[GuiBundleCandidate {
        bundle_name: "Claude.app",
        macos_exec: "Claude",
    }],
    gui_support_dir_name: "Claude",
    launcher_prefix: "Claude",
    legacy_launcher_prefixes: &[],
    cli_binary: "claude",
    cli_wrapper_prefix: "claude",
    cli_config_env: "CLAUDE_CONFIG_DIR",
    cli_config_subdir: None,
    cli_stock_config_dir_name: ".claude",
    cli_import_leaves: &[],
    cli_extra_env: &[],
    cli_home_links_except: None,
    has_usage: true,
    shared_surfaces: &[
        "CLAUDE.md",
        "agents",
        "commands",
        "output-styles",
        "rules",
        "skills",
        "workflows",
    ],
    supports_signed_clone: true,
};

pub const CODEX: AppSpec = AppSpec {
    // OpenAI folded the standalone Codex desktop app into ChatGPT (same
    // CFBundleIdentifier, renamed bundle/executable, confirmed no separate
    // Codex app is distributed any more — see learn.chatgpt.com/docs/app).
    // The CLI is unaffected and still called Codex, hence the display-name
    // split below.
    display_name: "ChatGPT",
    cli_display_name: "Codex",
    // Newest first so a not-yet-updated install of the old standalone app is
    // still recognised.
    gui_bundle_candidates: &[
        GuiBundleCandidate {
            bundle_name: "ChatGPT.app",
            macos_exec: "ChatGPT",
        },
        GuiBundleCandidate {
            bundle_name: "Codex.app",
            macos_exec: "Codex",
        },
    ],
    gui_support_dir_name: "Codex",
    launcher_prefix: "ChatGPT",
    legacy_launcher_prefixes: &["Codex"],
    cli_binary: "codex",
    cli_wrapper_prefix: "codex",
    cli_config_env: "CODEX_HOME",
    cli_config_subdir: None,
    cli_stock_config_dir_name: ".codex",
    cli_import_leaves: &[],
    cli_extra_env: &[],
    cli_home_links_except: None,
    has_usage: true,
    shared_surfaces: &["AGENTS.md", "rules", "skills"],
    supports_signed_clone: false,
};

pub const GEMINI: AppSpec = AppSpec {
    display_name: "Gemini",
    cli_display_name: "Gemini",
    // CLI only. Google's Gemini.app is a sandboxed native app with no
    // `--user-data-dir` to point at a profile's data, so there is no desktop
    // launcher to build. The GUI fields below are never used.
    gui_bundle_candidates: &[],
    gui_support_dir_name: "Gemini",
    launcher_prefix: "Gemini",
    legacy_launcher_prefixes: &[],
    cli_binary: "gemini",
    cli_wrapper_prefix: "gemini",
    // Gemini CLI keeps its sign-in in `<GEMINI_CLI_HOME>/.gemini/oauth_creds.json`
    // (unless `GEMINI_FORCE_ENCRYPTED_FILE_STORAGE` moves it to one Keychain
    // item every profile would share), so the variable isolates the account.
    cli_config_env: "GEMINI_CLI_HOME",
    cli_config_subdir: Some(".gemini"),
    cli_stock_config_dir_name: ".gemini",
    // Antigravity's: its desktop app's and IDE's data, its CLI's, and its
    // global customizations (`config/`, MCP servers and hooks).
    cli_import_leaves: &[
        "antigravity",
        "antigravity-backup",
        "antigravity-cli",
        "antigravity-ide",
        "config",
    ],
    cli_extra_env: &[],
    cli_home_links_except: None,
    has_usage: false,
    // `.agents` because Gemini reads the cross-tool `~/.agents/skills` from
    // under `GEMINI_CLI_HOME` too. Left out: `settings.json` (hooks, MCP
    // servers, the sign-in method), `policies/` (tool allowlists) and
    // `extensions/` (installed code), as for Claude.
    shared_surfaces: &[
        ".agents",
        ".gemini/GEMINI.md",
        ".gemini/agents",
        ".gemini/commands",
        ".gemini/skills",
    ],
    supports_signed_clone: false,
};

/// Google's Antigravity CLI, `agy`: where Google moved its individual plans
/// (free, AI Pro, AI Ultra) when it ended their Gemini CLI sign-in.
pub const ANTIGRAVITY: AppSpec = AppSpec {
    display_name: "Antigravity",
    cli_display_name: "Antigravity",
    // CLI only: a profile is its `agy-<slug>` wrapper. The GUI fields below
    // are never used.
    gui_bundle_candidates: &[],
    gui_support_dir_name: "Antigravity",
    launcher_prefix: "Antigravity",
    legacy_launcher_prefixes: &[],
    cli_binary: "agy",
    cli_wrapper_prefix: "agy",
    // agy has no config-home variable. It keeps its state in
    // `~/.gemini/antigravity-cli` and its sign-in in one Keychain item
    // (`gemini`/`antigravity`) every account would share, so the wrapper gives
    // it a home of its own, and has it keep the sign-in in that home's
    // `.gemini/antigravity-cli/antigravity-oauth-token` instead: agy uses the
    // file rather than the Keychain when `SSH_TTY` is set (as CodexBar's
    // per-account usage reads rely on).
    cli_config_env: "HOME",
    cli_config_subdir: Some(".gemini/antigravity-cli"),
    cli_stock_config_dir_name: ".gemini/antigravity-cli",
    cli_import_leaves: &[],
    cli_extra_env: &[("SSH_TTY", "claude-switch")],
    // The profile's `.gemini` is its own: agy's state and sign-in, and
    // Antigravity's MCP servers and hooks (`.gemini/config`).
    cli_home_links_except: Some(&[".gemini"]),
    has_usage: true,
    // Antigravity's global rules.
    shared_surfaces: &[".gemini/GEMINI.md"],
    supports_signed_clone: false,
};

/// Borrow the static [`AppSpec`] for a kind.
pub fn spec(kind: AppKind) -> &'static AppSpec {
    match kind {
        AppKind::Claude => &CLAUDE,
        AppKind::Codex => &CODEX,
        AppKind::Gemini => &GEMINI,
        AppKind::Antigravity => &ANTIGRAVITY,
    }
}

impl AppSpec {
    /// Whether a profile of this app can have a desktop launcher.
    pub fn has_desktop(&self) -> bool {
        !self.gui_bundle_candidates.is_empty()
    }
}

impl AppKind {
    /// Convenience accessor for this kind's [`AppSpec`].
    pub fn spec(self) -> &'static AppSpec {
        spec(self)
    }

    /// Lowercase token used in serialisation and synthetic ids.
    pub fn as_str(self) -> &'static str {
        match self {
            AppKind::Claude => "claude",
            AppKind::Codex => "codex",
            AppKind::Gemini => "gemini",
            AppKind::Antigravity => "antigravity",
        }
    }

    /// Parse a lowercase app token (`"claude"`/`"codex"`/`"gemini"`/`"antigravity"`).
    pub fn from_token(token: &str) -> Option<AppKind> {
        match token {
            "claude" => Some(AppKind::Claude),
            "codex" => Some(AppKind::Codex),
            "gemini" => Some(AppKind::Gemini),
            "antigravity" => Some(AppKind::Antigravity),
            _ => None,
        }
    }

    /// Parse a synthetic default-entry id of the form `default:<app>`. Returns
    /// `None` for managed ids or unknown apps.
    pub fn from_default_id(id: &str) -> Option<AppKind> {
        id.strip_prefix("default:").and_then(AppKind::from_token)
    }
}

/// Format the synthetic default-entry id for a kind, e.g. `"default:claude"`.
pub fn default_id(kind: AppKind) -> String {
    format!("default:{}", kind.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_kind_is_claude() {
        assert_eq!(AppKind::default(), AppKind::Claude);
    }

    #[test]
    fn codex_spec_exposes_codex_surface() {
        let codex = spec(AppKind::Codex);
        assert_eq!(
            codex.gui_bundle_candidates,
            &[
                GuiBundleCandidate {
                    bundle_name: "ChatGPT.app",
                    macos_exec: "ChatGPT",
                },
                GuiBundleCandidate {
                    bundle_name: "Codex.app",
                    macos_exec: "Codex",
                },
            ]
        );
        assert_eq!(codex.display_name, "ChatGPT");
        assert_eq!(codex.cli_display_name, "Codex");
        assert_eq!(codex.launcher_prefix, "ChatGPT");
        assert_eq!(codex.legacy_launcher_prefixes, &["Codex"]);
        assert_eq!(codex.cli_binary, "codex");
        assert_eq!(codex.cli_config_env, "CODEX_HOME");
        assert_eq!(codex.cli_stock_config_dir_name, ".codex");
        assert!(codex.has_usage);
    }

    #[test]
    fn gemini_spec_is_cli_only_with_a_home_stand_in() {
        let gemini = spec(AppKind::Gemini);
        assert!(!gemini.has_desktop());
        assert!(spec(AppKind::Claude).has_desktop());
        assert_eq!(gemini.cli_binary, "gemini");
        assert_eq!(gemini.cli_config_env, "GEMINI_CLI_HOME");
        assert_eq!(gemini.cli_config_subdir, Some(".gemini"));
        assert_eq!(gemini.cli_stock_config_dir_name, ".gemini");
        assert!(!gemini.has_usage);
        assert!(!gemini.supports_signed_clone);
    }

    #[test]
    fn antigravity_spec_gives_agy_a_home_of_its_own() {
        let antigravity = spec(AppKind::Antigravity);
        assert!(!antigravity.has_desktop());
        assert_eq!(antigravity.cli_binary, "agy");
        assert_eq!(antigravity.cli_wrapper_prefix, "agy");
        assert_eq!(antigravity.cli_config_env, "HOME");
        assert_eq!(
            antigravity.cli_config_subdir,
            Some(".gemini/antigravity-cli")
        );
        assert_eq!(antigravity.cli_extra_env, &[("SSH_TTY", "claude-switch")]);
        assert_eq!(antigravity.cli_home_links_except, Some(&[".gemini"][..]));
        assert!(antigravity.has_usage);
    }

    #[test]
    fn default_id_round_trips() {
        for kind in [
            AppKind::Claude,
            AppKind::Codex,
            AppKind::Gemini,
            AppKind::Antigravity,
        ] {
            assert_eq!(AppKind::from_default_id(&default_id(kind)), Some(kind));
        }
    }

    #[test]
    fn from_default_id_rejects_non_default_and_unknown() {
        assert_eq!(AppKind::from_default_id("some-managed-uuid"), None);
        assert_eq!(AppKind::from_default_id("default:grok"), None);
        assert_eq!(AppKind::from_default_id("claude"), None);
    }

    #[test]
    fn app_kind_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&AppKind::Codex).unwrap(), "\"codex\"");
        let parsed: AppKind = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(parsed, AppKind::Codex);
    }
}
