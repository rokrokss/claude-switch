//! iCloud sync of the usage history. Each Mac signed in to one iCloud
//! account keeps its own history in claude-switch's iCloud container, one
//! file per Mac ([`document`]), and reads the others', so a profile's spend
//! counts what its account spent on all of them. Off unless
//! `settings.sync.icloud` is on; the README's FAQ says what the files hold.
//!
//! As in OpenUsage's `ICloudUsageSyncStore` (MIT,
//! <https://github.com/robinebers/openusage>): a save waits 3 s for the next
//! to replace it, a change to any file reloads them all, a file that doesn't
//! read is named in Settings and left out, and turning sync off deletes this
//! Mac's file and goes back to this Mac's figures alone.

#[cfg(target_os = "macos")]
mod device;
pub mod document;
#[cfg(target_os = "macos")]
mod icloud;

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::accounts::{self, AccountStatus};
use crate::usage::spend::Histories;
use crate::{logging, tray};
use document::HistoryDocument;

#[cfg(target_os = "macos")]
use device::Device;

/// How long a save waits for the next.
const SAVE_DELAY: Duration = Duration::from_secs(3);
/// The event Settings hears the status by.
const EVENT: &str = "icloud-sync";

#[derive(Default)]
struct Engine {
    on: bool,
    /// What the last read of the local logs found.
    local: Option<Arc<Histories>>,
    /// Every Mac's file as last read, this Mac's own included.
    documents: Vec<HistoryDocument>,
    /// The files that didn't read, and why.
    unreadable: Vec<String>,
    /// What went wrong last, until something goes right.
    problem: Option<String>,
    /// Reads and writes under way.
    busy: usize,
    /// Counts the saves asked for: one runs only if no other came after it.
    saves: u64,
}

static ENGINE: LazyLock<Mutex<Engine>> = LazyLock::new(Mutex::default);
/// Reads one at a time, so an older one can't land after a newer.
static RELOADS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(target_os = "macos")]
static THIS_MAC: OnceLock<Device> = OnceLock::new();

fn engine() -> MutexGuard<'static, Engine> {
    ENGINE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Settings' view of sync.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub on: bool,
    pub syncing: bool,
    /// Why sync isn't working, while it isn't.
    pub problem: Option<String>,
    pub unreadable: Vec<String>,
    /// Every Mac with a file, this one first, then the most recent.
    pub devices: Vec<SyncDevice>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDevice {
    pub id: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub this_mac: bool,
    /// This Mac's profiles signed in to an account that Mac has history of.
    pub profiles: Vec<String>,
}

/// Called once, from setup.
pub fn setup(app: &AppHandle) {
    let on = crate::app_state::load().is_ok_and(|state| state.settings.sync.icloud);
    apply(app, on);
}

/// Turns sync on or off, as `settings.sync.icloud` now says.
pub fn apply(app: &AppHandle, on: bool) {
    {
        let mut engine = engine();
        if engine.on == on {
            return;
        }
        engine.on = on;
        engine.documents.clear();
        engine.unreadable.clear();
        engine.problem = None;
    }
    let handle = app.clone();
    if on {
        let watcher = app.clone();
        let _ = app.run_on_main_thread(move || watch(watcher));
        tauri::async_runtime::spawn(async move {
            reload(&handle).await;
            save(&handle).await;
        });
    } else {
        let _ = app.run_on_main_thread(unwatch);
        tauri::async_runtime::spawn(async move {
            remove_own_file(&handle).await;
            rerender(&handle).await;
        });
    }
    emit(app);
}

/// A read of the local logs finished with `histories`: saves this Mac's
/// file, [`SAVE_DELAY`] later.
pub fn local_changed(app: &AppHandle, histories: Arc<Histories>) {
    let ticket = {
        let mut engine = engine();
        engine.local = Some(histories);
        if !engine.on {
            return;
        }
        engine.saves += 1;
        engine.saves
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SAVE_DELAY).await;
        if engine().saves == ticket {
            save(&app).await;
        }
    });
}

/// `histories` with what the other Macs' files hold of each home's account
/// added, for `homes` in the panel's order. As it is while sync is off.
pub fn combined(histories: &Histories, homes: &[&str], today: NaiveDate) -> Histories {
    let peers: Vec<HistoryDocument> = {
        let engine = engine();
        let Some(own) = own_id().filter(|_| engine.on) else {
            return histories.clone();
        };
        engine
            .documents
            .iter()
            .filter(|document| document.device_id != own)
            .cloned()
            .collect()
    };
    if peers.is_empty() {
        return histories.clone();
    }
    document::combined(histories, &account_ids(homes), &peers, today)
}

