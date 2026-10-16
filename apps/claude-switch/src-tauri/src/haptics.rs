//! A trackpad tap for each slot a dragged row snaps into, as macOS's own
//! lists give (OpenUsage's Customize does the same).

/// One light tap, on a Force Touch trackpad; nothing elsewhere.
#[tauri::command]
pub fn haptic_tick() {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{
            NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
            NSHapticFeedbackPerformer,
        };
        NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
            NSHapticFeedbackPattern::LevelChange,
            NSHapticFeedbackPerformanceTime::Now,
        );
    }
}
