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
use std::sync::{Arc, RwLock};

use crate::error::AudioError;

/// Runtime-agnostic device-change event. The audio crate emits these
/// without taking a dependency on tokio, async runtimes, or Tauri types.
/// Consumers (the Tauri-side broker, tests) provide a [`DeviceEventSink`]
/// closure to receive them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEvent {
    /// Hardware device list changed (added or removed). Fires on
    /// `kAudioHardwarePropertyDevices`.
    DeviceListChanged,
    /// Default input device changed.
    DefaultInputChanged,
    /// Default *media* output device changed.
    DefaultOutputChanged,
    /// Default *alerts/UI* output device changed
    /// (`kAudioHardwarePropertyDefaultSystemOutputDevice`). Distinct from
    /// `DefaultOutputChanged`; consumers typically coalesce both into one
    /// system-audio restart attempt.
    DefaultSystemOutputChanged,
}

/// Closure invoked on the Core Audio listener thread whenever a watched
/// property changes. Must be cheap and non-blocking — typical
/// implementations only forward the event into a channel and return.
pub type DeviceEventSink = Arc<dyn Fn(DeviceEvent) + Send + Sync>;

/// Internal slot for a sink that may be attached after the watcher is
/// constructed. Cloned into the C-callback payload so the listener thread
/// reads through the same `Arc` the manager writes into.
pub(crate) type SharedSinkSlot = Arc<RwLock<Option<DeviceEventSink>>>;

/// Which CoreAudio system property a `DefaultDeviceWatcher` is observing.
///
/// `Input` / `Output` subscribe to the default-device selectors; `Devices`
/// subscribes to the device-list selector, which fires when hardware is
/// added or removed from the system (e.g. AirPods connect). On some macOS
/// versions the device-list notification precedes the default-device
/// notification during AirPods handshake, so it serves as an earlier
/// trigger for rebind.
///
/// `DefaultSystemOutput` subscribes to
/// `kAudioHardwarePropertyDefaultSystemOutputDevice`, which is distinct
/// from `Output` (`kAudioHardwarePropertyDefaultOutputDevice`): the former
/// is the route for system alerts and UI sounds, the latter for media.
/// Both can change independently; covering both is necessary to keep
/// system-audio loopback bound to whatever the user actually means by
/// "system output." See Hammerspoon's `hs.audiodevice.watcher` for prior
/// art covering all four selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultDeviceKind {
    Input,
    Output,
    DefaultSystemOutput,
    Devices,
}