/// Each of `homes` with the id its account goes by in the files, if known.
fn account_ids(homes: &[&str]) -> Vec<(String, Option<String>)> {
    homes
        .iter()
        .map(|home| {
            let key = match accounts::read(home) {
                Ok(AccountStatus::SignedIn { account }) => account.key,
                _ => None,
            };
            ((*home).to_owned(), key.as_deref().map(document::account_id))
        })
        .collect()
}

/// The status, as Settings shows it.
#[tauri::command]
pub async fn icloud_sync() -> SyncStatus {
    status()
}

fn status() -> SyncStatus {
    let (on, syncing, problem, unreadable, mut documents) = {
        let engine = engine();
        (
            engine.on,
            engine.busy > 0,
            engine.problem.clone(),
            engine.unreadable.clone(),
            engine.documents.clone(),
        )
    };
    let own = own_id();
    documents.sort_by_key(|document| Some(&document.device_id) != own.as_ref());
    let targets = tray::targets().unwrap_or_default();
    let ids: Vec<&str> = targets.iter().map(|target| target.id.as_str()).collect();
    let accounts = account_ids(&ids);
    let devices = documents
        .into_iter()
        .map(|document| SyncDevice {
            profiles: targets
                .iter()
                .zip(&accounts)
                .filter(|(_, (_, account))| {
                    account
                        .as_ref()
                        .is_some_and(|account| document.accounts.contains_key(account))
                })
                .map(|(target, _)| target.name.clone())
                .collect(),
            this_mac: Some(&document.device_id) == own.as_ref(),
            id: document.device_id,
            name: document.device_name,
            updated_at: document.updated_at,
        })
        .collect();
    SyncStatus {
        on,
        syncing,
        problem,
        unreadable,
        devices,
    }
}

fn emit(app: &AppHandle) {
    let _ = app.emit(EVENT, status());
}

fn set_busy(app: &AppHandle, busy: bool) {
    {
        let mut engine = engine();
        engine.busy = if busy {
            engine.busy + 1
        } else {
            engine.busy.saturating_sub(1)
        };
    }
    emit(app);
}

fn report(app: &AppHandle, what: &str, problem: String) {
    logging::warn("icloud", &format!("{what}: {problem}"));
    engine().problem = Some(problem);
    emit(app);
}

/// Reads every Mac's file again, and shows what changed.
async fn reload(app: &AppHandle) {
    let _one_at_a_time = RELOADS.lock().await;
    if !engine().on {
        return;
    }
    set_busy(app, true);
    let read = tauri::async_runtime::spawn_blocking(read_all).await;
    set_busy(app, false);
    let (documents, unreadable) = match read {
        Ok(Ok(read)) => read,
        Ok(Err(problem)) => return report(app, "could not read the Macs' files", problem),
        Err(err) => return report(app, "could not read the Macs' files", err.to_string()),
    };
    let changed = {
        let mut engine = engine();
        // Turned off while reading: what was read is no longer wanted.
        if !engine.on {
            return;
        }
        let changed = engine.documents != documents;
        engine.documents = documents;
        engine.problem = (!unreadable.is_empty())
            .then(|| "Some Macs' usage couldn't be read; they are left out.".to_owned());
        engine.unreadable = unreadable;
        changed
    };
    emit(app);
    if changed {
        rerender(app).await;
    }
}

/// Writes this Mac's file from the last read of the local logs, if there
/// has been one, then reads every file again.
async fn save(app: &AppHandle) {
    let Some(local) = ({
        let engine = engine();
        engine.local.clone().filter(|_| engine.on)
    }) else {
        return;
    };
    set_busy(app, true);
    let homes: Vec<String> = tray::targets()
        .unwrap_or_default()
        .into_iter()
        .map(|target| target.id)
        .collect();
    let written = tauri::async_runtime::spawn_blocking(move || {
        let ids: Vec<&str> = homes.iter().map(String::as_str).collect();
        write_own_file(&local, &account_ids(&ids))
    })
    .await;
    set_busy(app, false);
    match written {
        Ok(Ok(path)) => {
            // Turned off while writing: this Mac must not stay among the others.
            if !engine().on {
                let _ = tauri::async_runtime::spawn_blocking(move || delete(&path)).await;
                return;
            }
            engine().problem = None;
            reload(app).await;
        }
        Ok(Err(problem)) => report(app, "could not save this Mac's usage", problem),
        Err(err) => report(app, "could not save this Mac's usage", err.to_string()),
    }
}

