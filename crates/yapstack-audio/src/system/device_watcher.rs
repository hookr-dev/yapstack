//! Pushes a "default device changed" signal into our stream-health path via
//! a Core Audio property listener. Replaces the only-symptom-based cpal error
//! / write-pos watchdogs for the dominant mid-session silent-death mode on
//! macOS, where the default input or output device changes (AirPods connect,
//! USB mic unplug, Sound Settings toggle, exclusive-mode grab) and the cpal
//! stream is left bound to a device that no longer carries audio.
//!
//! The same primitive handles input and output — CoreAudio exposes them as
//! two property selectors on the same system object. Construct one
//! `DefaultDeviceWatcher` per direction.
//!
//! On non-macOS builds this is a no-op stub so call sites don't need
//! `#[cfg]` gates.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::AudioError;

/// Which default device a `DefaultDeviceWatcher` is observing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultDeviceKind {
    Input,
    Output,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use coreaudio_sys::{
        kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
        kAudioObjectSystemObject, noErr, AudioObjectAddPropertyListener, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectPropertySelector, AudioObjectRemovePropertyListener,
    };
    use std::ffi::c_void;
    use tracing::{info, warn};

    pub(super) struct WatcherInner {
        // Box'd to give the listener callback a stable pointer for the lifetime of
        // the watcher. The `Arc<AtomicBool>` is cloned into the box so the callback
        // can flip it without touching the watcher's public Arc.
        flag_box: *mut Arc<AtomicBool>,
        property: AudioObjectPropertyAddress,
    }

    // SAFETY: the `flag_box` pointer is only accessed from the Core Audio
    // listener thread via `listener_proc`, which reads an `Arc<AtomicBool>` and
    // stores to it atomically. The watcher's owning thread never dereferences
    // the raw pointer — only `Box::from_raw` on drop, after the listener is
    // unregistered.
    unsafe impl Send for WatcherInner {}
    unsafe impl Sync for WatcherInner {}

    unsafe extern "C" fn listener_proc(
        _object_id: AudioObjectID,
        _n_addresses: u32,
        _addresses: *const AudioObjectPropertyAddress,
        client_data: *mut c_void,
    ) -> i32 {
        if client_data.is_null() {
            return 0;
        }
        let flag = &*(client_data as *const Arc<AtomicBool>);
        flag.store(true, Ordering::Release);
        0 // noErr
    }

    fn selector_for(kind: DefaultDeviceKind) -> AudioObjectPropertySelector {
        match kind {
            DefaultDeviceKind::Input => kAudioHardwarePropertyDefaultInputDevice,
            DefaultDeviceKind::Output => kAudioHardwarePropertyDefaultOutputDevice,
        }
    }

    pub(super) fn register(
        kind: DefaultDeviceKind,
        flag: Arc<AtomicBool>,
    ) -> Result<WatcherInner, AudioError> {
        let property = AudioObjectPropertyAddress {
            mSelector: selector_for(kind),
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };

        // Box the Arc so the listener proc has a stable pointer for its lifetime.
        let flag_box: *mut Arc<AtomicBool> = Box::into_raw(Box::new(flag));

        // SAFETY: `flag_box` is a valid `*mut Arc<AtomicBool>` that outlives the
        // listener because we unregister the listener before dropping the box
        // in `Drop for WatcherInner`.
        let status = unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject,
                &property,
                Some(listener_proc),
                flag_box as *mut c_void,
            )
        };

        if status != noErr as i32 {
            // Reclaim the box so we don't leak on the error path.
            let _ = unsafe { Box::from_raw(flag_box) };
            warn!(
                "CoreAudio default-{:?} listener registration failed: OSStatus={}",
                kind, status
            );
            return Err(AudioError::PlatformNotSupported);
        }

        info!(
            "CoreAudio default-{:?} device change listener registered",
            kind
        );
        Ok(WatcherInner { flag_box, property })
    }

    impl Drop for WatcherInner {
        fn drop(&mut self) {
            // SAFETY: registration matched this exact property + client_data pair.
            let status = unsafe {
                AudioObjectRemovePropertyListener(
                    kAudioObjectSystemObject,
                    &self.property,
                    Some(listener_proc),
                    self.flag_box as *mut c_void,
                )
            };
            if status != noErr as i32 {
                warn!(
                    "CoreAudio default-device listener removal returned OSStatus={}",
                    status
                );
            }
            // SAFETY: the listener is unregistered; no thread can still be
            // reading through `flag_box`.
            let _ = unsafe { Box::from_raw(self.flag_box) };
        }
    }
}

/// Watches for default-device changes on macOS for a single direction
/// (input or output). On other platforms this is a no-op stub that always
/// reports "no change".
pub struct DefaultDeviceWatcher {
    flag: Arc<AtomicBool>,
    kind: DefaultDeviceKind,
    #[cfg(target_os = "macos")]
    _inner: imp::WatcherInner,
}

impl DefaultDeviceWatcher {
    pub fn new(kind: DefaultDeviceKind) -> Result<Self, AudioError> {
        let flag = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "macos")]
        {
            let inner = imp::register(kind, Arc::clone(&flag))?;
            Ok(Self {
                flag,
                kind,
                _inner: inner,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self { flag, kind })
        }
    }

    pub fn kind(&self) -> DefaultDeviceKind {
        self.kind
    }

    /// Atomically consumes the pending change flag. Returns `true` if the
    /// default device has changed since the last call to `take_change`.
    pub fn take_change(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_change_starts_false_for_output() {
        let w = DefaultDeviceWatcher::new(DefaultDeviceKind::Output).expect("watcher registers");
        assert!(!w.take_change());
    }

    #[test]
    fn take_change_starts_false_for_input() {
        let w = DefaultDeviceWatcher::new(DefaultDeviceKind::Input).expect("watcher registers");
        assert!(!w.take_change());
    }

    #[test]
    fn take_change_consumes_flag() {
        let w = DefaultDeviceWatcher::new(DefaultDeviceKind::Output).expect("watcher registers");
        w.flag.store(true, Ordering::Release);
        assert!(w.take_change());
        assert!(!w.take_change());
    }

    #[test]
    fn input_and_output_flags_are_independent() {
        let input = DefaultDeviceWatcher::new(DefaultDeviceKind::Input).expect("input registers");
        let output =
            DefaultDeviceWatcher::new(DefaultDeviceKind::Output).expect("output registers");
        output.flag.store(true, Ordering::Release);
        assert!(!input.take_change());
        assert!(output.take_change());
    }
}
