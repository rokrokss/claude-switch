//! claude-switch's iCloud container, through Foundation: finding it, reading,
//! writing and deleting the files in it as iCloud expects (coordinated, so
//! that a transfer in progress isn't torn), and hearing when a file in it
//! changes. After OpenUsage's `ICloudUsageHistoryFileStore` (MIT,
//! <https://github.com/robinebers/openusage>).
//!
//! Only a build signed with the iCloud entitlements and the provisioning
//! profile that allows them (`scripts/icloud-config.sh`) has a container.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_foundation::{
    ns_string, NSArray, NSData, NSDataWritingOptions, NSError, NSFileCoordinator,
    NSFileCoordinatorReadingOptions, NSFileCoordinatorWritingOptions, NSFileManager,
    NSMetadataItemFSNameKey, NSMetadataQuery, NSMetadataQueryDidFinishGatheringNotification,
    NSMetadataQueryDidUpdateNotification, NSMetadataQueryUbiquitousDataScope, NSNotification,
    NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSPredicate, NSURL,
};

/// The container the provisioning profile allows.
const CONTAINER: &str = "iCloud.app.claude-switch";
/// Where in it each Mac's file goes: outside `Documents/`, so iCloud Drive
/// doesn't show it in Finder. Another format would go beside it, in `v2`.
const HISTORY: &str = "History/v1";

/// The folder of every Mac's file, made if need be. Blocking, and never on
/// the main thread: the first call of a launch sets the container up, which
/// can take a while.
pub fn history_dir() -> Result<PathBuf, String> {
    if !signed_for_icloud() {
        return Err("This build of claude-switch isn't signed for iCloud.".into());
    }
    let container = NSFileManager::defaultManager()
        .URLForUbiquityContainerIdentifier(Some(ns_string!(CONTAINER)))
        .and_then(|url| url.to_file_path())
        .ok_or("iCloud Drive isn't available. Check that this Mac is signed in to iCloud and iCloud Drive is on.")?;
    let dir = container.join(HISTORY);
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("couldn't make {} in iCloud: {err}", dir.display()))?;
    Ok(dir)
}

/// Whether the app carries the provisioning profile that allows its iCloud
/// entitlements: without it, there is no container to ask for.
fn signed_for_icloud() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("embedded.provisionprofile")))
        .is_some_and(|profile| profile.is_file())
}

/// The Macs' files in `dir`. One iCloud hasn't brought down yet is asked
/// for, and read once it arrives: the watch hears of it.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // What iCloud leaves in place of a file not downloaded.
        if let Some(pending) = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".icloud"))
        {
            start_download(&dir.join(pending));
        } else if name.ends_with(".json") && !name.starts_with('.') {
            start_download(&entry.path());
            files.push(entry.path());
        }
    }
    files.sort();
    files
}

fn start_download(path: &Path) {
    if let Some(url) = NSURL::from_file_path(path) {
        // A file already here is left as it is.
        let _ = NSFileManager::defaultManager().startDownloadingUbiquitousItemAtURL_error(&url);
    }
}

/// The file at `path`, read once nothing else is writing it.
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    let result = RefCell::new(None);
    coordinate(
        path,
        |coordinator, url, error, accessor| {
            coordinator.coordinateReadingItemAtURL_options_error_byAccessor(
                url,
                NSFileCoordinatorReadingOptions::empty(),
                Some(error),
                accessor,
            )
        },
        |path| *result.borrow_mut() = Some(std::fs::read(path).map_err(|err| err.to_string())),
    )?;
    result
        .into_inner()
        .unwrap_or_else(|| Err("iCloud didn't hand the file over".into()))
}

/// Replaces the file at `path` with `bytes`, in one step.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let data = NSData::with_bytes(bytes);
    let result = RefCell::new(None);
    coordinate(
        path,
        |coordinator, url, error, accessor| {
            coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
                url,
                NSFileCoordinatorWritingOptions::ForReplacing,
                Some(error),
                accessor,
            )
        },
        |path| {
            let written = NSURL::from_file_path(path)
                .ok_or_else(|| "not a file path".to_owned())
                .and_then(|url| {
                    data.writeToURL_options_error(&url, NSDataWritingOptions::Atomic)
                        .map_err(|err| describe(&err))
                });
            *result.borrow_mut() = Some(written);
        },
    )?;
    result
        .into_inner()
        .unwrap_or_else(|| Err("iCloud didn't take the file".into()))
}

