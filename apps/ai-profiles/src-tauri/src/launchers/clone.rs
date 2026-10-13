//! Signed clones: a profile's own copy of the vendor app, signature untouched.
//!
//! A wrapper (see [`super::wrapper`]) gives a profile a Dock identity of its own
//! by re-signing a copy of the app, which costs the profile everything macOS ties
//! to the vendor's signature: Keychain items, permissions, passkeys, Cowork's
//! folders. A signed clone keeps all of that. It is an APFS copy-on-write clone
//! of the vendor bundle that is never modified inside: the only thing added is a
//! Finder custom icon, which is metadata on the bundle directory (an `Icon\r`
//! file and a FinderInfo flag) rather than part of the sealed contents, so the
//! CDHash and designated requirement stay the vendor's.
//!
//! That metadata is not invisible to everything. `codesign --verify --strict`
//! rejects the FinderInfo attribute as detritus, while plain and `--deep`
//! verification, the vendor's designated requirement and Gatekeeper all accept
//! the clone (measured on macOS 26.6.2 with Claude 2.31226.1; see
//! `docs/signed-clone-design.md`). Which runtime checks look at it is listed
//! there as items still to verify.
//!
//! The clone keeps the vendor's bundle identifier and Info.plist, so nothing of
//! ours can be recorded inside it. It is ours by where it is — under the
//! profile's own directory, in [`CLONE_DIR_NAME`] — and what we know about it is
//! kept beside it, in a [`Record`].
//!
//! Three rules govern it:
//!
//! - It is never changed while anything runs from it, the vendor's updater
//!   included. Changes wait for the next launch.
//! - It only ever moves forward. The vendor's updater installs into the clone
//!   itself, so a clone can be newer than the installed app, and making it again
//!   from that app then would undo the update (see [`plan`]).
//! - Launched without the profile's arguments — after an update relaunches it,
//!   or from a Dock tile — it runs on the vendor's default data. Such a process
//!   is a [`Stray`], and is put right before the profile opens.

use std::cmp::Ordering;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::launchers::{icons, wrapper};
use crate::paths::{clone_bundle_path, clone_dir, profiles_root, ResolvedGuiApp, CLONE_DIR_NAME};
use crate::profiles::Profile;

/// The record of a profile's clone, kept beside it in its [`CLONE_DIR_NAME`].
const RECORD_FILE: &str = "clone.json";

/// The file whose lock is held while a profile's clone is checked and changed.
/// The GUI, a launcher and the clone watch are separate processes that can all
/// open the same profile at once.
const LOCK_FILE: &str = ".lock";

/// The file Finder keeps a folder's custom icon in, inside the folder itself.
/// Its name really does end in a carriage return.
const CUSTOM_ICON_FILE: &str = "Icon\r";

const CODESIGN: &str = "/usr/bin/codesign";
const SPCTL: &str = "/usr/sbin/spctl";

/// How old a stray may be and still be quit without asking. A process this
/// young was started a moment ago, by an update or a Dock click, and nobody has
/// typed into it yet; an older one may hold something the user is writing.
pub const FRESH_STRAY: Duration = Duration::from_secs(20);

/// How long a stray is given to quit after being asked to.
pub const QUIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Distinguishes the staging paths of builds running at the same time.
static NEXT_STAGING: AtomicUsize = AtomicUsize::new(0);

/// What we know about a profile's clone that the clone itself cannot say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// `CFBundleVersion` of the vendor app the clone was made from. A clone
    /// whose own version differs has been updated by the vendor's updater since.
    #[serde(default)]
    pub cloned_from_version: Option<String>,
    /// The profile color the clone's custom icon was drawn in.
    #[serde(default)]
    pub color: Option<String>,
    /// The ai-profiles version that made the clone.
    #[serde(default)]
    pub built_by: Option<String>,
}

/// The record of profile `id`'s clone; empty when there is none or it can't be
/// read.
pub fn read_record(id: &str) -> Record {
    clone_dir(id)
        .ok()
        .and_then(|dir| fs::read(dir.join(RECORD_FILE)).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_record(id: &str, record: &Record) -> AppResult<()> {
    let dir = clone_dir(id)?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(RECORD_FILE), serde_json::to_vec_pretty(record)?)?;
    Ok(())
}

/// Everything [`plan`] decides on, gathered from disk and the process list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// There is a clone at the profile's clone path.
    pub exists: bool,
    /// Something runs from inside the clone: the app, a helper, or the vendor's
    /// updater installing into it.
    pub in_use: bool,
    /// `CFBundleVersion` of the installed vendor app.
    pub vendor_version: Option<String>,
    /// `CFBundleVersion` of the clone.
    pub clone_version: Option<String>,
    /// The clone carries a custom icon in the profile's current color.
    pub icon_current: bool,
}

