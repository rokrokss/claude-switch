//! Settings for the menu bar item, its panel and what goes with them:
//! notifications, the command line and local API, sync, updates. Kept in the
//! app state (`state.json`) as one group per Settings section; a patch
//! replaces a group whole.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::usage::metrics::DEFAULT_STARS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub menu_bar: MenuBarSettings,
    pub display: DisplaySettings,
    pub layout: LayoutSettings,
    pub notifications: NotificationSettings,
    pub privacy: PrivacySettings,
    pub integrations: IntegrationSettings,
    pub sync: SyncSettings,
    pub updates: UpdateSettings,
    pub advanced: AdvancedSettings,
}

/// Replaces the groups it carries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsPatch {
    pub menu_bar: Option<MenuBarSettings>,
    pub display: Option<DisplaySettings>,
    pub layout: Option<LayoutSettings>,
    pub notifications: Option<NotificationSettings>,
    pub privacy: Option<PrivacySettings>,
    pub integrations: Option<IntegrationSettings>,
    pub sync: Option<SyncSettings>,
    pub updates: Option<UpdateSettings>,
    pub advanced: Option<AdvancedSettings>,
}

impl SettingsPatch {
    pub fn apply_to(self, settings: &mut Settings) {
        replace(&mut settings.menu_bar, self.menu_bar);
        replace(&mut settings.display, self.display);
        replace(&mut settings.layout, self.layout);
        replace(&mut settings.notifications, self.notifications);
        replace(&mut settings.privacy, self.privacy);
        replace(&mut settings.integrations, self.integrations);
        replace(&mut settings.sync, self.sync);
        replace(&mut settings.updates, self.updates);
        replace(&mut settings.advanced, self.advanced);
    }
}

fn replace<T>(slot: &mut T, group: Option<T>) {
    if let Some(group) = group {
        *slot = group;
    }
}

