//! The entries the menu bar last showed, kept on disk: a restart draws them at
//! once while the first fetch runs, and a process without the menu bar (the
//! command line) can read them.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::TrayEntry;
use crate::app_state::atomic_write;
use crate::error::AppResult;
use crate::paths::{app_data_dir, ensure_app_dir};

/// Bumped when the entries' shape changes, so an older file is ignored
/// rather than misread.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    entries: Vec<TrayEntry>,
}

/// `<app data>/usage-snapshots.json`.
pub fn path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("usage-snapshots.json"))
}

/// The entries last saved, or none when there are none to trust.
pub fn load() -> Vec<TrayEntry> {
    let Ok(path) = path() else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<File>(&raw) {
        Ok(file) if file.version == VERSION => file.entries,
        _ => Vec::new(),
    }
}

pub fn save(entries: &[TrayEntry]) -> AppResult<()> {
    ensure_app_dir()?;
    let body = serde_json::to_vec(&File {
        version: VERSION,
        entries: entries.to_vec(),
    })?;
    atomic_write(&path()?, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_kind::AppKind;
    use crate::test_support::APP_DIR_TEST_LOCK as TEST_LOCK;
    use crate::usage::metrics::{Metric, MetricValue, Unit};

    fn purge() {
        let _ = fs::remove_dir_all(app_data_dir().unwrap());
    }

    fn entry() -> TrayEntry {
        TrayEntry {
            id: "p1".into(),
            app: AppKind::Claude,
            name: "Work".into(),
            command: "claude-work".into(),
            plan: Some("Max 20x".into()),
            usage: None,
            metrics: vec![Metric {
                id: "session".into(),
                label: "Session".into(),
                value: MetricValue::Meter {
                    used: 18.0,
                    limit: 100.0,
                    unit: Unit::Percent,
                    resets_at: None,
                    period_secs: Some(18000),
                    not_started: false,
                },
                state: None,
            }],
            error: None,
        }
    }

    #[test]
    fn saved_entries_load_back() {
        let _guard = TEST_LOCK.lock().unwrap();
        purge();
        save(&[entry()]).unwrap();
        let loaded = load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].metrics, entry().metrics);
        assert_eq!(loaded[0].plan.as_deref(), Some("Max 20x"));
        purge();
    }

    #[test]
    fn nothing_saved_or_another_version_loads_nothing() {
        let _guard = TEST_LOCK.lock().unwrap();
        purge();
        assert!(load().is_empty());
        ensure_app_dir().unwrap();
        fs::write(path().unwrap(), r#"{"version":999,"entries":[]}"#).unwrap();
        assert!(load().is_empty());
        fs::write(path().unwrap(), "not json").unwrap();
        assert!(load().is_empty());
        purge();
    }
}