/// What to do with a profile's clone before launching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Launch it as it is.
    UseAsIs,
    /// Make it afresh from the vendor app, replacing whatever is there.
    Build,
    /// Keep the bundle, which is newer than the vendor app, and draw its icon
    /// again.
    RestoreIcon,
}

/// Pure: what to do with a clone, given `facts`.
///
/// - Nothing there: make one.
/// - In use: leave it alone, whatever else is true.
/// - Older than the vendor app: make it again, which moves it forward.
/// - The vendor's version, but without its icon (an update in place drops it)
///   or in an old color: make it again. A fresh clone shares the vendor's disk
///   blocks again, and has never run, so its icon can be set without touching
///   an app macOS has already seen run.
/// - Newer than the vendor app (it updated itself): keep it — making it again
///   would undo the update, which the app would then download again on every
///   launch — and only draw its icon again if that is missing.
/// - Versions that can't be compared: launch what is there.
pub fn plan(facts: &Facts) -> Plan {
    if !facts.exists {
        return Plan::Build;
    }
    if facts.in_use {
        return Plan::UseAsIs;
    }
    let order = match (&facts.clone_version, &facts.vendor_version) {
        (Some(clone), Some(vendor)) => compare_versions(clone, vendor),
        _ => None,
    };
    match order {
        Some(Ordering::Less) => Plan::Build,
        Some(Ordering::Equal) if !facts.icon_current => Plan::Build,
        Some(Ordering::Greater) if !facts.icon_current => Plan::RestoreIcon,
        _ => Plan::UseAsIs,
    }
}

/// Pure: dotted numeric versions compared part by part, a missing part counting
/// as zero. `None` if either has a part that is not a number.
pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let parse = |version: &str| -> Option<Vec<u64>> {
        version
            .trim()
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect()
    };
    let (left, right) = (parse(left)?, parse(right)?);
    let length = left.len().max(right.len());
    let part = |parts: &[u64], index: usize| parts.get(index).copied().unwrap_or(0);
    Some(
        (0..length)
            .map(|index| part(&left, index).cmp(&part(&right, index)))
            .find(|order| order.is_ne())
            .unwrap_or(Ordering::Equal),
    )
}

