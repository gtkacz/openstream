//! Whether the operating system asks apps for less motion. egui has no signal for it.

// shortcut: read once per launch, subscribe to the portal's SettingChanged if live changes matter.

/// A portal that activates slowly or never must not hold the start screen still forever.
#[cfg(target_os = "linux")]
const PORTAL_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(target_os = "linux")]
pub async fn prefers_less_motion() -> bool {
    answer_within(PORTAL_PATIENCE, read_portal())
        .await
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
async fn read_portal() -> bool {
    use ashpd::desktop::settings::Settings;
    let settings = match Settings::new().await {
        Ok(settings) => settings,
        Err(error) => {
            tracing::debug!(%error, "no settings portal to ask about motion");
            return false;
        }
    };
    let standard = settings
        .read::<u32>("org.freedesktop.appearance", "reduced-motion")
        .await
        .ok();
    let animations_enabled = settings
        .read::<bool>("org.gnome.desktop.interface", "enable-animations")
        .await
        .ok();
    let less = decide(standard, animations_enabled);
    tracing::debug!(
        ?standard,
        ?animations_enabled,
        less,
        "system motion preference"
    );
    less
}

/// The portal's standard key speaks first, and a desktop without it may still have GNOME's.
#[cfg(target_os = "linux")]
fn decide(standard: Option<u32>, animations_enabled: Option<bool>) -> bool {
    match (standard, animations_enabled) {
        (Some(reduced), _) => reduced != 0,
        (None, Some(enabled)) => !enabled,
        (None, None) => false,
    }
}

#[cfg(target_os = "linux")]
async fn answer_within<T>(
    patience: std::time::Duration,
    read: impl Future<Output = T>,
) -> Option<T> {
    tokio::time::timeout(patience, read).await.ok()
}

#[cfg(windows)]
pub async fn prefers_less_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: windows_sys::core::BOOL = 1;
    // SAFETY: this action writes one BOOL through the pointer, which is valid for the whole call.
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    read != 0 && enabled == 0
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn prefers_less_motion() -> bool {
    false
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_first_answer_decides_and_silence_means_no_preference() {
        assert!(decide(Some(1), Some(true)));
        assert!(!decide(Some(0), Some(false)));
        assert!(decide(None, Some(false)));
        assert!(!decide(None, Some(true)));
        assert!(!decide(None, None));
    }

    #[tokio::test]
    async fn a_portal_that_never_answers_counts_as_no_answer() {
        let silence = answer_within(Duration::ZERO, std::future::pending::<u32>()).await;
        assert_eq!(silence, None);
    }

    #[tokio::test]
    async fn a_portal_that_answers_in_time_is_heard() {
        let heard = answer_within(Duration::from_secs(1), async { 1u32 }).await;
        assert_eq!(heard, Some(1));
    }
}
