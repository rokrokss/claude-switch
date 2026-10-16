//! When a meter is worth a notification. The rules are OpenUsage's quota
//! notifications (MIT; `PaceNotificationLogic.swift` and
//! `QuotaNotificationEvaluator.swift`), kept exact:
//!
//! - The first reading of a meter after launch is only a baseline, so a
//!   limit already low when claude-switch opens doesn't alert at once.
//! - "Cutting it close" fires when the pace moves into close-to-limit from
//!   below; "Will run out" when it reaches running out (or spent) from below.
//!   A jump straight from healthy to running out fires only the latter.
//! - "Almost out" fires when what's left crosses below 10%, pace or not.
//! - Each fires once per reset window; a later reset time re-arms them, and
//!   so does improving (for the pace alerts) or climbing back over 10%.
//! - A crossing counts as alerted only once the notification was delivered,
//!   and one no enabled trigger caught stays pending, so turning the trigger
//!   on while it still holds alerts on the next reading.
//! - A meter that stops reporting forgets what it had alerted.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;

use chrono::{DateTime, Utc};

use crate::settings::NotificationSettings;
use crate::usage::pace::MeterState;

/// What's left, below which a meter is almost out.
const ALMOST_OUT_BELOW: f64 = 0.10;
/// Reset times can wobble by milliseconds between fetches; only a later
/// reset by more than this starts a new window.
const RESET_JITTER_SECS: f64 = 1.0;

/// The three things a notification can say about a meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trigger {
    AlmostOut,
    CuttingItClose,
    WillRunOut,
}

impl Trigger {
    /// The notification's title, the name of its switch in Settings.
    pub fn title(self) -> &'static str {
        match self {
            Trigger::AlmostOut => "Almost out",
            Trigger::CuttingItClose => "Cutting it close",
            Trigger::WillRunOut => "Will run out",
        }
    }

    pub fn body(self) -> &'static str {
        match self {
            Trigger::AlmostOut => "Less than 10% of this limit is left.",
            Trigger::CuttingItClose => "At this pace, it ends with little to spare.",
            Trigger::WillRunOut => "At this pace, it runs out before it resets.",
        }
    }

    /// For notification identifiers and logs.
    pub fn id(self) -> &'static str {
        match self {
            Trigger::AlmostOut => "almost-out",
            Trigger::CuttingItClose => "cutting-it-close",
            Trigger::WillRunOut => "will-run-out",
        }
    }

    fn is_on(self, toggles: &NotificationSettings) -> bool {
        match self {
            Trigger::AlmostOut => toggles.almost_out,
            Trigger::CuttingItClose => toggles.cutting_it_close,
            Trigger::WillRunOut => toggles.will_run_out,
        }
    }
}

/// How bad a meter's pace is, worst last. States with no pace to go on
/// (a share-used band, a session not yet started) are `Untracked`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
    #[default]
    Untracked,
    Healthy,
    Close,
    RunningOut,
}

impl Bucket {
    fn of(state: &MeterState) -> Self {
        match state {
            MeterState::Level { .. } => Bucket::Untracked,
            MeterState::Healthy { .. } => Bucket::Healthy,
            MeterState::CloseToLimit { .. } => Bucket::Close,
            MeterState::RunningOut { .. } | MeterState::Spent => Bucket::RunningOut,
        }
    }
}

/// What is remembered about one meter between readings.
#[derive(Debug, Clone, Default, PartialEq)]
struct Memory {
    /// The reset of the window `fired` belongs to.
    resets_at: Option<DateTime<Utc>>,
    /// Alerted in this window.
    fired: BTreeSet<Trigger>,
    bucket: Bucket,
    was_almost_out: bool,
    /// A baseline has been recorded.
    primed: bool,
}

#[derive(Debug, PartialEq)]
struct Transition {
    fire: Vec<Trigger>,
    next: Memory,
}

