//! When each Claude profile the usage endpoint rate limited may be asked
//! again, kept on disk and shared by the app and `claude-switch usage`: a
//! restart, or another process, mustn't ask while the profile is held back.
//! Asking during a hold doesn't end it sooner, and may be what earns the next
//! one.
//!
//! A hold lasts the endpoint's `Retry-After` (or [`DEFAULT_WAIT`]). A rate
//! limit straight after another one ran out waits longer each time, by
//! [`STEP`] doubling, up to [`MAX_WAIT`]; an answer with usage forgets them.
//!
//! `<app data>/usage-backoff.json`, beside the menu bar's cache.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::app_state::atomic_write;
use crate::error::AppResult;
use crate::paths::{app_data_dir, ensure_app_dir};

/// Bumped when the file's shape changes, so an older file is ignored rather
/// than misread.
const VERSION: u32 = 2;
/// The wait after a rate limit that names none.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(5 * 60);
/// What a second rate limit in a row adds to the wait, doubling with each
/// after it.
const STEP: Duration = Duration::from_secs(5 * 60);
/// The longest hold, however many rate limits in a row.
const MAX_WAIT: Duration = Duration::from_secs(3 * 60 * 60);
/// A hold that ran out longer ago than this no longer counts toward the next.
const FORGET_AFTER: TimeDelta = TimeDelta::days(1);

/// A profile held back: until when, after how many rate limits in a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    pub until: DateTime<Utc>,
    pub strikes: u32,
}

/// Profile id → its hold, run out or not.
pub type Holds = HashMap<String, Hold>;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    holds: Holds,
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
        Ok(file) if file.version == VERSION => file.holds,
        _ => Holds::new(),
    }
}

fn save(holds: &Holds) -> AppResult<()> {
    ensure_app_dir()?;
    let body = serde_json::to_vec(&File {
        version: VERSION,
        holds: holds.clone(),
    })?;
    atomic_write(&path()?, &body)
}

/// Pure: until when profile `id` is held back, as of `now`, if it is.
pub fn until(holds: &Holds, id: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    holds
        .get(id)
        .map(|hold| hold.until)
        .filter(|until| *until > now)
}

/// Pure: the hold a rate limit at `now` puts on a profile last held by
/// `previous`, for the `retry_after` the endpoint named, if it named one.
pub fn next(previous: Option<&Hold>, retry_after: Option<Duration>, now: DateTime<Utc>) -> Hold {
    let strikes = match previous {
        Some(last) if now - last.until < FORGET_AFTER => last.strikes.saturating_add(1),
        _ => 1,
    };
    let extra = match strikes {
        0 | 1 => Duration::ZERO,
        _ => STEP.saturating_mul(2u32.saturating_pow(strikes - 2)),
    };
    let wait = retry_after
        .unwrap_or(DEFAULT_WAIT)
        .saturating_add(extra)
        .min(MAX_WAIT);
    Hold {
        until: now + TimeDelta::from_std(wait).unwrap_or(TimeDelta::zero()),
        strikes,
    }
}

/// Holds profile `id` back after a rate limit, and saves it. The hold set.
pub fn hold(id: &str, retry_after: Option<Duration>) -> AppResult<Hold> {
    let mut holds = load();
    let hold = next(holds.get(id), retry_after, Utc::now());
    holds.insert(id.to_owned(), hold);
    save(&holds)?;
    Ok(hold)
}

/// Lets profile `id` go after an answer with usage, forgetting its rate
/// limits. Writes nothing when it wasn't held.
pub fn release(id: &str) -> AppResult<()> {
    let mut holds = load();
    if holds.remove(id).is_none() {
        return Ok(());
    }
    save(&holds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::APP_DIR_TEST_LOCK as TEST_LOCK;

    fn purge() {
        let _ = fs::remove_dir_all(app_data_dir().unwrap());
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    const HOUR: Duration = Duration::from_secs(60 * 60);

    #[test]
    fn a_hold_is_kept_until_an_answer_lets_it_go() {
        let _guard = TEST_LOCK.lock().unwrap();
        purge();
        let first = hold("p1", Some(HOUR)).unwrap();
        assert_eq!(first.strikes, 1);
        assert_eq!(load().get("p1"), Some(&first));
        release("p1").unwrap();
        assert!(load().is_empty());
        purge();
    }

    #[test]
    fn nothing_saved_or_another_version_loads_nothing() {
        let _guard = TEST_LOCK.lock().unwrap();
        purge();
        assert!(load().is_empty());
        ensure_app_dir().unwrap();
        fs::write(path().unwrap(), r#"{"version":1,"until":{}}"#).unwrap();
        assert!(load().is_empty());
        fs::write(path().unwrap(), "not json").unwrap();
        assert!(load().is_empty());
        purge();
    }

    #[test]
    fn a_profile_is_held_until_its_time_and_no_longer() {
        let holds = Holds::from([(
            "p1".to_owned(),
            Hold {
                until: at("2026-10-17T12:30:00Z"),
                strikes: 1,
            },
        )]);
        assert_eq!(
            until(&holds, "p1", at("2026-10-17T12:29:59Z")),
            Some(at("2026-10-17T12:30:00Z"))
        );
        assert_eq!(until(&holds, "p1", at("2026-10-17T12:30:00Z")), None);
        assert_eq!(until(&holds, "p2", at("2026-10-17T12:00:00Z")), None);
    }

    #[test]
    fn the_first_rate_limit_waits_what_the_endpoint_says_or_five_minutes() {
        let now = at("2026-10-17T12:00:00Z");
        assert_eq!(
            next(None, Some(HOUR), now),
            Hold {
                until: at("2026-10-17T13:00:00Z"),
                strikes: 1
            }
        );
        assert_eq!(next(None, None, now).until, at("2026-10-17T12:05:00Z"));
    }

    #[test]
    fn rate_limits_in_a_row_wait_longer_each_time() {
        let mut now = at("2026-10-17T12:00:00Z");
        let mut last = None;
        let mut waits = Vec::new();
        for _ in 0..7 {
            let hold = next(last.as_ref(), Some(HOUR), now);
            waits.push((hold.until - now).num_minutes());
            now = hold.until;
            last = Some(hold);
        }
        assert_eq!(waits, [60, 65, 70, 80, 100, 140, 180]);
    }

    #[test]
    fn a_hold_long_run_out_starts_the_count_again() {
        let long_ago = Hold {
            until: at("2026-10-15T12:00:00Z"),
            strikes: 4,
        };
        assert_eq!(
            next(Some(&long_ago), Some(HOUR), at("2026-10-17T12:00:00Z")).strikes,
            1
        );
    }
}