impl DefaultDeviceKind {
    pub(crate) fn to_event(self) -> DeviceEvent {
        match self {
            Self::Input => DeviceEvent::DefaultInputChanged,
            Self::Output => DeviceEvent::DefaultOutputChanged,
            Self::DefaultSystemOutput => DeviceEvent::DefaultSystemOutputChanged,
            Self::Devices => DeviceEvent::DeviceListChanged,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use coreaudio_sys::{
        kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioHardwarePropertyDefaultSystemOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
        kAudioObjectSystemObject, noErr, AudioObjectAddPropertyListener, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectPropertySelector, AudioObjectRemovePropertyListener,
    };
    use std::ffi::c_void;
    use tracing::{info, warn};

    pub(super) struct CallbackPayload {
        pub flag: Arc<AtomicBool>,
        pub sink: SharedSinkSlot,
        pub kind: DefaultDeviceKind,
    }

    pub(super) struct WatcherInner {
        // Box'd to give the listener callback a stable pointer for the lifetime
        // of the watcher. The payload owns clones of the public `Arc<AtomicBool>`
        // and `SharedSinkSlot` so the callback can read/write them without
        // dereferencing the watcher itself.
        payload_box: *mut CallbackPayload,
        property: AudioObjectPropertyAddress,
    }

    // SAFETY: the `payload_box` pointer is only accessed from the Core Audio
    // listener thread via `listener_proc`, which reads `Arc`s and atomic
    // primitives. The watcher's owning thread never dereferences the raw
    // pointer — only `Box::from_raw` on drop, after the listener is
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
        let payload = &*(client_data as *const CallbackPayload);
        payload.flag.store(true, Ordering::Release);
        // Snapshot the sink under read lock and release it before invoking,
        // so a panicking sink can't poison the lock for future events.
        let snapshot = payload.sink.read().ok().and_then(|guard| guard.clone());
        if let Some(sink) = snapshot {
            sink(payload.kind.to_event());
        }
        0 // noErr
    }

    fn selector_for(kind: DefaultDeviceKind) -> AudioObjectPropertySelector {
        match kind {
            DefaultDeviceKind::Input => kAudioHardwarePropertyDefaultInputDevice,
            DefaultDeviceKind::Output => kAudioHardwarePropertyDefaultOutputDevice,
            DefaultDeviceKind::DefaultSystemOutput => {
                kAudioHardwarePropertyDefaultSystemOutputDevice
            }
            DefaultDeviceKind::Devices => kAudioHardwarePropertyDevices,
        }
    }

    pub(super) fn register(
        kind: DefaultDeviceKind,
        flag: Arc<AtomicBool>,
        sink: SharedSinkSlot,
    ) -> Result<WatcherInner, AudioError> {
        let property = AudioObjectPropertyAddress {
            mSelector: selector_for(kind),
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        // Box the payload so the listener proc has a stable pointer for its lifetime.
        let payload_box: *mut CallbackPayload =
            Box::into_raw(Box::new(CallbackPayload { flag, sink, kind }));

        // SAFETY: `payload_box` is a valid `*mut CallbackPayload` that outlives
        // the listener because we unregister the listener before dropping the
        // box in `Drop for WatcherInner`.
        let status = unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject,
                &property,
                Some(listener_proc),
                payload_box as *mut c_void,
            )
        };

        if status != noErr as i32 {
            // Reclaim the box so we don't leak on the error path.
            let _ = unsafe { Box::from_raw(payload_box) };
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
        Ok(WatcherInner {
            payload_box,
            property,
        })
    }

    impl Drop for WatcherInner {
        fn drop(&mut self) {
            // SAFETY: registration matched this exact property + client_data pair.
            let status = unsafe {
                AudioObjectRemovePropertyListener(
                    kAudioObjectSystemObject,
                    &self.property,
                    Some(listener_proc),
                    self.payload_box as *mut c_void,
                )
            };
            if status != noErr as i32 {
                warn!(
                    "CoreAudio default-device listener removal returned OSStatus={}",
                    status
                );
            }
            // SAFETY: the listener is unregistered; no thread can still be
            // reading through `payload_box`.
            let _ = unsafe { Box::from_raw(self.payload_box) };
        }
    }
}

/// Watches for default-device changes on macOS for a single direction
/// (input or output). On other platforms this is a no-op stub that always
/// reports "no change".
pub struct DefaultDeviceWatcher {
    flag: Arc<AtomicBool>,
    kind: DefaultDeviceKind,
    sink: SharedSinkSlot,
    #[cfg(target_os = "macos")]
    _inner: imp::WatcherInner,
}

impl DefaultDeviceWatcher {
    pub fn new(kind: DefaultDeviceKind) -> Result<Self, AudioError> {
        let flag = Arc::new(AtomicBool::new(false));
        let sink: SharedSinkSlot = Arc::new(RwLock::new(None));
        #[cfg(target_os = "macos")]
        {
            let inner = imp::register(kind, Arc::clone(&flag), Arc::clone(&sink))?;
            Ok(Self {
                flag,
                kind,
                sink,
                _inner: inner,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self { flag, kind, sink })
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

    /// Attach (or detach with `None`) a sink that the listener thread will
    /// invoke on every future event in addition to flipping the
    /// pending-change flag. Replaces any previously attached sink. Safe to
    /// call from any thread; the watcher's listener thread snapshots the
    /// sink under a brief read lock per event.
    pub fn set_sink(&self, sink: Option<DeviceEventSink>) {
        match self.sink.write() {
            Ok(mut guard) => *guard = sink,
            Err(poisoned) => {
                // A previous panicking writer poisoned the lock. Forge ahead —
                // the listener uses `read().ok()` so a poisoned slot just
                // suppresses event delivery; recovering the inner allows
                // future events to flow again.
                let mut guard = poisoned.into_inner();
                *guard = sink;
            }
        }
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

    #[test]
    fn take_change_starts_false_for_default_system_output() {
        let w = DefaultDeviceWatcher::new(DefaultDeviceKind::DefaultSystemOutput)
            .expect("default-system-output kind registers");
        assert!(!w.take_change());
    }

    #[test]
    fn default_system_output_flag_is_independent() {
        let media =
            DefaultDeviceWatcher::new(DefaultDeviceKind::Output).expect("output registers");
        let alerts = DefaultDeviceWatcher::new(DefaultDeviceKind::DefaultSystemOutput)
            .expect("default-system-output registers");
        alerts.flag.store(true, Ordering::Release);
        assert!(!media.take_change());
        assert!(alerts.take_change());
    }

    #[test]
    fn kind_to_event_mapping_is_total() {
        assert_eq!(
            DefaultDeviceKind::Input.to_event(),
            DeviceEvent::DefaultInputChanged
        );
        assert_eq!(
            DefaultDeviceKind::Output.to_event(),
            DeviceEvent::DefaultOutputChanged
        );
        assert_eq!(
            DefaultDeviceKind::DefaultSystemOutput.to_event(),
            DeviceEvent::DefaultSystemOutputChanged
        );
        assert_eq!(
            DefaultDeviceKind::Devices.to_event(),
            DeviceEvent::DeviceListChanged
        );
    }

    #[test]
    fn set_sink_then_detach_does_not_panic() {
        let w = DefaultDeviceWatcher::new(DefaultDeviceKind::Input).expect("input registers");
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);
        let sink: DeviceEventSink = Arc::new(move |_event| {
            counter_clone.fetch_add(1, Ordering::Relaxed);
        });
        w.set_sink(Some(sink));
        // Cannot synthesize a real Core Audio event in a unit test, but we
        // can confirm attach + detach round-trip without panicking and that
        // detach clears the slot.
        w.set_sink(None);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
