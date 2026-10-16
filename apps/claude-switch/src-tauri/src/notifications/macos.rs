//! Posting through the User Notifications framework, so a notification comes
//! from claude-switch, with its name and icon, and a click on it comes back
//! here. As in OpenUsage (MIT; `AppNotifications.swift`).
//!
//! The framework only serves a process inside an app bundle, and throws
//! otherwise; a development run from `target/` can't post, and says so as
//! [`Permission::Unavailable`].

use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::AppHandle;
use tokio::sync::oneshot;

use super::rules::Alert;
use super::Permission;
use crate::tray;

/// Groups claude-switch's notifications into one stack, and tells a click
/// on one of them from any other.
const THREAD: &str = "claude-switch.usage";

/// Whether this process may use the framework at all.
fn bundled() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

pub async fn permission() -> Permission {
    if !bundled() {
        return Permission::Unavailable;
    }
    let (sender, receiver) = oneshot::channel();
    {
        let sender = Mutex::new(Some(sender));
        let handler = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // SAFETY: the framework hands the handler valid settings.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(status);
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&handler);
    }
    match receiver.await {
        Ok(UNAuthorizationStatus::NotDetermined) => Permission::NotDetermined,
        Ok(
            UNAuthorizationStatus::Authorized
            | UNAuthorizationStatus::Provisional
            | UNAuthorizationStatus::Ephemeral,
        ) => Permission::Granted,
        _ => Permission::Denied,
    }
}

/// Asks, if the user hasn't answered yet: macOS asks only once.
pub async fn request_permission() -> Permission {
    if permission().await == Permission::NotDetermined {
        request().await;
    }
    permission().await
}

/// Shows macOS's prompt; true when the user allows notifications.
async fn request() -> bool {
    let (sender, receiver) = oneshot::channel();
    {
        let sender = Mutex::new(Some(sender));
        let handler = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            if let Some(error) = describe(error) {
                eprintln!("claude-switch: could not ask to post notifications: {error}");
            }
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(granted.as_bool());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &handler,
            );
    }
    receiver.await.unwrap_or(false)
}

/// Posts `alert` at once; true once macOS has taken it. False when the user
/// hasn't allowed notifications, so the alert is tried again later. Asks
/// first if they haven't answered yet.
pub async fn post(alert: &Alert) -> bool {
    let allowed = match permission().await {
        Permission::Granted => true,
        Permission::NotDetermined => request().await,
        Permission::Denied | Permission::Unavailable => false,
    };
    if !allowed {
        return false;
    }
    let (sender, receiver) = oneshot::channel();
    {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(alert.trigger.title()));
        content.setSubtitle(&NSString::from_str(&alert.subtitle));
        content.setBody(&NSString::from_str(alert.trigger.body()));
        content.setThreadIdentifier(&NSString::from_str(THREAD));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        // Unique, so two alerts about one meter don't replace each other.
        let id = format!(
            "claude-switch.{}.{}.{}",
            alert.key,
            alert.trigger.id(),
            uuid::Uuid::new_v4()
        );
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&id),
            &content,
            None,
        );
        let sender = Mutex::new(Some(sender));
        let handler = RcBlock::new(move |error: *mut NSError| {
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(describe(error));
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, Some(&handler));
    }
    match receiver.await {
        Ok(None) => true,
        Ok(Some(error)) => {
            eprintln!(
                "claude-switch: could not post the {} notification for {}: {error}",
                alert.trigger.id(),
                alert.key
            );
            false
        }
        Err(_) => false,
    }
}

fn describe(error: *mut NSError) -> Option<String> {
    // SAFETY: the framework passes a valid error or none.
    unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string())
}

struct Ivars {
    app: AppHandle,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate`
    // doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "ClaudeSwitchNotificationDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Shows the banner even while claude-switch is the active app.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }

        /// A click on one of claude-switch's notifications opens the panel.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            // SAFETY: a constant the framework defines.
            let clicked = response
                .actionIdentifier()
                .isEqualToString(unsafe { UNNotificationDefaultActionIdentifier });
            let ours = response
                .notification()
                .request()
                .content()
                .threadIdentifier()
                .to_string()
                == THREAD;
            if clicked && ours {
                let app = self.ivars().app.clone();
                let _ = self
                    .ivars()
                    .app
                    .run_on_main_thread(move || tray::show_panel(&app));
            }
            handler.call(());
        }
    }
);

/// The framework holds its delegate weakly; this keeps it.
static DELEGATE: OnceLock<Retained<Delegate>> = OnceLock::new();

/// Makes clicks on claude-switch's notifications open the panel, and lets
/// banners show while claude-switch is the active app.
pub fn listen_for_clicks(app: &AppHandle) {
    if !bundled() {
        return;
    }
    let delegate = DELEGATE.get_or_init(|| {
        let this = Delegate::alloc().set_ivars(Ivars { app: app.clone() });
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
}
