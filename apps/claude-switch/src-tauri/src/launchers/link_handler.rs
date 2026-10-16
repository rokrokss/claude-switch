//! Who opens `claude://` links while a profile signs in.
//!
//! Claude Desktop signs in with Google one of two ways, and picks by whether it
//! is the default handler of `claude://` links. If it is, it signs in through
//! the default browser, which hands the result back as a `claude://` link. If it
//! isn't, it signs in in an `ASWebAuthenticationSession`, which hands the result
//! straight back to the process that asked. It tells by bundle identifier, and a
//! signed clone has the stock app's, as does a profile that runs the stock app
//! itself, so either always takes the browser — and LaunchServices gives the
//! link to whichever Claude it picks, which drops a sign-in it did not start.
//! The profile never signs in while another Claude runs.
//!
//! So while such a profile ([`covers`]) runs signed out, the clone watch makes
//! claude-switch the default handler, and once none does it hands the scheme
//! back ([`sync`]). Every Claude makes itself the default handler again when it
//! starts, so the watch takes it again on its next look. claude-switch declares
//! the scheme in its Info.plist for this, and passes any link it is given
//! meanwhile on to Claude, where it would have gone otherwise ([`forward`]).

use crate::accounts;
use crate::app_kind::{AppKind, CLAUDE};
use crate::error::{AppError, AppResult};
use crate::launch;
use crate::launchers::{clone, wrapper};
use crate::paths::{profile_dir, resolve_gui_app};
use crate::profiles::Profile;

/// The scheme Claude Desktop signs in through.
pub const SCHEME: &str = "claude";

/// claude-switch's own bundle identifier, as `tauri.conf.json` sets it.
const HOST_BUNDLE_ID: &str = "app.claude-switch";

/// Who should be the default handler of [`SCHEME`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handler {
    Host,
    Vendor,
}

/// Pure: who to make the default handler of [`SCHEME`], if it should change.
///
/// `signing_in` says whether some profile runs signed out, `current` is the
/// bundle identifier of the app that opens the scheme now, `vendor` Claude's,
/// and `host_can` whether claude-switch is registered to open it at all. Only
/// Claude is ever replaced, and only claude-switch handed back: a handler the
/// user chose is theirs.
fn plan(signing_in: bool, current: Option<&str>, vendor: &str, host_can: bool) -> Option<Handler> {
    // LaunchServices doesn't keep the case of a bundle identifier.
    let is = |id: &str| current.is_some_and(|current| current.eq_ignore_ascii_case(id));
    if signing_in && host_can && is(vendor) {
        Some(Handler::Host)
    } else if !signing_in && is(HOST_BUNDLE_ID) {
        Some(Handler::Vendor)
    } else {
        None
    }
}

/// Pure: whether `profile`'s desktop app is Claude under Claude's own bundle
/// identifier — the stock app or a signed clone — and so signs in through the
/// browser whenever Claude is the default handler. A wrapper
/// (`distinct_dock_icon`) has an identifier of its own and never is.
pub fn covers(profile: &Profile) -> bool {
    profile.app == AppKind::Claude && profile.surfaces.gui && !profile.distinct_dock_icon
}

/// Make claude-switch the default handler of [`SCHEME`] while one of
/// `profiles` that it [`covers`] runs signed out, and hand the scheme back to
/// Claude once none does.
pub fn sync(profiles: &[Profile]) -> AppResult<()> {
    let Some(vendor) = resolve_gui_app(&CLAUDE) else {
        return Ok(());
    };
    let Some(vendor_id) = wrapper::bundle_identifier(&vendor.bundle_path) else {
        return Ok(());
    };
    let exec = clone::vendor_executable(&vendor);
    let processes = launch::process_list()?;
    let signing_in = signing_in(profiles, &processes, &exec)?;
    match plan(
        signing_in,
        current_handler().as_deref(),
        &vendor_id,
        host_can_open(),
    ) {
        Some(Handler::Host) => {
            set_handler(HOST_BUNDLE_ID)?;
            eprintln!(
                "claude-switch: a profile is signing in; {SCHEME}:// links come here meanwhile"
            );
        }
        Some(Handler::Vendor) => {
            set_handler(&vendor_id)?;
            eprintln!(
                "claude-switch: no profile is signing in; {SCHEME}:// links go to Claude again"
            );
        }
        None => {}
    }
    Ok(())
}

