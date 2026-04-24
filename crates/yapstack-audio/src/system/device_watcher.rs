//! Pushes a "default device changed" signal into our stream-health path via
//! a Core Audio property listener. Replaces the only-symptom-based cpal error
//! / write-pos watchdogs for the dominant mid-session silent-death mode on
//! macOS, where the default input or output device changes (AirPods connect,
//! USB mic unplug, Sound Settings toggle, exclusive-mode grab) and the cpal
//! stream is left bound to a device that no longer carries audio.
//!
//! The same primitive handles input, output, and device-list (hardware
//! added/removed) via three property selectors on the same system object.
//! Construct one `DefaultDeviceWatcher` per kind.
//!
//! On non-macOS builds this is a no-op stub so call sites don't need
//! `#[cfg]` gates.
//!
//! # Why we do this in-house instead of relying on cpal
//!
//! cpal 0.17.x (and as of this writing, cpal master too) **does not
//! automatically reroute a stream when the default device changes**. The
//! stream stays bound to the device it was created against and silently
//! produces either nothing (output loopback when the underlying device is
//! no longer actively playing) or zero-filled callbacks (input). Upstream
//! tracking:
//!
//! - [`cpal#1175`](https://github.com/RustAudio/cpal/issues/1175) — "default
//!   devices don't get automatically rerouted upon disconnection"
//!   (confirmed by maintainer 2026-04-22). Filed against the newer
//!   `AudioHardwareCreateProcessTap` path from
//!   [PR #1003](https://github.com/RustAudio/cpal/pull/1003), so upgrading
//!   cpal does *not* fix this.
//! - [`cpal#704`](https://github.com/RustAudio/cpal/issues/704),
//!   [`cpal#1012`](https://github.com/RustAudio/cpal/issues/1012),
//!   [`cpal#1030`](https://github.com/RustAudio/cpal/issues/1030) — related
//!   older reports of silent fallback on input disconnect and loopback
//!   edge cases.
//!
//! **If cpal ever lands a `DeviceChanged` error-callback variant or native
//! auto-rerouting**, reassess whether this watcher is still needed. Until
//! then, this is the authoritative signal; cpal's error callback is a
//! backup (Layer 1) and `write_pos` stall detection is a second backup
//! (Layer 2).
//!
//! # Known edge case: default-output "revert" during AirPods handshake
//!
//! On AirPods / Bluetooth output connect, macOS briefly reports the *old*
//! device as default for a window of ~100-200 ms while the AVAudioEngine
//! aggregate is being set up (cf. AirPods + AVAudioEngine
//! [notes](https://supermegaultragroovy.com/2021/01/28/more-on-avaudioengine-airpods/)
//! and Apple [DF thread 763583](https://developer.apple.com/forums/thread/763583)).
//! A naive listener-driven restart that re-queries `default_output_device()`
//! during that revert window binds back to the same dead device.
//!
//! Our workaround in the health-check path:
//! 1. Sleep ~200 ms after the listener fires to let macOS settle.
//! 2. Re-query the current default and compare to the bound device. If
//!    unchanged, treat the listener event as spurious (don't restart).
//! 3. If the restart does rebind to the same device anyway
//!    ([`RestartReport::same_device`](crate::manager::RestartReport)),
//!    skip the cooldown and retry on the next poll; cap at
//!    `STREAM_RESTART_MAX_ATTEMPTS`.
//!
//! We also subscribe to `kAudioHardwarePropertyDevices` (device-list
//! change) because it fires earlier than the default-output property on
//! some macOS versions during Bluetooth handshake.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::AudioError;

/// Which CoreAudio system property a `DefaultDeviceWatcher` is observing.
///
/// `Input` / `Output` subscribe to the default-device selectors; `Devices`
/// subscribes to the device-list selector, which fires when hardware is
/// added or removed from the system (e.g. AirPods connect). On some macOS
/// versions the device-list notification precedes the default-device
/// notification during AirPods handshake, so it serves as an earlier
/// trigger for rebind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultDeviceKind {
    Input,
    Output,
    Devices,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use coreaudio_sys::{
        kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMaster,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, noErr,
        AudioObjectAddPropertyListener, AudioObjectID, AudioObjectPropertyAddress,
        AudioObjectPropertySelector, AudioObjectRemovePropertyListener,
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
            DefaultDeviceKind::Devices => kAudioHardwarePropertyDevices,
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

    #[test]
    fn take_change_starts_false_for_devices() {
        let w =
            DefaultDeviceWatcher::new(DefaultDeviceKind::Devices).expect("devices kind registers");
        assert!(!w.take_change());
    }

    #[test]
    fn devices_flag_is_independent_from_default_flags() {
        let default_out =
            DefaultDeviceWatcher::new(DefaultDeviceKind::Output).expect("output registers");
        let devices =
            DefaultDeviceWatcher::new(DefaultDeviceKind::Devices).expect("devices registers");
        devices.flag.store(true, Ordering::Release);
        assert!(!default_out.take_change());
        assert!(devices.take_change());
    }
}