/// The executable the app bundle at `bundle` runs (`CFBundleExecutable`),
/// read from its Info.plist every time rather than assumed. Nothing about the
/// vendor app is fixed in this code — its version, executable, signature and
/// icon are all read off the installed app when they are needed — so an update
/// of the vendor app needs no update of ai-profiles.
pub fn bundle_executable(bundle: &Path) -> Option<String> {
    ::plist::Value::from_file(bundle.join("Contents/Info.plist"))
        .ok()?
        .into_dictionary()?
        .get("CFBundleExecutable")?
        .as_string()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The executable `vendor` runs, as its Info.plist says now; the name its
/// [`crate::app_kind::AppSpec`] knows it by if that can't be read.
pub fn vendor_executable(vendor: &ResolvedGuiApp) -> String {
    bundle_executable(&vendor.bundle_path).unwrap_or_else(|| vendor.macos_exec.to_owned())
}

/// Bring `profile`'s clone into shape for a launch and return its path.
/// `version` is this app's own, recorded with a clone it makes.
///
/// Only [`plan`] decides what happens, and a clone in use is never touched.
pub fn ensure(profile: &Profile, vendor: &ResolvedGuiApp, version: &str) -> AppResult<PathBuf> {
    let spec = profile.app.spec();
    let dest = clone_bundle_path(&profile.id, &profile.name, spec)?;
    let dir = clone_dir(&profile.id)?;
    fs::create_dir_all(&dir)?;
    // Held until this returns; the OS lets it go if the process dies.
    let lock = fs::File::create(dir.join(LOCK_FILE))?;
    lock.lock()?;
    let processes = crate::launch::process_list()?;
    clear_leftovers(&profile.id, &processes);
    adopt_renamed(&profile.id, &dest, &processes);

    let record = read_record(&profile.id);
    let facts = Facts {
        exists: dest.exists(),
        in_use: in_use(&processes, &dest),
        vendor_version: wrapper::bundle_version(&vendor.bundle_path),
        clone_version: wrapper::bundle_version(&dest),
        icon_current: has_custom_icon(&dest)
            && record.color.as_deref() == Some(profile.color.as_str()),
    };
    match plan(&facts) {
        Plan::UseAsIs => {}
        Plan::Build => {
            ensure_replaceable(&dest)?;
            let icon = icons::render_icns(&profile.color, &vendor.bundle_path)?;
            if !build(vendor, &dest, &icon)? {
                return Ok(dest);
            }
            write_record(
                &profile.id,
                &Record {
                    cloned_from_version: facts.vendor_version.clone(),
                    color: Some(profile.color.clone()),
                    built_by: Some(version.to_owned()),
                },
            )?;
        }
        Plan::RestoreIcon => {
            // Best effort: the bundle has run, and macOS may refuse an app of
            // another developer the change (App Management). The profile still
            // opens, just without its color until the vendor app catches up and
            // the clone is made again.
            let icon = icons::render_icns(&profile.color, &vendor.bundle_path)?;
            match set_custom_icon(&dest, &icon) {
                Ok(()) => write_record(
                    &profile.id,
                    &Record {
                        color: Some(profile.color.clone()),
                        ..record
                    },
                )?,
                Err(err) => eprintln!(
                    "ai-profiles: left {} without its icon: {}",
                    dest.display(),
                    err.message()
                ),
            }
        }
    }
    Ok(dest)
}

/// Make a fresh clone of `vendor` at `dest`, with `icon` as its Finder icon.
///
/// It is assembled beside `dest` under a hidden name without `.app`, so
/// LaunchServices never takes it for an app, and verified before it takes
/// `dest`'s place. Whatever was at `dest` is moved aside first and put back if
/// the swap fails. Returns whether the new clone took `dest`'s place: it does
/// not when something started running from the old one in the meantime.
fn build(vendor: &ResolvedGuiApp, dest: &Path, icon: &[u8]) -> AppResult<bool> {
    let dir = dest
        .parent()
        .ok_or_else(|| AppError::Validation(format!("{} has no parent", dest.display())))?;
    fs::create_dir_all(dir)?;
    let stem = dest
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let token = format!(
        "{}-{}",
        std::process::id(),
        NEXT_STAGING.fetch_add(1, AtomicOrdering::Relaxed)
    );
    let staging = dir.join(format!(".{stem}.building-{token}"));
    let _staging = RemoveOnDrop(staging.clone());
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }

    wrapper::clone_bundle(&vendor.bundle_path, &staging, &vendor_executable(vendor))?;
    set_custom_icon(&staging, icon)?;
    verify(&staging, &vendor.bundle_path)?;

    // The clone at `dest` was not in use when this build was decided, seconds
    // ago. Look again: one that has started since is left as it is.
    if dest.exists() && in_use(&crate::launch::process_list()?, dest) {
        return Ok(false);
    }
    let parked = if dest.exists() {
        let parked = dir.join(format!(".{stem}.replaced-{token}"));
        fs::rename(dest, &parked)?;
        Some(parked)
    } else {
        None
    };
    if let Err(err) = fs::rename(&staging, dest) {
        if let Some(parked) = &parked {
            let _ = fs::rename(parked, dest);
        }
        return Err(err.into());
    }
    if let Some(parked) = parked {
        // A clone that has run may be protected from deletion by another
        // developer's app (App Management). It is out of the way either way, and
        // the next build tries again (see `clear_leftovers`).
        if let Err(err) = fs::remove_dir_all(&parked) {
            eprintln!(
                "ai-profiles: left the previous clone at {}: {err}",
                parked.display()
            );
        }
    }
    Ok(true)
}

/// Deletes the directory it names when dropped, if it is still there.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Fail unless `clone` carries `vendor`'s signature, unchanged:
///
/// - `codesign --verify --deep` accepts it. Not `--strict`: the Finder icon's
///   FinderInfo attribute on the bundle directory is what strict validation
///   rejects, and it is the one change a signed clone makes on purpose.
/// - It satisfies the vendor's designated requirement.
/// - Its CDHash is the vendor's.
/// - Gatekeeper accepts it.
pub fn verify(clone: &Path, vendor: &Path) -> AppResult<()> {
    run(
        Command::new(CODESIGN)
            .args(["--verify", "--deep"])
            .arg(clone),
        "verifying the signature of",
        clone,
    )?;
    let requirement = designated_requirement(vendor)?;
    run(
        Command::new(CODESIGN)
            .arg("--verify")
            .arg(format!("-R={requirement}"))
            .arg(clone),
        "checking the vendor's requirement against",
        clone,
    )?;
    let (clone_hash, vendor_hash) = (cdhash(clone)?, cdhash(vendor)?);
    if clone_hash != vendor_hash {
        return Err(AppError::Validation(format!(
            "{} has CDHash {clone_hash}, not the vendor's {vendor_hash}",
            clone.display()
        )));
    }
    run(
        Command::new(SPCTL)
            .args(["--assess", "--type", "execute"])
            .arg(clone),
        "assessing with Gatekeeper",
        clone,
    )
}