impl Settings {
    /// Drops what the settings keep about entry `id`, which is gone. True
    /// when that was anything.
    pub fn forget(&mut self, id: &str) -> bool {
        let starred = self.menu_bar.stars.remove(id).is_some();
        let laid_out = self.layout.profiles.remove(id).is_some();
        let expanded = self.layout.expanded.remove(id);
        starred || laid_out || expanded
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuBarSettings {
    pub style: MenuBarStyle,
    /// The metric ids starred for the strip, by entry id, in the order shown.
    /// An entry without one shows [`DEFAULT_STARS`]; an empty list shows none.
    pub stars: BTreeMap<String, Vec<String>>,
    /// The shortcut that opens the panel from anywhere, e.g. `CmdOrCtrl+Shift+U`.
    pub shortcut: Option<String>,
}

impl MenuBarSettings {
    /// The metrics starred for entry `id`.
    pub fn stars_for(&self, id: &str) -> Vec<String> {
        match self.stars.get(id) {
            Some(stars) => stars.clone(),
            None => DEFAULT_STARS
                .iter()
                .map(|star| (*star).to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MenuBarStyle {
    /// The app's mark beside its values.
    #[default]
    Text,
    /// A small glyph of up to four bars.
    Bars,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DisplaySettings {
    pub meter_style: MeterStyle,
    pub reset_display: ResetDisplay,
    /// Show the even-pace tick and projection on meters that are on course too.
    pub always_show_pacing: bool,
    pub time_format: TimeFormat,
    pub density: Density,
    pub reduce_animations: bool,
    pub increase_transparency: bool,
    pub total_spend: TotalSpendSettings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeterStyle {
    #[default]
    Used,
    Left,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetDisplay {
    /// "Resets in 3h 25m".
    #[default]
    Countdown,
    /// "Resets today at 18:38".
    Exact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFormat {
    /// The system's.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "12h")]
    TwelveHour,
    #[serde(rename = "24h")]
    TwentyFourHour,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Density {
    #[default]
    Default,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TotalSpendSettings {
    pub shown: bool,
    pub metric: TotalSpendMetric,
    pub period: SpendPeriod,
}

impl Default for TotalSpendSettings {
    fn default() -> Self {
        Self {
            shown: true,
            metric: TotalSpendMetric::default(),
            period: SpendPeriod::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TotalSpendMetric {
    #[default]
    Cost,
    CostPerMillionTokens,
    Tokens,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpendPeriod {
    #[default]
    Today,
    Yesterday,
    Last30Days,
}

/// Which metrics each profile's panel card shows, and where.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayoutSettings {
    /// By entry id. An entry without one shows its app's default layout.
    pub profiles: BTreeMap<String, ProfileLayout>,
    /// The entries whose cards are open to their on-demand metrics.
    pub expanded: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfileLayout {
    /// Metric ids always on the card, in order.
    pub always_visible: Vec<String>,
    /// Metric ids shown once the card is opened, in order. A metric in
    /// neither list is off.
    pub on_demand: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationSettings {
    /// Under 10% of a window left.
    pub almost_out: bool,
    /// Projected to finish close to the limit.
    pub cutting_it_close: bool,
    /// Projected to run out before the reset.
    pub will_run_out: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrivacySettings {
    /// Show the app's mark instead of usage while the screen is shared or recorded.
    pub hide_while_screen_sharing: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IntegrationSettings {
    /// Serve usage on 127.0.0.1 for other apps to read. Off: claude-switch
    /// opens no listener unless asked to.
    pub local_api: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncSettings {
    /// Combine spend history with this account's other Macs through iCloud.
    pub icloud: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateSettings {
    pub check_automatically: bool,
    pub beta: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_automatically: true,
            beta: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AdvancedSettings {
    pub log_level: LogLevel,
    /// The model whose prices stand in for Codex models without one.
    pub codex_fallback_model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Error,
    Warning,
    #[default]
    Info,
    Debug,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_read_as_the_defaults() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.display.meter_style, MeterStyle::Used);
        assert!(settings.display.total_spend.shown);
        assert!(settings.updates.check_automatically);
        assert!(!settings.integrations.local_api);
    }

    #[test]
    fn a_group_missing_a_field_keeps_the_rest() {
        let settings: Settings =
            serde_json::from_str(r#"{"display":{"meterStyle":"left"}}"#).unwrap();
        assert_eq!(settings.display.meter_style, MeterStyle::Left);
        assert_eq!(settings.display.reset_display, ResetDisplay::Countdown);
    }

    #[test]
    fn time_formats_read_as_written_in_settings() {
        let display: DisplaySettings = serde_json::from_str(r#"{"timeFormat":"24h"}"#).unwrap();
        assert_eq!(display.time_format, TimeFormat::TwentyFourHour);
    }

    #[test]
    fn a_patch_replaces_only_the_groups_it_carries() {
        let mut settings = Settings::default();
        settings.notifications.almost_out = true;
        SettingsPatch {
            display: Some(DisplaySettings {
                meter_style: MeterStyle::Left,
                ..DisplaySettings::default()
            }),
            ..SettingsPatch::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.display.meter_style, MeterStyle::Left);
        assert!(settings.notifications.almost_out);
    }

    #[test]
    fn an_entry_without_stars_shows_the_default_ones() {
        let mut menu_bar = MenuBarSettings::default();
        assert_eq!(menu_bar.stars_for("p1"), ["session", "weekly"]);
        menu_bar.stars.insert("p1".into(), Vec::new());
        assert!(menu_bar.stars_for("p1").is_empty());
    }

    #[test]
    fn forgetting_an_entry_drops_its_stars_and_layout() {
        let mut settings = Settings::default();
        settings
            .menu_bar
            .stars
            .insert("p1".into(), vec!["weekly".into()]);
        settings.layout.expanded.insert("p1".into());
        assert!(settings.forget("p1"));
        assert!(!settings.forget("p1"));
        assert!(settings.menu_bar.stars.is_empty() && settings.layout.expanded.is_empty());
    }
}