/// Deletes this Mac's file, as sync goes off. A launch that never reached
/// the files wrote none.
async fn remove_own_file(app: &AppHandle) {
    if own_id().is_none() {
        return;
    }
    let removed = tauri::async_runtime::spawn_blocking(|| delete(&own_file()?)).await;
    match removed {
        Ok(Ok(())) => {}
        Ok(Err(problem)) => report(app, "could not delete this Mac's file", problem),
        Err(err) => report(app, "could not delete this Mac's file", err.to_string()),
    }
}

/// Shows the panel's figures again, with what the files now add.
async fn rerender(app: &AppHandle) {
    let local = engine().local.clone();
    if let Some(local) = local {
        tray::show_history(app, &local).await;
    }
}

#[cfg(target_os = "macos")]
fn this_mac() -> Result<Device, String> {
    if let Some(device) = THIS_MAC.get() {
        return Ok(device.clone());
    }
    let device = device::this_mac()?;
    Ok(THIS_MAC.get_or_init(|| device).clone())
}

/// This Mac's id, once known: not before a read has reached the files.
#[cfg(target_os = "macos")]
fn own_id() -> Option<String> {
    THIS_MAC.get().map(|device| device.id.clone())
}

#[cfg(target_os = "macos")]
fn own_file() -> Result<PathBuf, String> {
    let device = this_mac()?;
    Ok(icloud::history_dir()?.join(format!("{}.json", device.id)))
}

#[cfg(target_os = "macos")]
fn read_all() -> Result<(Vec<HistoryDocument>, Vec<String>), String> {
    let dir = icloud::history_dir()?;
    // Known before any file is used, so this Mac's own is told from the rest.
    this_mac()?;
    let mut documents = Vec::new();
    let mut unreadable = Vec::new();
    for path in icloud::files(&dir) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match icloud::read(&path).and_then(|bytes| document::parse(&bytes)) {
            Ok(document) => documents.push(document),
            Err(why) => {
                logging::warn("icloud", &format!("left out {name}: {why}"));
                unreadable.push(format!("{name}: {why}"));
            }
        }
    }
    Ok((document::newest_by_device(documents), unreadable))
}

#[cfg(target_os = "macos")]
fn write_own_file(
    local: &Histories,
    homes: &[(String, Option<String>)],
) -> Result<PathBuf, String> {
    let device = this_mac()?;
    let document = document::local(&device.id, &device.name, local, homes, Utc::now());
    let bytes = serde_json::to_vec_pretty(&document).map_err(|err| err.to_string())?;
    let path = own_file()?;
    icloud::write(&path, &bytes)?;
    Ok(path)
}

#[cfg(target_os = "macos")]
fn delete(path: &std::path::Path) -> Result<(), String> {
    icloud::delete(path)
}

#[cfg(target_os = "macos")]
fn watch(app: AppHandle) {
    icloud::watch(move || {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { reload(&app).await });
    });
}

#[cfg(target_os = "macos")]
fn unwatch() {
    icloud::unwatch();
}

// iCloud is a macOS service: elsewhere sync stays off with a problem.

#[cfg(not(target_os = "macos"))]
const ELSEWHERE: &str = "iCloud sync needs macOS.";

#[cfg(not(target_os = "macos"))]
fn own_id() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
fn own_file() -> Result<PathBuf, String> {
    Err(ELSEWHERE.into())
}

#[cfg(not(target_os = "macos"))]
fn read_all() -> Result<(Vec<HistoryDocument>, Vec<String>), String> {
    Err(ELSEWHERE.into())
}

#[cfg(not(target_os = "macos"))]
fn write_own_file(_: &Histories, _: &[(String, Option<String>)]) -> Result<PathBuf, String> {
    Err(ELSEWHERE.into())
}

#[cfg(not(target_os = "macos"))]
fn delete(_: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn watch(_: AppHandle) {}

#[cfg(not(target_os = "macos"))]
fn unwatch() {}