/// The designated requirement of the code at `path`, as text.
fn designated_requirement(path: &Path) -> AppResult<String> {
    let output = Command::new(CODESIGN)
        .args(["-d", "-r-"])
        .arg(path)
        .output()?;
    parse_designated_requirement(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        AppError::Validation(format!("{} has no designated requirement", path.display()))
    })
}

/// Pure: the requirement in `codesign -d -r-` output
/// (`designated => <requirement>`).
pub fn parse_designated_requirement(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("designated => "))
        .map(str::to_owned)
        .filter(|requirement| !requirement.is_empty())
}

/// The CDHash of the code at `path`.
fn cdhash(path: &Path) -> AppResult<String> {
    let output = Command::new(CODESIGN)
        .args(["-d", "-vvv"])
        .arg(path)
        .output()?;
    parse_cdhash(&String::from_utf8_lossy(&output.stderr))
        .ok_or_else(|| AppError::Validation(format!("{} has no CDHash", path.display())))
}

/// Pure: the CDHash in `codesign -d -vvv` output (`CDHash=<hex>`).
pub fn parse_cdhash(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .find_map(|line| line.trim().strip_prefix("CDHash="))
        .map(str::to_owned)
}

/// Run `command`, failing with its output unless it succeeds. `doing` and
/// `target` name it in the error.
fn run(command: &mut Command, doing: &str, target: &Path) -> AppResult<()> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "{doing} {} failed ({}): {}",
        target.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// Whether `bundle` carries a Finder custom icon.
pub fn has_custom_icon(bundle: &Path) -> bool {
    bundle.join(CUSTOM_ICON_FILE).is_file()
}

/// Give `target` the `.icns` image `icns` as its Finder custom icon. The Dock
/// shows a running app's bundle icon, and a custom icon counts as one.
#[cfg(target_os = "macos")]
fn set_custom_icon(target: &Path, icns: &[u8]) -> AppResult<()> {
    use objc2::rc::autoreleasepool;
    use objc2::AnyThread;
    use objc2_app_kit::{NSImage, NSWorkspace, NSWorkspaceIconCreationOptions};
    use objc2_foundation::{NSData, NSString};

    let path = target
        .to_str()
        .ok_or_else(|| AppError::Validation(format!("{} is not valid UTF-8", target.display())))?;
    // AppKit hands back autoreleased objects, and a worker thread has no pool of
    // its own to drain them into.
    let applied = autoreleasepool(|_| {
        let data = NSData::with_bytes(icns);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            return false;
        };
        NSWorkspace::sharedWorkspace().setIcon_forFile_options(
            Some(&image),
            &NSString::from_str(path),
            NSWorkspaceIconCreationOptions::empty(),
        )
    });
    if applied && has_custom_icon(target) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "macOS did not set the icon of {}",
            target.display()
        )))
    }
}

#[cfg(not(target_os = "macos"))]
fn set_custom_icon(target: &Path, _icns: &[u8]) -> AppResult<()> {
    Err(AppError::Validation(format!(
        "custom icons need macOS ({})",
        target.display()
    )))
}

/// Pure: whether any process in `ps_output` runs from inside `bundle`.
pub fn in_use(ps_output: &str, bundle: &Path) -> bool {
    crate::launch::runs_from(ps_output, bundle)
}

/// Fail if anything in `bundle` belongs to someone other than whoever owns
/// the clone directory: the vendor's updater has been seen to install as root,
/// and such a bundle can be neither replaced nor given an icon. We never
/// escalate; the user is told how to clear it.
fn ensure_replaceable(bundle: &Path) -> AppResult<()> {
    let Some(owner) = bundle
        .parent()
        .and_then(|dir| fs::metadata(dir).ok())
        .map(|metadata| metadata.uid())
    else {
        return Ok(());
    };
    if bundle.exists() && owned_by_other(bundle, owner) {
        return Err(AppError::Validation(format!(
            "{} has files that belong to another user (the vendor's updater may have run as \
             root), so it can't be replaced. Quit the profile and remove it with: sudo rm -rf '{}'",
            bundle.display(),
            bundle.display()
        )));
    }
    Ok(())
}

/// Whether anything under `path` (itself included, links not followed) belongs
/// to a user other than `owner`.
fn owned_by_other(path: &Path, owner: u32) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.uid() != owner {
        return true;
    }
    if !metadata.is_dir() {
        return false;
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| owned_by_other(&entry.path(), owner))
        })
        .unwrap_or(false)
}

