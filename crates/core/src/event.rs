//! The system event bus vocabulary.
//!
//! The platform layer (macOS today, Windows later) turns OS signals into these
//! plain Rust values and hands them to the engine, which feeds them to every
//! running live collector. Nothing in here knows about any operating system.

/// One OS-level happening, already translated out of platform types.
#[derive(Debug, Clone, PartialEq)]
pub enum SystemEvent {
    /// The frontmost application changed (or is reported for the first time).
    /// `bundle_id` is empty when the app has none (e.g. a bare executable).
    FrontApp {
        bundle_id: String,
        name: String,
    },
    /// Periodic (about once a minute) idle check.
    IdleTick {
        /// Seconds since the last keyboard/mouse/trackpad input.
        secs_since_input: f64,
        /// "Keep the display awake" assertions held at this tick (macOS
        /// `PreventUserIdleDisplaySleep`), each attributed to the frontmost
        /// app or not by the platform layer.
        display_assertions: Vec<DisplayAssertion>,
    },
    WillSleep,
    DidWake,
    ScreensSlept,
    ScreensWoke,
    ScreenLocked,
    ScreenUnlocked,
    WillPowerOff,
    /// Tracking was paused (tray, CLI, or restored at startup).
    Paused,
    /// Tracking resumed (by the user or because a timed pause expired).
    Resumed,
}

/// One "keep the display awake" assertion seen at an idle tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayAssertion {
    /// Who holds it, e.g. `"caffeinate[49222] caffeinate command-line tool"`.
    /// For logging only.
    pub holder: String,
    /// Held by the frontmost app's process (or created on its behalf).
    /// Only these count as evidence of use; see `presence`.
    pub front_app: bool,
}

impl DisplayAssertion {
    pub fn new(holder: impl Into<String>, front_app: bool) -> Self {
        Self {
            holder: holder.into(),
            front_app,
        }
    }
}

impl SystemEvent {
    /// Short name for debug logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::FrontApp { .. } => "FrontApp",
            Self::IdleTick { .. } => "IdleTick",
            Self::WillSleep => "WillSleep",
            Self::DidWake => "DidWake",
            Self::ScreensSlept => "ScreensSlept",
            Self::ScreensWoke => "ScreensWoke",
            Self::ScreenLocked => "ScreenLocked",
            Self::ScreenUnlocked => "ScreenUnlocked",
            Self::WillPowerOff => "WillPowerOff",
            Self::Paused => "Paused",
            Self::Resumed => "Resumed",
        }
    }
}

/// System processes that are shown in front while the user is *not* using the
/// machine (lock screen, login window, screensaver). Time with one of these in
/// front is away time, never app time.
pub fn away_app_reason(bundle_id: &str) -> Option<AwayReason> {
    if bundle_id == "com.apple.loginwindow" {
        return Some(AwayReason::Locked);
    }
    // com.apple.ScreenSaver.Engine, com.apple.ScreenSaver.Engine.legacyScreenSaver, ...
    if bundle_id
        .to_ascii_lowercase()
        .starts_with("com.apple.screensaver")
    {
        return Some(AwayReason::ScreenSaver);
    }
    None
}

/// Why the user counts as away. Stored as the key of `active_idle` sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwayReason {
    Idle,
    Sleep,
    DisplayOff,
    Locked,
    ScreenSaver,
}

impl AwayReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Sleep => "sleep",
            Self::DisplayOff => "display_off",
            Self::Locked => "locked",
            Self::ScreenSaver => "screensaver",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn away_apps() {
        assert_eq!(
            away_app_reason("com.apple.loginwindow"),
            Some(AwayReason::Locked)
        );
        assert_eq!(
            away_app_reason("com.apple.ScreenSaver.Engine"),
            Some(AwayReason::ScreenSaver)
        );
        assert_eq!(
            away_app_reason("com.apple.ScreenSaver.Engine.legacyScreenSaver"),
            Some(AwayReason::ScreenSaver)
        );
        assert_eq!(away_app_reason("com.apple.Safari"), None);
    }
}
