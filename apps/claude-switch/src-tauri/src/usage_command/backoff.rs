//! When each Claude profile the usage endpoint rate limited may be asked
//! again, kept on disk: the app keeps its back-off in memory, and each
//! `claude-switch usage` is a new process, so without this a script looping
//! it would ask on every run.
//!
//! `<app data>/usage-backoff.json`, beside the menu bar's cache, which the
//! app owns: this file is the command line's.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::app_state::atomic_write;
use crate::error::AppResult;
use crate::paths::{app_data_dir, ensure_app_dir};

/// Bumped when the file's shape changes, so an older file is ignored rather
/// than misread.
const VERSION: u32 = 1;

/// Profile id → when it may be asked again.
pub type Holds = HashMap<String, DateTime<Utc>>;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    until: Holds,
}

/// `<app data>/usage-backoff.json`.
pub fn path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("usage-backoff.json"))
}

/// The holds last saved, or none when there are none to trust.
pub fn load() -> Holds {
    let Ok(path) = path() else {
        return Holds::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Holds::new();
    };
    match serde_json::from_str::<File>(&raw) {
        Ok(file) if file.version == VERSION => file.until,
        _ => Holds::new(),
    }
}

pub fn save(holds: &Holds) -> AppResult<()> {
    ensure_app_dir()?;
    let body = serde_json::to_vec(&File {
        version: VERSION,
        until: holds.clone(),
    })?;
    atomic_write(&path()?, &body)
}

/// Pure: until when profile `id` is held back, as of `now`, if it is.
pub fn until(holds: &Holds, id: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    holds.get(id).copied().filter(|until| *until > now)
}

/// Pure: `holds` with this run's fetches recorded, each a profile id and when
/// it may be asked again, `None` when its fetch wasn't rate limited, and the
/// holds run out by `now` dropped. Whether that changed them.
pub fn record(
    holds: &mut Holds,
    fetched: &[(String, Option<DateTime<Utc>>)],
    now: DateTime<Utc>,
) -> bool {
    let before = holds.clone();
    for (id, until) in fetched {
        match until {
            Some(until) => holds.insert(id.clone(), *until),
            None => holds.remove(id),
        };
    }
    holds.retain(|_, until| *until > now);
    *holds != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::APP_DIR_TEST_LOCK as TEST_LOCK;
    use chrono::TimeDelta;

    fn purge() {
        let _ = fs::remove_dir_all(app_data_dir().unwrap());
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn saved_holds_load_back() {
        let _guard = TEST_LOCK.lock().unwrap();
        purge();
        let holds = Holds::from([("p1".to_owned(), at("2026-10-17T12:30:00Z"))]);
        save(&holds).unwrap();
        assert_eq!(load(), holds);
        purge();
    }

    #[test]
    fn nothing_saved_or_another_version_loads_nothing() {
        let _guard = TEST_LOCK.lock().unwrap();
        purge();
        assert!(load().is_empty());
        ensure_app_dir().unwrap();
        fs::write(path().unwrap(), r#"{"version":999,"until":{}}"#).unwrap();
        assert!(load().is_empty());
        fs::write(path().unwrap(), "not json").unwrap();
        assert!(load().is_empty());
        purge();
    }

    #[test]
    fn a_profile_is_held_until_its_time_and_no_longer() {
        let holds = Holds::from([("p1".to_owned(), at("2026-10-17T12:30:00Z"))]);
        assert_eq!(
            until(&holds, "p1", at("2026-10-17T12:29:59Z")),
            Some(at("2026-10-17T12:30:00Z"))
        );
        assert_eq!(until(&holds, "p1", at("2026-10-17T12:30:00Z")), None);
        assert_eq!(until(&holds, "p2", at("2026-10-17T12:00:00Z")), None);
    }

    #[test]
    fn a_rate_limit_holds_a_profile_and_anything_else_lets_it_go() {
        let now = at("2026-10-17T12:00:00Z");
        let mut holds = Holds::from([
            ("released".to_owned(), now + TimeDelta::minutes(10)),
            ("kept".to_owned(), now + TimeDelta::minutes(10)),
            ("run out".to_owned(), now - TimeDelta::seconds(1)),
        ]);
        let fetched = [
            ("held".to_owned(), Some(now + TimeDelta::minutes(30))),
            ("released".to_owned(), None),
            ("never held".to_owned(), None),
        ];
        assert!(record(&mut holds, &fetched, now));
        assert_eq!(
            holds,
            Holds::from([
                ("held".to_owned(), now + TimeDelta::minutes(30)),
                ("kept".to_owned(), now + TimeDelta::minutes(10)),
            ])
        );
    }

    #[test]
    fn fetches_that_change_no_hold_change_nothing() {
        let now = at("2026-10-17T12:00:00Z");
        let mut holds = Holds::new();
        assert!(!record(&mut holds, &[("p1".to_owned(), None)], now));
        let mut holds = Holds::from([("p1".to_owned(), now + TimeDelta::minutes(5))]);
        assert!(!record(&mut holds, &[("p2".to_owned(), None)], now));
    }
}