/// Whether one of `profiles` that it [`covers`] has its desktop app running,
/// signed out.
fn signing_in(profiles: &[Profile], processes: &str, exec: &str) -> AppResult<bool> {
    for profile in profiles.iter().filter(|profile| covers(profile)) {
        let gui_data = profile_dir(&profile.id)?.join("gui-data");
        let running =
            launch::find_running_pid(processes, &gui_data.display().to_string(), exec).is_some();
        if running && !accounts::desktop_signed_in(&gui_data) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Pass a `claude://` link claude-switch was given on to Claude, where
/// LaunchServices would have sent it had claude-switch not held the scheme.
pub fn forward(url: &str) -> AppResult<()> {
    let vendor = resolve_gui_app(&CLAUDE)
        .ok_or_else(|| AppError::Validation(format!("{} isn't installed", CLAUDE.display_name)))?;
    let vendor_id = wrapper::bundle_identifier(&vendor.bundle_path).ok_or_else(|| {
        AppError::Validation(format!(
            "cannot read the bundle identifier of {}",
            vendor.bundle_path.display()
        ))
    })?;
    let status = launch::open_command()
        .arg("-b")
        .arg(&vendor_id)
        .arg(url)
        .status()?;
    if !status.success() {
        return Err(AppError::Validation(format!(
            "`open -b {vendor_id}` exited with status {status}"
        )));
    }
    Ok(())
}

/// The URL whose handler is asked about, as Electron asks.
#[cfg(target_os = "macos")]
fn scheme_url() -> Option<objc2::rc::Retained<objc2_foundation::NSURL>> {
    use objc2_foundation::{NSString, NSURL};
    NSURL::URLWithString(&NSString::from_str(&format!("{SCHEME}://")))
}

/// The bundle identifier of the app that opens [`SCHEME`] links now, found the
/// way Claude itself looks when it decides how to sign in.
#[cfg(target_os = "macos")]
fn current_handler() -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSBundle;
    let url = scheme_url()?;
    let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url)?;
    Some(
        NSBundle::bundleWithURL(&app)?
            .bundleIdentifier()?
            .to_string(),
    )
}

#[cfg(not(target_os = "macos"))]
fn current_handler() -> Option<String> {
    None
}

/// Whether claude-switch is registered to open [`SCHEME`] links. One that isn't
/// (a development build) could not take them, and LaunchServices would quietly
/// keep giving them to Claude.
#[cfg(target_os = "macos")]
fn host_can_open() -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSBundle;
    let Some(url) = scheme_url() else {
        return false;
    };
    NSWorkspace::sharedWorkspace()
        .URLsForApplicationsToOpenURL(&url)
        .iter()
        .filter_map(|app| NSBundle::bundleWithURL(&app)?.bundleIdentifier())
        .any(|id| id.to_string().eq_ignore_ascii_case(HOST_BUNDLE_ID))
}

#[cfg(not(target_os = "macos"))]
fn host_can_open() -> bool {
    false
}