/// Remove what a build left behind in profile `id`'s clone directory — a
/// staging bundle or a replaced clone that could not be deleted then — unless
/// something still runs from it.
fn clear_leftovers(id: &str, processes: &str) {
    let Ok(dir) = clone_dir(id) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let leftover =
            name.starts_with('.') && (name.contains(".building-") || name.contains(".replaced-"));
        if leftover && !in_use(processes, &entry.path()) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The clones in profile `id`'s clone directory: every non-hidden `.app`.
fn bundles_in(id: &str) -> Vec<PathBuf> {
    let Ok(dir) = clone_dir(id) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            !name.starts_with('.') && name.ends_with(".app") && path.is_dir()
        })
        .collect()
}

/// When the profile was renamed since its clone was made, the clone sits under
/// the old name. Move it to `dest` rather than make a new one, which could undo
/// an update the old one installed. Left alone while in use; a failed move
/// leaves `dest` empty, and the clone is made again there.
fn adopt_renamed(id: &str, dest: &Path, processes: &str) {
    if dest.exists() {
        return;
    }
    if let Some(previous) = bundles_in(id)
        .into_iter()
        .find(|bundle| !in_use(processes, bundle))
    {
        let _ = fs::rename(previous, dest);
    }
}

/// Remove profile `id`'s clone and its record, except a clone something still
/// runs from, which is left for a later call. Best effort: a clone is a cache of
/// the vendor app and nothing else, so failing to remove one is only reported.
pub fn remove_unless_in_use(id: &str) {
    let Ok(dir) = clone_dir(id) else {
        return;
    };
    if !dir.exists() {
        return;
    }
    let processes = crate::launch::process_list().unwrap_or_default();
    clear_leftovers(id, &processes);
    let mut kept = false;
    let _ = fs::remove_file(dir.join(LOCK_FILE));
    for bundle in bundles_in(id) {
        if in_use(&processes, &bundle) {
            kept = true;
            continue;
        }
        if let Err(err) = fs::remove_dir_all(&bundle) {
            kept = true;
            eprintln!(
                "ai-profiles: could not remove the clone at {}: {err}",
                bundle.display()
            );
        }
    }
    if !kept {
        let _ = fs::remove_dir_all(&dir);
    }
}

/// The main process of a profile's clone, running without the profile's
/// `--user-data-dir`.
///
/// macOS starts an app with no arguments and a launchd environment when an
/// update relaunches it, when its Dock tile or Recents entry is clicked, and
/// when windows are reopened at login. A clone started that way runs on the
/// vendor's default data and config — the user's default account, under the
/// profile's icon — or, when the default instance is already up, hands itself
/// over to that one and quits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stray {
    pub pid: i32,
    /// The profile whose clone it runs from.
    pub profile_id: String,
    /// How long it has been running, if `ps` said.
    pub age: Option<Duration>,
}

impl Stray {
    /// Whether it may be quit without asking: see [`FRESH_STRAY`]. A stray of
    /// unknown age is not.
    pub fn is_fresh(&self) -> bool {
        self.age.is_some_and(|age| age <= FRESH_STRAY)
    }
}

/// Pure: the profile whose clone `command` is the main process of, when that
/// runs without `--user-data-dir`. `profiles_root` is
/// [`crate::paths::profiles_root`], `exec` the vendor's executable name.
///
/// The main process runs `<root>/<id>/app.noindex/<name>.app/Contents/MacOS/<exec>`.
/// Helpers live deeper, in the bundle's `Contents/Frameworks`, so a bundle name
/// with a `/` in it means a helper and is rejected.
pub fn stray_profile(command: &str, profiles_root: &str, exec: &str) -> Option<String> {
    let rest = command.strip_prefix(profiles_root)?.strip_prefix('/')?;
    let (id, rest) = rest.split_once('/')?;
    let rest = rest.strip_prefix(CLONE_DIR_NAME)?.strip_prefix('/')?;
    let marker = format!(".app/Contents/MacOS/{exec}");
    let at = rest.find(&marker)?;
    if id.is_empty() || rest[..at].contains('/') {
        return None;
    }
    let args = &rest[at + marker.len()..];
    if !(args.is_empty() || args.starts_with(' ')) || args.contains("--user-data-dir=") {
        return None;
    }
    Some(id.to_owned())
}

/// Pure: the strays in `ps -ax -o pid=,etime=,command=` output.
pub fn strays(ps_output: &str, profiles_root: &str, exec: &str) -> Vec<Stray> {
    ps_output
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim_start().splitn(2, char::is_whitespace);
            let pid = fields.next()?.parse::<i32>().ok()?;
            let rest = fields.next()?.trim_start();
            let (elapsed, command) = rest.split_once(char::is_whitespace)?;
            let profile_id = stray_profile(command.trim(), profiles_root, exec)?;
            Some(Stray {
                pid,
                profile_id,
                age: parse_elapsed(elapsed),
            })
        })
        .collect()
}