/// Which triggers a reading fires, and what to remember of it. `state` is
/// `None` for a meter without data. The fired triggers are not marked as
/// such here: that waits until they were delivered.
fn transition(
    state: Option<&MeterState>,
    remaining: f64,
    resets_at: Option<DateTime<Utc>>,
    previous: &Memory,
    toggles: &NotificationSettings,
) -> Transition {
    let mut next = previous.clone();
    if window_advanced(resets_at, previous.resets_at) {
        next.fired.clear();
        next.was_almost_out = false;
        next.bucket = Bucket::Untracked;
    }
    next.resets_at = resets_at.or(previous.resets_at);

    // Nothing to go on: fire nothing and change nothing, so a gap in the
    // data reads as neither an improvement nor a worsening.
    let Some(state) = state else {
        return Transition {
            fire: Vec::new(),
            next,
        };
    };
    let current = Bucket::of(state);
    let almost_out = remaining < ALMOST_OUT_BELOW;

    if !next.primed {
        next.primed = true;
        next.bucket = current;
        next.was_almost_out = almost_out;
        next.fired.clear();
        return Transition {
            fire: Vec::new(),
            next,
        };
    }

    let mut fire = Vec::new();
    let candidate =
        |trigger: Trigger, next: &Memory| trigger.is_on(toggles) && !next.fired.contains(&trigger);

    if current != Bucket::Untracked {
        let before = next.bucket;
        let mut pace_fired = false;
        // Into close from below; a jump past it to running out skips it.
        if current == Bucket::Close
            && before < Bucket::Close
            && candidate(Trigger::CuttingItClose, &next)
        {
            fire.push(Trigger::CuttingItClose);
            pace_fired = true;
        }
        if current >= Bucket::RunningOut
            && before < Bucket::RunningOut
            && candidate(Trigger::WillRunOut, &next)
        {
            fire.push(Trigger::WillRunOut);
            pace_fired = true;
        }
        // Improving re-arms what no longer applies.
        if current < before {
            if current <= Bucket::Healthy {
                next.fired.remove(&Trigger::CuttingItClose);
            }
            if current <= Bucket::Close {
                next.fired.remove(&Trigger::WillRunOut);
            }
        }
        // A worsening nothing alerted stays pending.
        if current <= before || pace_fired {
            next.bucket = current;
        }
    }

    let crossed = almost_out && !next.was_almost_out;
    let mut almost_out_fired = false;
    if crossed && candidate(Trigger::AlmostOut, &next) {
        fire.push(Trigger::AlmostOut);
        almost_out_fired = true;
    }
    if !almost_out {
        next.fired.remove(&Trigger::AlmostOut);
    }
    if !crossed || almost_out_fired {
        next.was_almost_out = almost_out;
    }

    Transition { fire, next }
}

/// Whether `resets_at` starts a later window than `previous`.
fn window_advanced(resets_at: Option<DateTime<Utc>>, previous: Option<DateTime<Utc>>) -> bool {
    match (resets_at, previous) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(resets_at), Some(previous)) => {
            (resets_at - previous).num_milliseconds() as f64 / 1000.0 > RESET_JITTER_SECS
        }
    }
}

/// One meter as read after a fetch.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Stable across fetches: the profile's id and the metric's.
    pub key: String,
    /// The profile's name and the metric's label.
    pub subtitle: String,
    pub state: Option<MeterState>,
    /// The share of the limit left, 0 to 1.
    pub remaining: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

/// A notification to deliver.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub key: String,
    pub trigger: Trigger,
    pub subtitle: String,
}

/// What has been alerted, meter by meter, since launch.
#[derive(Debug, Default)]
pub struct Evaluator {
    memory: HashMap<String, Memory>,
}

