//! App Nap prevention via `NSProcessInfo.beginActivity`.
//!
//! When another app holds focus during dictation, macOS marks the YapStack
//! main window occluded and throttles its WKWebView JS runtime under App Nap.
//! `await` continuations stall until the user refocuses — the bug users see
//! as "dictation processing hangs until I switch back to the main tab".
//!
//! Scoping an `NSActivityUserInitiated` activity across the dictation
//! lifetime keeps the process out of App Nap for those seconds. The activity
//! token is held in Tauri state; begin/end are balanced from the frontend.

use crate::commands::error::CommandError;

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2_foundation::{NSProcessInfo, NSString};
    use std::sync::Mutex;

    /// NSActivityOptions bitmask for user-initiated work. Matches
    /// `NSActivityUserInitiated` in `<Foundation/NSProcessInfo.h>`:
    /// disables App Nap, idle sleep, sudden termination, and automatic
    /// termination for the duration of the activity.
    const NS_ACTIVITY_USER_INITIATED: u64 = 0x00FFFFFF;

    /// Wrapper around the retained activity token. The underlying
    /// NSObject is safe to move across threads — Apple documents that
    /// `beginActivity:` / `endActivity:` are thread-safe and the returned
    /// token can be ended from any thread.
    struct ActivityToken(Retained<NSObject>);
    unsafe impl Send for ActivityToken {}
    unsafe impl Sync for ActivityToken {}

    pub struct AppNapState {
        token: Mutex<Option<ActivityToken>>,
    }

    impl AppNapState {
        pub fn new() -> Self {
            Self {
                token: Mutex::new(None),
            }
        }

        pub fn begin(&self, reason: &str) {
            let mut slot = self.token.lock().expect("AppNapState lock poisoned");
            if slot.is_some() {
                return;
            }
            let info = NSProcessInfo::processInfo();
            let ns_reason = NSString::from_str(reason);
            let token: Retained<NSObject> = unsafe {
                objc2::msg_send_id![
                    &*info,
                    beginActivityWithOptions: NS_ACTIVITY_USER_INITIATED,
                    reason: &*ns_reason,
                ]
            };
            *slot = Some(ActivityToken(token));
        }

        pub fn end(&self) {
            let mut slot = self.token.lock().expect("AppNapState lock poisoned");
            let Some(ActivityToken(token)) = slot.take() else { return };
            let info = NSProcessInfo::processInfo();
            unsafe {
                let _: () = objc2::msg_send![&*info, endActivity: &*token];
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::AppNapState;

#[cfg(not(target_os = "macos"))]
pub struct AppNapState;

#[cfg(not(target_os = "macos"))]
impl AppNapState {
    pub fn new() -> Self {
        Self
    }
    pub fn begin(&self, _reason: &str) {}
    pub fn end(&self) {}
}

pub type AppNapStateArc = std::sync::Arc<AppNapState>;

#[tauri::command]
#[specta::specta]
pub fn prevent_app_nap_begin(
    state: tauri::State<'_, AppNapStateArc>,
    reason: String,
) -> Result<(), CommandError> {
    state.begin(&reason);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn prevent_app_nap_end(state: tauri::State<'_, AppNapStateArc>) -> Result<(), CommandError> {
    state.end();
    Ok(())
}
