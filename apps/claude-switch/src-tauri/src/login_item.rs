//! Opening claude-switch at login, so its menu bar item is there after a
//! restart. It is macOS's own login item for the app (`SMAppService`), the
//! one System Settings → General → Login Items lists and can turn off, so
//! the state is always read from macOS rather than kept here.

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Whether claude-switch opens at login, as macOS has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginItem {
    On,
    Off,
    /// Turned on, but waiting for the user to allow it in System Settings.
    NeedsApproval,
    /// This macOS has no login item API for apps (before macOS 13).
    Unavailable,
}

#[tauri::command]
pub fn login_item() -> LoginItem {
    platform::status()
}

#[tauri::command]
pub fn set_login_item(enabled: bool) -> AppResult<LoginItem> {
    platform::set(enabled).map_err(AppError::Validation)?;
    Ok(platform::status())
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::NSError;

    use super::LoginItem;

    #[link(name = "ServiceManagement", kind = "framework")]
    extern "C" {}

    /// `SMAppService.mainAppService`: the running app's own login item.
    fn main_app() -> Option<Retained<AnyObject>> {
        let class = AnyClass::get(c"SMAppService")?;
        // SAFETY: a class property returning an autoreleased SMAppService.
        unsafe { msg_send![class, mainAppService] }
    }

    pub fn status() -> LoginItem {
        let Some(service) = main_app() else {
            return LoginItem::Unavailable;
        };
        // SAFETY: `status` is an NSInteger property of SMAppService.
        let status: isize = unsafe { msg_send![&*service, status] };
        // SMAppServiceStatus: notRegistered, enabled, requiresApproval, notFound.
        // The main app reads as notFound until it is first registered, so
        // that is off too; a copy that can't be one says why when turned on.
        match status {
            1 => LoginItem::On,
            2 => LoginItem::NeedsApproval,
            _ => LoginItem::Off,
        }
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let service = main_app().ok_or("this version of macOS has no login items API")?;
        let mut error: *mut NSError = std::ptr::null_mut();
        // SAFETY: both take an NSError out-parameter and return BOOL.
        let ok: Bool = unsafe {
            if enabled {
                msg_send![&*service, registerAndReturnError: &mut error]
            } else {
                msg_send![&*service, unregisterAndReturnError: &mut error]
            }
        };
        if ok.as_bool() {
            return Ok(());
        }
        // SAFETY: on failure the method leaves an autoreleased NSError, or none.
        let message = unsafe { error.as_ref() }
            .map(|error| error.localizedDescription().to_string())
            .unwrap_or_else(|| "macOS would not change it".to_owned());
        Err(format!(
            "{message} Check System Settings → General → Login Items."
        ))
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::LoginItem;

    pub fn status() -> LoginItem {
        LoginItem::Unavailable
    }

    pub fn set(_enabled: bool) -> Result<(), String> {
        Err("login items are macOS only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_the_state_never_fails() {
        // A test binary isn't a bundle; whatever macOS says, asking is safe.
        let _ = login_item();
    }
}