/// Deletes the file at `path`, if there is one.
pub fn delete(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let result = RefCell::new(None);
    coordinate(
        path,
        |coordinator, url, error, accessor| {
            coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
                url,
                NSFileCoordinatorWritingOptions::ForDeleting,
                Some(error),
                accessor,
            )
        },
        |path| {
            let removed = match std::fs::remove_file(path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.to_string()),
                _ => Ok(()),
            };
            *result.borrow_mut() = Some(removed);
        },
    )?;
    result.into_inner().unwrap_or(Ok(()))
}

type Accessor<'a> = block2::DynBlock<dyn Fn(NonNull<NSURL>) + 'a>;

/// Runs `access` on the path a file coordinator hands out for `path`, through
/// `call`, one of the coordinator's methods.
fn coordinate(
    path: &Path,
    call: impl FnOnce(&NSFileCoordinator, &NSURL, &mut Option<Retained<NSError>>, &Accessor<'_>),
    access: impl Fn(&Path),
) -> Result<(), String> {
    let url = NSURL::from_file_path(path)
        .ok_or_else(|| format!("{} isn't a file path", path.display()))?;
    let accessor = RcBlock::new(|url: NonNull<NSURL>| {
        // SAFETY: the coordinator hands the accessor a valid URL, alive for
        // the call.
        if let Some(path) = unsafe { url.as_ref() }.to_file_path() {
            access(&path);
        }
    });
    let mut error = None;
    call(&NSFileCoordinator::new(), &url, &mut error, &accessor);
    match error {
        Some(error) => Err(describe(&error)),
        None => Ok(()),
    }
}

fn describe(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

/// The running watch: the query and what listens to it.
struct Watch {
    query: Retained<NSMetadataQuery>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

thread_local! {
    /// Main thread only, as the query lives on its run loop.
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

/// Calls `changed` whenever iCloud has news of the files in the container:
/// once when the query has found what is there, then on every change, this
/// Mac's own writes included. On the main thread: the query reports on the
/// run loop it starts on.
pub fn watch(changed: impl Fn() + 'static) {
    WATCH.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let query = NSMetadataQuery::new();
        // SAFETY: the scope is an NSString, the predicate takes the one key
        // its format names, and both arrays hold what the query reads.
        unsafe {
            let scope: &AnyObject = NSMetadataQueryUbiquitousDataScope;
            query.setSearchScopes(&NSArray::from_slice(&[scope]));
            let key: &AnyObject = NSMetadataItemFSNameKey;
            query.setPredicate(Some(&NSPredicate::predicateWithFormat_argumentArray(
                ns_string!("%K LIKE '*.json'"),
                Some(&NSArray::from_slice(&[key])),
            )));
        }
        let changed = Rc::new(changed);
        let center = NSNotificationCenter::defaultCenter();
        let queue = NSOperationQueue::mainQueue();
        let object: &AnyObject = &query;
        // SAFETY: Foundation's own constant names, set before any code runs.
        let names = unsafe {
            [
                NSMetadataQueryDidFinishGatheringNotification,
                NSMetadataQueryDidUpdateNotification,
            ]
        };
        let observers = names
            .into_iter()
            .map(|name| {
                let changed = Rc::clone(&changed);
                let block = RcBlock::new(move |_: NonNull<NSNotification>| changed());
                // SAFETY: the block runs on the main queue, the thread `changed`
                // was made on, and only while the query is kept in `WATCH`.
                unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(name),
                        Some(object),
                        Some(&queue),
                        &block,
                    )
                }
            })
            .collect();
        query.startQuery();
        *slot.borrow_mut() = Some(Watch { query, observers });
    });
}

/// Stops the watch [`watch`] started. On the main thread.
pub fn unwatch() {
    WATCH.with(|slot| {
        let Some(watch) = slot.borrow_mut().take() else {
            return;
        };
        watch.query.stopQuery();
        let center = NSNotificationCenter::defaultCenter();
        for observer in &watch.observers {
            // SAFETY: each observer was added to this center by `watch`.
            unsafe { center.removeObserver(observer.as_ref()) };
        }
    });
}