/// Pure: the pids of the strays in `ps -ax -o pid=,command=` output.
pub fn stray_pids(ps_output: &str, profiles_root: &str, exec: &str) -> Vec<i32> {
    ps_output
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(char::is_whitespace)?;
            stray_profile(command.trim(), profiles_root, exec)?;
            pid.parse::<i32>().ok()
        })
        .collect()
}

/// Pure: `ps`'s elapsed time, `[[dd-]hh:]mm:ss`, as a duration.
pub fn parse_elapsed(text: &str) -> Option<Duration> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let parts: Vec<u64> = clock
        .split(':')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<_>>()?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    Some(Duration::from_secs(
        ((days * 24 + hours) * 60 + minutes) * 60 + seconds,
    ))
}

/// Every running process, `pid elapsed command` each.
pub fn process_list_with_age() -> AppResult<String> {
    let output = Command::new("ps")
        .args(["-ax", "-o", "pid=,etime=,command="])
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The strays running now, of any profile.
pub fn running_strays(exec: &str) -> AppResult<Vec<Stray>> {
    let root = profiles_root()?.display().to_string();
    Ok(strays(&process_list_with_age()?, &root, exec))
}

/// Ask the process `pid` to quit the way the Quit menu item does (SIGTERM),
/// and wait up to [`QUIT_TIMEOUT`] for it to be gone. It is never killed.
pub fn quit(pid: i32) -> AppResult<()> {
    let alive = || {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    };
    Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()?;
    let deadline = Instant::now() + QUIT_TIMEOUT;
    while alive() {
        if Instant::now() >= deadline {
            return Err(AppError::Validation(format!(
                "process {pid} didn't quit within {} seconds",
                QUIT_TIMEOUT.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Quit `profile_id`'s strays before the profile opens: the fresh ones always,
/// the others only with `quit_old`. Returns the first stray left running, which
/// stands in the way of opening the profile.
pub fn settle_strays(profile_id: &str, exec: &str, quit_old: bool) -> AppResult<Option<Stray>> {
    for stray in running_strays(exec)? {
        if stray.profile_id != profile_id {
            continue;
        }
        if quit_old || stray.is_fresh() {
            quit(stray.pid)?;
        } else {
            return Ok(Some(stray));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/Users/u/Library/Application Support/ai-profiles/profiles";
    const ID: &str = "11111111-2222-3333-4444-555555555555";

    fn clone_exec() -> String {
        format!("{ROOT}/{ID}/app.noindex/Claude (Work).app/Contents/MacOS/Claude")
    }

    fn facts() -> Facts {
        Facts {
            exists: true,
            in_use: false,
            vendor_version: Some("2.31226.1".into()),
            clone_version: Some("2.31226.1".into()),
            icon_current: true,
        }
    }

    #[test]
    fn a_missing_clone_is_built() {
        assert_eq!(
            plan(&Facts {
                exists: false,
                ..facts()
            }),
            Plan::Build
        );
    }

    #[test]
    fn a_clone_in_use_is_never_touched() {
        for clone_version in ["1.0.0", "2.31226.1", "9.0.0"] {
            for icon_current in [false, true] {
                let facts = Facts {
                    in_use: true,
                    clone_version: Some(clone_version.into()),
                    icon_current,
                    ..facts()
                };
                assert_eq!(plan(&facts), Plan::UseAsIs, "{facts:?}");
            }
        }
    }

    #[test]
    fn a_clone_only_moves_forward() {
        let older = Facts {
            clone_version: Some("2.31000.0".into()),
            ..facts()
        };
        assert_eq!(plan(&older), Plan::Build);

        let newer = Facts {
            clone_version: Some("2.31300.0".into()),
            ..facts()
        };
        assert_eq!(plan(&newer), Plan::UseAsIs, "never made again from older");
        assert_eq!(
            plan(&Facts {
                icon_current: false,
                ..newer
            }),
            Plan::RestoreIcon
        );
    }

    #[test]
    fn a_current_clone_without_its_icon_is_made_again() {
        assert_eq!(plan(&facts()), Plan::UseAsIs);
        assert_eq!(
            plan(&Facts {
                icon_current: false,
                ..facts()
            }),
            Plan::Build
        );
    }

    #[test]
    fn versions_that_cannot_be_compared_launch_what_is_there() {
        let facts = Facts {
            clone_version: Some("2.x".into()),
            icon_current: false,
            ..facts()
        };
        assert_eq!(plan(&facts), Plan::UseAsIs);
        assert_eq!(
            plan(&Facts {
                clone_version: None,
                ..facts
            }),
            Plan::UseAsIs
        );
    }

    #[test]
    fn versions_compare_numerically_part_by_part() {
        assert_eq!(compare_versions("2.9", "2.10"), Some(Ordering::Less));
        assert_eq!(
            compare_versions("2.31226.1", "2.31226.1"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_versions("2.31226", "2.31226.0"),
            Some(Ordering::Equal)
        );
        assert_eq!(compare_versions("3", "2.99999.9"), Some(Ordering::Greater));
        assert_eq!(compare_versions("2.1-beta", "2.1"), None);
    }

    #[test]
    fn a_clone_main_process_without_its_data_dir_is_a_stray() {
        assert_eq!(
            stray_profile(&clone_exec(), ROOT, "Claude").as_deref(),
            Some(ID)
        );
        let with_flag = format!("{} --user-data-dir=/x/gui-data", clone_exec());
        assert_eq!(stray_profile(&with_flag, ROOT, "Claude"), None);
        let other_args = format!("{} --some-flag", clone_exec());
        assert_eq!(
            stray_profile(&other_args, ROOT, "Claude").as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn helpers_and_other_bundles_are_not_strays() {
        let helper = format!(
            "{ROOT}/{ID}/app.noindex/Claude (Work).app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer) --type=renderer"
        );
        let ship_it = format!(
            "{ROOT}/{ID}/app.noindex/Claude (Work).app/Contents/Frameworks/Squirrel.framework/Resources/ShipIt com.anthropic.claudefordesktop.ShipIt"
        );
        let stock = "/Applications/Claude.app/Contents/MacOS/Claude".to_owned();
        let elsewhere = format!("{ROOT}/{ID}/other/Claude.app/Contents/MacOS/Claude");
        for command in [helper, ship_it, stock, elsewhere] {
            assert_eq!(stray_profile(&command, ROOT, "Claude"), None, "{command}");
        }
    }

    #[test]
    fn strays_carry_their_age_from_ps() {
        let ps = format!(
            "  101       00:05 {exec}\n  102    01:02:03 {exec} --user-data-dir=/d\n  103 2-00:00:10 {exec}\n  104       00:01 /usr/bin/true\n",
            exec = clone_exec()
        );
        let found = strays(&ps, ROOT, "Claude");
        assert_eq!(
            found,
            vec![
                Stray {
                    pid: 101,
                    profile_id: ID.into(),
                    age: Some(Duration::from_secs(5)),
                },
                Stray {
                    pid: 103,
                    profile_id: ID.into(),
                    age: Some(Duration::from_secs(2 * 86_400 + 10)),
                },
            ]
        );
        assert!(found[0].is_fresh());
        assert!(!found[1].is_fresh());
        let unknown = Stray {
            age: None,
            ..found[0].clone()
        };
        assert!(!unknown.is_fresh(), "unknown age is not fresh");
    }

    #[test]
    fn stray_pids_read_the_plain_process_list() {
        let ps = format!(
            "  101 {exec}\n  102 {exec} --user-data-dir=/d\n",
            exec = clone_exec()
        );
        assert_eq!(stray_pids(&ps, ROOT, "Claude"), vec![101]);
    }

    #[test]
    fn elapsed_times_parse_in_every_ps_form() {
        assert_eq!(parse_elapsed("00:07"), Some(Duration::from_secs(7)));
        assert_eq!(parse_elapsed("12:11:51"), Some(Duration::from_secs(43_911)));
        assert_eq!(
            parse_elapsed("3-01:00:00"),
            Some(Duration::from_secs(3 * 86_400 + 3_600))
        );
        assert_eq!(parse_elapsed("soon"), None);
        assert_eq!(parse_elapsed("1:2:3:4"), None);
    }

    #[test]
    fn requirement_and_cdhash_are_read_from_codesign_output() {
        let stdout = "designated => identifier \"com.anthropic.claudefordesktop\" and anchor apple generic\n";
        assert_eq!(
            parse_designated_requirement(stdout).as_deref(),
            Some("identifier \"com.anthropic.claudefordesktop\" and anchor apple generic")
        );
        assert_eq!(parse_designated_requirement("Executable=/x\n"), None);
        let stderr = "Executable=/x\nCDHash=8da88def\nSignature size=9000\n";
        assert_eq!(parse_cdhash(stderr).as_deref(), Some("8da88def"));
    }

    #[test]
    fn a_record_round_trips_and_an_unreadable_one_is_empty() {
        let _guard = crate::test_support::APP_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let id = "clone-record-test";
        let record = Record {
            cloned_from_version: Some("2.31226.1".into()),
            color: Some("#7C3AED".into()),
            built_by: Some("1.4.0".into()),
        };
        write_record(id, &record).unwrap();
        assert_eq!(read_record(id), record);
        let raw = fs::read_to_string(clone_dir(id).unwrap().join(RECORD_FILE)).unwrap();
        assert!(raw.contains("clonedFromVersion"), "{raw}");

        fs::write(clone_dir(id).unwrap().join(RECORD_FILE), b"not json").unwrap();
        assert_eq!(read_record(id), Record::default());
        let _ = fs::remove_dir_all(crate::paths::profile_dir(id).unwrap());
    }

    #[test]
    fn leftovers_are_cleared_and_a_renamed_clone_is_adopted() {
        let _guard = crate::test_support::APP_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let id = "clone-leftover-test";
        let dir = clone_dir(id).unwrap();
        fs::create_dir_all(dir.join(".Claude (Old).building-1-0/Contents")).unwrap();
        fs::create_dir_all(dir.join(".Claude (Old).replaced-1-1/Contents")).unwrap();
        fs::create_dir_all(dir.join("Claude (Old).app/Contents")).unwrap();
        fs::write(dir.join("Claude (Old).app/marker"), b"kept").unwrap();

        clear_leftovers(id, "");
        let dest = dir.join("Claude (New).app");
        adopt_renamed(id, &dest, "");

        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["Claude (New).app".to_owned()]);
        assert_eq!(fs::read(dest.join("marker")).unwrap(), b"kept");

        remove_unless_in_use(id);
        assert!(!dir.exists(), "nothing runs from it, so it all goes");
        let _ = fs::remove_dir_all(crate::paths::profile_dir(id).unwrap());
    }

    #[test]
    fn a_clone_in_use_is_neither_adopted_nor_removed() {
        let _guard = crate::test_support::APP_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let id = "clone-in-use-test";
        let dir = clone_dir(id).unwrap();
        let old = dir.join("Claude (Old).app");
        fs::create_dir_all(old.join("Contents/MacOS")).unwrap();
        let ps = format!("  7 {}/Contents/MacOS/Claude\n", old.display());

        adopt_renamed(id, &dir.join("Claude (New).app"), &ps);
        assert!(old.exists(), "a clone in use keeps its name");
        assert!(in_use(&ps, &old));
        let _ = fs::remove_dir_all(crate::paths::profile_dir(id).unwrap());
    }

    /// Opt-in: clones the installed Claude into the test data directory (never
    /// launching it), gives it an icon and checks that it verifies as the
    /// vendor's own code. Gated behind AI_PROFILES_E2E=1 like the other tests
    /// that need the real system.
    #[test]
    fn a_built_clone_keeps_the_vendor_signature_and_carries_its_icon() {
        if std::env::var("AI_PROFILES_E2E").is_err() {
            eprintln!("skipping; set AI_PROFILES_E2E=1 to run");
            return;
        }
        let _guard = crate::test_support::APP_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(vendor) = crate::paths::resolve_gui_app(&crate::app_kind::CLAUDE) else {
            eprintln!("Claude not installed; skipping");
            return;
        };
        let profile = Profile {
            id: "clone-e2e-test".into(),
            app: crate::app_kind::AppKind::Claude,
            name: "CloneTest".into(),
            slug: "clonetest".into(),
            color: "#2563EB".into(),
            created_at: "2026-10-14T00:00:00Z".into(),
            surfaces: crate::profiles::Surfaces {
                gui: true,
                cli: false,
            },
            distinct_dock_icon: false,
            signed_clone: true,
            last_used_at: None,
        };

        let clone = match ensure(&profile, &vendor, "0.1.0") {
            Ok(clone) => clone,
            Err(err) if err.message().contains("different volume") => {
                eprintln!("test data dir is on another volume; skipping");
                return;
            }
            Err(err) => panic!("{}", err.message()),
        };

        assert!(has_custom_icon(&clone));
        verify(&clone, &vendor.bundle_path).unwrap();
        let record = read_record(&profile.id);
        assert_eq!(record.color.as_deref(), Some("#2563EB"));
        assert_eq!(
            record.cloned_from_version,
            wrapper::bundle_version(&vendor.bundle_path)
        );
        // Ensuring again changes nothing.
        let inode = fs::metadata(&clone).unwrap().ino();
        ensure(&profile, &vendor, "0.1.0").unwrap();
        assert_eq!(fs::metadata(&clone).unwrap().ino(), inode);

        remove_unless_in_use(&profile.id);
        assert!(!clone.exists());
        let _ = fs::remove_dir_all(crate::paths::profile_dir(&profile.id).unwrap());
    }
}