impl Evaluator {
    /// Delivers, through `deliver`, an alert for every trigger `readings`
    /// fire, and remembers what was delivered. `deliver` says whether the
    /// alert reached the user. Meters missing from `readings` are forgotten.
    pub async fn evaluate<F, Fut>(
        &mut self,
        readings: Vec<Reading>,
        toggles: &NotificationSettings,
        mut deliver: F,
    ) where
        F: FnMut(Alert) -> Fut,
        Fut: Future<Output = bool>,
    {
        let mut memory = HashMap::with_capacity(readings.len());
        for reading in readings {
            let previous = self.memory.remove(&reading.key).unwrap_or_default();
            let Transition { fire, mut next } = transition(
                reading.state.as_ref(),
                reading.remaining,
                reading.resets_at,
                &previous,
                toggles,
            );
            let mut pace_delivered = false;
            let mut almost_out_delivered = false;
            for &trigger in &fire {
                let alert = Alert {
                    key: reading.key.clone(),
                    trigger,
                    subtitle: reading.subtitle.clone(),
                };
                if deliver(alert).await {
                    next.fired.insert(trigger);
                    if trigger == Trigger::AlmostOut {
                        almost_out_delivered = true;
                    } else {
                        pace_delivered = true;
                    }
                }
            }
            // An alert that didn't arrive leaves its crossing pending.
            if fire.iter().any(|t| *t != Trigger::AlmostOut) && !pace_delivered {
                next.bucket = previous.bucket;
            }
            if fire.contains(&Trigger::AlmostOut) && !almost_out_delivered {
                next.was_almost_out = previous.was_almost_out;
            }
            memory.insert(reading.key, next);
        }
        self.memory = memory;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::pace::Severity;

    fn all_on() -> NotificationSettings {
        NotificationSettings {
            almost_out: true,
            cutting_it_close: true,
            will_run_out: true,
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn reset() -> DateTime<Utc> {
        at(1_700_000_000)
    }

    fn healthy() -> MeterState {
        MeterState::Healthy {
            projected_fraction: 0.5,
        }
    }

    fn close() -> MeterState {
        MeterState::CloseToLimit {
            spare_percent: 5,
            projected_fraction: 0.95,
        }
    }

    fn running() -> MeterState {
        MeterState::RunningOut {
            runs_out_at: None,
            projected_fraction: 1.2,
        }
    }

    fn level(severity: Severity) -> MeterState {
        MeterState::Level { severity }
    }

    /// One reading from `previous`, half the limit left unless said.
    struct Step {
        state: Option<MeterState>,
        remaining: f64,
        resets_at: DateTime<Utc>,
        toggles: NotificationSettings,
    }

    fn step(state: MeterState) -> Step {
        Step {
            state: Some(state),
            remaining: 0.5,
            resets_at: reset(),
            toggles: all_on(),
        }
    }

    impl Step {
        fn left(mut self, remaining: f64) -> Self {
            self.remaining = remaining;
            self
        }

        fn resetting(mut self, resets_at: DateTime<Utc>) -> Self {
            self.resets_at = resets_at;
            self
        }

        fn toggles(mut self, toggles: NotificationSettings) -> Self {
            self.toggles = toggles;
            self
        }

        fn from(self, previous: &Memory) -> Transition {
            transition(
                self.state.as_ref(),
                self.remaining,
                Some(self.resets_at),
                previous,
                &self.toggles,
            )
        }

        fn first(self) -> Transition {
            self.from(&Memory::default())
        }
    }

    fn no_data() -> Step {
        Step {
            state: None,
            ..step(healthy())
        }
    }

    // Pace edges.

    #[test]
    fn healthy_to_close_fires_once() {
        let first = step(healthy()).first();
        assert!(first.fire.is_empty());
        let second = step(close()).from(&first.next);
        assert_eq!(second.fire, [Trigger::CuttingItClose]);
        assert!(step(close()).from(&second.next).fire.is_empty());
    }

    #[test]
    fn close_to_running_out_fires_will_run_out() {
        let state = step(healthy()).first().next;
        let state = step(close()).from(&state).next;
        assert_eq!(step(running()).from(&state).fire, [Trigger::WillRunOut]);
    }

    #[test]
    fn a_jump_from_healthy_to_running_out_fires_only_will_run_out() {
        let state = step(healthy()).first().next;
        assert_eq!(step(running()).from(&state).fire, [Trigger::WillRunOut]);
    }

    #[test]
    fn reaching_spent_from_below_fires_will_run_out() {
        let state = step(healthy()).first().next;
        assert_eq!(
            step(MeterState::Spent).from(&state).fire,
            [Trigger::WillRunOut]
        );
    }

    // Almost out.

    #[test]
    fn almost_out_fires_once_per_window() {
        let primed = step(close()).left(0.5).first().next;
        let first = step(close()).left(0.08).from(&primed);
        assert!(first.fire.contains(&Trigger::AlmostOut));
        let again = step(close()).left(0.05).from(&first.next);
        assert!(!again.fire.contains(&Trigger::AlmostOut));
    }

    #[test]
    fn climbing_back_over_a_tenth_re_arms_almost_out() {
        let primed = step(close()).left(0.5).first().next;
        let first = step(close()).left(0.05).from(&primed);
        assert!(first.fire.contains(&Trigger::AlmostOut));
        let recovered = step(close()).left(0.5).from(&first.next);
        assert!(!recovered.fire.contains(&Trigger::AlmostOut));
        let dips = step(close()).left(0.05).from(&recovered.next);
        assert!(dips.fire.contains(&Trigger::AlmostOut));
    }

    // The baseline.

    #[test]
    fn the_first_reading_is_only_a_baseline() {
        let first = step(running()).left(0.02).first();
        assert!(first.fire.is_empty());
        assert!(step(level(Severity::Critical))
            .left(0.01)
            .first()
            .fire
            .is_empty());
        assert!(step(level(Severity::Normal))
            .left(0.99)
            .first()
            .fire
            .is_empty());
        // Worsening again after a recovery alerts as usual.
        let recovered = step(healthy()).left(0.5).from(&first.next).next;
        let red = step(running()).left(0.02).from(&recovered);
        assert!(red.fire.contains(&Trigger::WillRunOut));
        assert!(red.fire.contains(&Trigger::AlmostOut));
    }

    // Reset windows.

    #[test]
    fn a_later_reset_re_arms_what_fired() {
        let state = step(healthy()).first().next;
        let state = step(close()).from(&state).next;
        assert!(step(close()).from(&state).fire.is_empty());
        let later = reset() + chrono::Duration::hours(1);
        let rolled = step(healthy()).resetting(later).from(&state).next;
        assert_eq!(
            step(close()).resetting(later).from(&rolled).fire,
            [Trigger::CuttingItClose]
        );
    }

    #[test]
    fn a_reset_that_wobbles_by_milliseconds_is_the_same_window() {
        let wobbled = reset() + chrono::Duration::milliseconds(90);
        for worse in [close(), running()] {
            let primed = step(healthy()).first().next;
            let fired = step(worse.clone()).from(&primed).next;
            assert!(step(worse).resetting(wobbled).from(&fired).fire.is_empty());
        }
    }

    // Improvement.

    #[test]
    fn improving_re_arms_the_pace_alerts() {
        let state = step(healthy()).first().next;
        let state = step(close()).from(&state).next;
        let state = step(running()).from(&state).next;
        let state = step(healthy()).from(&state).next;
        let close_again = step(close()).from(&state);
        assert_eq!(close_again.fire, [Trigger::CuttingItClose]);
        assert_eq!(
            step(running()).from(&close_again.next).fire,
            [Trigger::WillRunOut]
        );
    }

    // No data, and no pace.

    #[test]
    fn no_data_never_fires() {
        assert!(no_data().left(0.01).first().fire.is_empty());
        let primed = step(healthy()).first().next;
        assert!(no_data().left(0.01).from(&primed).fire.is_empty());
    }

    #[test]
    fn a_share_used_band_can_be_almost_out_but_has_no_pace() {
        let primed = step(level(Severity::Critical)).left(0.2).first().next;
        let first = step(level(Severity::Critical)).left(0.08).from(&primed);
        assert_eq!(first.fire, [Trigger::AlmostOut]);
    }

    #[test]
    fn a_gap_in_the_data_neither_improves_nor_swallows_a_worsening() {
        let state = step(healthy()).first().next;
        let state = no_data().from(&state).next;
        assert_eq!(step(close()).from(&state).fire, [Trigger::CuttingItClose]);
    }

    // Switches.

    #[test]
    fn triggers_that_are_off_do_not_fire() {
        let off = NotificationSettings::default();
        let state = step(healthy()).toggles(off.clone()).first().next;
        assert!(step(close())
            .left(0.05)
            .toggles(off)
            .from(&state)
            .fire
            .is_empty());
    }

    #[test]
    fn turning_one_trigger_off_silences_only_that_one() {
        let toggles = NotificationSettings {
            cutting_it_close: true,
            ..NotificationSettings::default()
        };
        let state = step(healthy()).toggles(toggles.clone()).first().next;
        let close = step(close())
            .left(0.05)
            .toggles(toggles.clone())
            .from(&state);
        assert_eq!(close.fire, [Trigger::CuttingItClose]);
        assert!(step(running())
            .left(0.02)
            .toggles(toggles)
            .from(&close.next)
            .fire
            .is_empty());
    }

    #[test]
    fn a_crossing_while_its_trigger_is_off_stays_pending() {
        let off = NotificationSettings {
            will_run_out: true,
            ..NotificationSettings::default()
        };
        let state = step(healthy()).toggles(off.clone()).first().next;
        let skipped = step(close()).left(0.2).toggles(off).from(&state);
        assert!(skipped.fire.is_empty());
        let on = NotificationSettings {
            cutting_it_close: true,
            will_run_out: true,
            ..NotificationSettings::default()
        };
        let fired = step(close()).left(0.2).toggles(on).from(&skipped.next);
        assert!(fired.fire.contains(&Trigger::CuttingItClose));
    }

    // The evaluator: delivery, pruning.

    fn reading(key: &str, state: MeterState, remaining: f64) -> Reading {
        Reading {
            key: key.to_owned(),
            subtitle: format!("Work {key}"),
            state: Some(state),
            remaining,
            resets_at: Some(reset()),
        }
    }

    /// Runs one evaluation, delivering (or failing to) every alert, and
    /// returns them.
    fn evaluate(
        evaluator: &mut Evaluator,
        readings: Vec<Reading>,
        toggles: &NotificationSettings,
        delivered: bool,
    ) -> Vec<Alert> {
        let mut alerts = Vec::new();
        tauri::async_runtime::block_on(evaluator.evaluate(readings, toggles, |alert| {
            alerts.push(alert);
            std::future::ready(delivered)
        }));
        alerts
    }

    #[test]
    fn alerts_carry_the_trigger_and_whose_metric_it_is() {
        let mut evaluator = Evaluator::default();
        let toggles = all_on();
        assert!(evaluate(
            &mut evaluator,
            vec![reading("session", healthy(), 0.2)],
            &toggles,
            true
        )
        .is_empty());
        let alerts = evaluate(
            &mut evaluator,
            vec![reading("session", running(), 0.05)],
            &toggles,
            true,
        );
        assert_eq!(
            alerts,
            [
                Alert {
                    key: "session".into(),
                    trigger: Trigger::WillRunOut,
                    subtitle: "Work session".into(),
                },
                Alert {
                    key: "session".into(),
                    trigger: Trigger::AlmostOut,
                    subtitle: "Work session".into(),
                },
            ]
        );
        // Delivered, so not again this window.
        assert!(evaluate(
            &mut evaluator,
            vec![reading("session", running(), 0.04)],
            &toggles,
            true
        )
        .is_empty());
    }

    #[test]
    fn an_alert_that_was_not_delivered_is_tried_again() {
        let mut evaluator = Evaluator::default();
        let toggles = all_on();
        evaluate(
            &mut evaluator,
            vec![reading("session", healthy(), 0.5)],
            &toggles,
            false,
        );
        let first = evaluate(
            &mut evaluator,
            vec![reading("session", close(), 0.05)],
            &toggles,
            false,
        );
        assert_eq!(first.len(), 2);
        let again = evaluate(
            &mut evaluator,
            vec![reading("session", close(), 0.05)],
            &toggles,
            true,
        );
        assert_eq!(again.len(), 2);
        assert!(evaluate(
            &mut evaluator,
            vec![reading("session", close(), 0.05)],
            &toggles,
            true
        )
        .is_empty());
    }

    #[test]
    fn a_meter_that_stops_reporting_is_forgotten() {
        let mut evaluator = Evaluator::default();
        let toggles = all_on();
        evaluate(
            &mut evaluator,
            vec![reading("session", healthy(), 0.5)],
            &toggles,
            true,
        );
        evaluate(
            &mut evaluator,
            vec![reading("weekly", healthy(), 0.5)],
            &toggles,
            true,
        );
        // Back, it starts over from a baseline.
        assert!(evaluate(
            &mut evaluator,
            vec![reading("session", running(), 0.05)],
            &toggles,
            true
        )
        .is_empty());
    }

    #[test]
    fn meters_are_judged_apart() {
        let mut evaluator = Evaluator::default();
        let toggles = all_on();
        let both = |session, weekly| {
            vec![
                reading("session", session, 0.5),
                reading("weekly", weekly, 0.5),
            ]
        };
        evaluate(&mut evaluator, both(healthy(), healthy()), &toggles, true);
        let alerts = evaluate(&mut evaluator, both(healthy(), close()), &toggles, true);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].key, "weekly");
    }
}