/// Make the app with bundle identifier `id` the default handler of [`SCHEME`],
/// the same way Claude makes itself the default handler.
#[cfg(target_os = "macos")]
fn set_handler(id: &str) -> AppResult<()> {
    use objc2_core_foundation::CFString;
    // SAFETY: both arguments are valid CFStrings for the length of the call.
    #[allow(deprecated)]
    let status = unsafe {
        objc2_core_services::LSSetDefaultHandlerForURLScheme(
            &CFString::from_str(SCHEME),
            &CFString::from_str(id),
        )
    };
    if status != 0 {
        return Err(AppError::Validation(format!(
            "could not make {id} the handler of {SCHEME}:// links (OSStatus {status})"
        )));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn set_handler(_id: &str) -> AppResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::Surfaces;

    const VENDOR: &str = "com.anthropic.claudefordesktop";

    fn profile(app: AppKind, gui: bool, distinct_dock_icon: bool, signed_clone: bool) -> Profile {
        Profile {
            id: "a".into(),
            app,
            name: "Work".into(),
            slug: "work".into(),
            color: "#7C3AED".into(),
            created_at: "2026-10-16T00:00:00Z".into(),
            surfaces: Surfaces { gui, cli: false },
            distinct_dock_icon,
            signed_clone,
            last_used_at: None,
        }
    }

    #[test]
    fn profiles_running_claude_under_its_own_bundle_id_are_covered() {
        assert!(
            covers(&profile(AppKind::Claude, true, false, false)),
            "the stock app"
        );
        assert!(
            covers(&profile(AppKind::Claude, true, false, true)),
            "a signed clone"
        );
        assert!(
            !covers(&profile(AppKind::Claude, true, true, false)),
            "a wrapper has its own bundle id"
        );
        assert!(
            !covers(&profile(AppKind::Claude, false, false, false)),
            "no desktop app"
        );
        assert!(
            !covers(&profile(AppKind::Claude, false, false, true)),
            "no desktop app"
        );
        assert!(
            !covers(&profile(AppKind::Codex, true, false, false)),
            "not Claude"
        );
    }

    #[test]
    fn claude_is_replaced_only_while_a_profile_signs_in_and_claude_switch_can_take_over() {
        assert_eq!(plan(true, Some(VENDOR), VENDOR, true), Some(Handler::Host));
        assert_eq!(
            plan(true, Some("COM.Anthropic.ClaudeForDesktop"), VENDOR, true),
            Some(Handler::Host),
            "the case of an identifier doesn't matter"
        );
        assert_eq!(
            plan(true, Some(VENDOR), VENDOR, false),
            None,
            "not registered"
        );
        assert_eq!(
            plan(false, Some(VENDOR), VENDOR, true),
            None,
            "nobody signing in"
        );
        assert_eq!(
            plan(true, Some(HOST_BUNDLE_ID), VENDOR, true),
            None,
            "held already"
        );
    }

    #[test]
    fn the_scheme_goes_back_to_claude_once_nobody_signs_in() {
        assert_eq!(
            plan(false, Some(HOST_BUNDLE_ID), VENDOR, true),
            Some(Handler::Vendor)
        );
        assert_eq!(
            plan(false, Some(HOST_BUNDLE_ID), VENDOR, false),
            Some(Handler::Vendor)
        );
    }

    #[test]
    fn a_handler_the_user_chose_is_left_alone() {
        for signing_in in [true, false] {
            assert_eq!(
                plan(signing_in, Some("org.example.links"), VENDOR, true),
                None
            );
            assert_eq!(plan(signing_in, None, VENDOR, true), None);
        }
    }

    #[test]
    fn the_app_is_registered_for_the_scheme_under_the_identifier_handed_over() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"].as_str(), Some(HOST_BUNDLE_ID));

        let info = ::plist::Value::from_reader_xml(include_bytes!("../../Info.plist").as_slice())
            .unwrap()
            .into_dictionary()
            .unwrap();
        let schemes: Vec<&str> = info
            .get("CFBundleURLTypes")
            .and_then(::plist::Value::as_array)
            .unwrap()
            .iter()
            .filter_map(::plist::Value::as_dictionary)
            .filter_map(|kind| kind.get("CFBundleURLSchemes")?.as_array())
            .flatten()
            .filter_map(::plist::Value::as_string)
            .collect();
        assert!(schemes.contains(&SCHEME), "{schemes:?}");
    }
}
