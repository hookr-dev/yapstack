//! System output volume control for the dictation duck feature.
//!
//! While the user is actively recording a dictation, we lower the system
//! output volume to a configured target so they can hear themselves and
//! anyone trying to talk over them. The prior level is snapshotted at duck
//! time and restored when recording ends (or on app exit as a safety net).
//!
//! Only ever lowers — never raises. If the current volume is already at or
//! below the target, `apply_duck` is a no-op and no snapshot is captured,
//! so a subsequent `restore` won't accidentally raise the user's volume.
//!
//! The snapshot tracks the **device id** alongside the level. If the user
//! switches default output mid-dictation (AirPods connect, USB DAC unplug),
//! `restore` targets the *original* device that was ducked rather than the
//! new default — otherwise we'd leak ducked state on the original device
//! and clobber the new device's volume.
//!
//! macOS uses `kAudioHardwareServiceDeviceProperty_VirtualMainVolume` (FourCC
//! 'vmvc'), the same property the menu-bar slider drives. It's the modern,
//! device-agnostic way to set "the system volume": works on built-in
//! speakers, AirPods, USB DACs, and aggregate devices, including those whose
//! hardware doesn't expose a true master scalar on the main element.
//!
//! Windows / Linux are stubs that return `Unsupported`; future ports plug in
//! against the same `SystemOutputVolume` trait.

use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum VolumeError {
    #[error("system output volume control is not supported on this platform")]
    Unsupported,
    #[cfg(target_os = "macos")]
    #[error("CoreAudio call failed: OSStatus={0}")]
    CoreAudio(i32),
}

/// Opaque identifier for an output device. On macOS this is an
/// `AudioObjectID`; on stub platforms it's a placeholder that's never read.
pub type DeviceId = u32;

pub trait SystemOutputVolume: Send + Sync {
    /// Resolve the current default output device. Returns `Unsupported` on
    /// platforms or system states without a usable default (e.g. a headless
    /// mac where `kAudioHardwarePropertyDefaultOutputDevice` resolves to
    /// `kAudioObjectUnknown` / 0).
    fn default_device(&self) -> Result<DeviceId, VolumeError>;
    /// Read the volume of `device` in [0.0, 1.0].
    fn get(&self, device: DeviceId) -> Result<f32, VolumeError>;
    /// Set `device`'s volume. `level` is clamped to [0.0, 1.0].
    fn set(&self, device: DeviceId, level: f32) -> Result<(), VolumeError>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DuckOutcome {
    /// Volume on `device` was lowered from `from` to `to`, and `(device, from)`
    /// is now snapshotted.
    Applied {
        device: DeviceId,
        from: f32,
        to: f32,
    },
    /// Current volume was already at or below the target — no change made,
    /// no snapshot captured.
    Skipped { current: f32, target: f32 },
    /// A snapshot was already held from a prior `apply_duck`; we re-applied
    /// the target without overwriting the original snapshot.
    AlreadyDucked { target: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Snapshot {
    device: DeviceId,
    level: f32,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

fn controller() -> Box<dyn SystemOutputVolume> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::CoreAudioController)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(stub::StubController)
    }
}

/// Duck the system volume to `target` (in 0.0..=1.0) if and only if the
/// current volume on the default output device is above `target`. Snapshots
/// the prior `(device, level)` pair so `restore` puts that exact device back,
/// even if the user changes default output during the duck.
pub fn apply_duck(target: f32) -> Result<DuckOutcome, VolumeError> {
    apply_duck_inner(&SNAPSHOT, controller().as_ref(), target)
}

fn apply_duck_inner(
    snap_cell: &Mutex<Option<Snapshot>>,
    ctrl: &dyn SystemOutputVolume,
    target: f32,
) -> Result<DuckOutcome, VolumeError> {
    let target = target.clamp(0.0, 1.0);
    let device = ctrl.default_device()?;
    let current = ctrl.get(device)?;

    // Hold the snapshot mutex across BOTH the snapshot write AND the volume
    // set. Without this, a concurrent `restore_inner` call landing between
    // "publish snapshot" and "lower volume" could clear the snapshot and
    // restore the original level, after which our `set(target)` would land
    // with no snapshot left to recover from — leaving the user ducked with
    // no way to undo it. Trade: `ctrl.set` runs under the mutex (one
    // CoreAudio call, typically sub-millisecond on macOS), in exchange for
    // strict apply/restore atomicity.
    let mut snap = snap_cell.lock().expect("system_volume snapshot poisoned");
    if let Some(snapped) = *snap {
        // Already ducked — re-apply target on the *originally* ducked
        // device (held in the snapshot), but don't clobber the snapshot.
        // Only set if we'd actually be lowering; never raise.
        if ctrl.get(snapped.device)? > target {
            ctrl.set(snapped.device, target)?;
        }
        return Ok(DuckOutcome::AlreadyDucked { target });
    }

    if current <= target {
        return Ok(DuckOutcome::Skipped { current, target });
    }

    // Write snapshot first, then apply. If the set call fails, the snapshot
    // is rolled back so a later restore can't try to "recover" to a level
    // we never actually reached.
    *snap = Some(Snapshot {
        device,
        level: current,
    });
    if let Err(e) = ctrl.set(device, target) {
        *snap = None;
        return Err(e);
    }
    Ok(DuckOutcome::Applied {
        device,
        from: current,
        to: target,
    })
}

/// Restore the snapshotted volume on the snapshotted device, if any. No-op
/// if `apply_duck` was never called or was a Skipped no-op. Clears the
/// snapshot regardless of whether the underlying `set` call succeeds — a
/// stale snapshot for a vanished device is worse than a one-off log line.
pub fn restore() -> Result<(), VolumeError> {
    restore_inner(&SNAPSHOT, controller().as_ref())
}

fn restore_inner(
    snap_cell: &Mutex<Option<Snapshot>>,
    ctrl: &dyn SystemOutputVolume,
) -> Result<(), VolumeError> {
    // Hold the mutex across both the snapshot read+clear AND the volume
    // set, mirroring `apply_duck_inner`. Together they form a serialized
    // critical section: an apply running concurrently with this restore
    // either fully precedes us (sees no snapshot, takes one, sets target —
    // we then no-op) or fully follows us (we set back to original, then
    // apply takes a fresh snapshot of the restored level and ducks again).
    // Never an interleaved state where we've cleared the snapshot mid-apply.
    let mut snap = snap_cell.lock().expect("system_volume snapshot poisoned");
    let Some(snapped) = snap.take() else {
        return Ok(());
    };
    ctrl.set(snapped.device, snapped.level)
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{DeviceId, SystemOutputVolume, VolumeError};
    use coreaudio_sys::{
        kAudioDevicePropertyScopeOutput, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
        noErr, AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress,
        AudioObjectSetPropertyData,
    };
    use std::mem;

    /// FourCC `'vmvc'` for `kAudioHardwareServiceDeviceProperty_VirtualMainVolume`.
    /// Not exposed by coreaudio-sys (it lives in AudioToolbox's AudioServices
    /// header rather than the AudioHardware ones bindgen pulls in) so we
    /// inline the selector value. Apple's deprecation note since 10.9 says
    /// to call AudioObjectGetPropertyData / SetPropertyData directly with
    /// this selector, which is what we do.
    const K_VIRTUAL_MAIN_VOLUME: u32 = 0x766d7663;

    pub struct CoreAudioController;

    fn volume_address() -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: K_VIRTUAL_MAIN_VOLUME,
            mScope: kAudioDevicePropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    impl SystemOutputVolume for CoreAudioController {
        fn default_device(&self) -> Result<DeviceId, VolumeError> {
            let address = AudioObjectPropertyAddress {
                mSelector: kAudioHardwarePropertyDefaultOutputDevice,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: kAudioObjectPropertyElementMain,
            };
            let mut device_id: AudioObjectID = 0;
            let mut size = mem::size_of::<AudioObjectID>() as u32;
            // SAFETY: stack out param, matching size; CoreAudio writes at
            // most `size` bytes to `device_id`.
            let status = unsafe {
                AudioObjectGetPropertyData(
                    kAudioObjectSystemObject,
                    &address,
                    0,
                    std::ptr::null(),
                    &mut size,
                    &mut device_id as *mut _ as *mut _,
                )
            };
            if status != noErr as i32 {
                return Err(VolumeError::CoreAudio(status));
            }
            // `kAudioObjectUnknown` (id 0) is what CoreAudio returns when no
            // default output device exists — e.g. a headless mac with no
            // built-in or attached audio. Surface this as `Unsupported`
            // rather than letting it propagate into a `!obj` error on the
            // next AudioObject call.
            if device_id == 0 {
                return Err(VolumeError::Unsupported);
            }
            Ok(device_id as DeviceId)
        }

        fn get(&self, device: DeviceId) -> Result<f32, VolumeError> {
            let address = volume_address();
            let mut value: f32 = 0.0;
            let mut size = mem::size_of::<f32>() as u32;
            // SAFETY: stack out param, matching size.
            let status = unsafe {
                AudioObjectGetPropertyData(
                    device as AudioObjectID,
                    &address,
                    0,
                    std::ptr::null(),
                    &mut size,
                    &mut value as *mut _ as *mut _,
                )
            };
            if status != noErr as i32 {
                return Err(VolumeError::CoreAudio(status));
            }
            Ok(value.clamp(0.0, 1.0))
        }

        fn set(&self, device: DeviceId, level: f32) -> Result<(), VolumeError> {
            let address = volume_address();
            let value: f32 = level.clamp(0.0, 1.0);
            let size = mem::size_of::<f32>() as u32;
            // SAFETY: read-only ref to stack `value`, matching size.
            let status = unsafe {
                AudioObjectSetPropertyData(
                    device as AudioObjectID,
                    &address,
                    0,
                    std::ptr::null(),
                    size,
                    &value as *const _ as *const _,
                )
            };
            if status != noErr as i32 {
                return Err(VolumeError::CoreAudio(status));
            }
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod stub {
    use super::{DeviceId, SystemOutputVolume, VolumeError};

    pub struct StubController;

    impl SystemOutputVolume for StubController {
        fn default_device(&self) -> Result<DeviceId, VolumeError> {
            Err(VolumeError::Unsupported)
        }
        fn get(&self, _device: DeviceId) -> Result<f32, VolumeError> {
            Err(VolumeError::Unsupported)
        }
        fn set(&self, _device: DeviceId, _level: f32) -> Result<(), VolumeError> {
            Err(VolumeError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // ---- Mock controller: drives the inner functions without real hardware. ----

    struct MockController {
        default: Mutex<DeviceId>,
        volumes: Mutex<HashMap<DeviceId, f32>>,
        /// Optional sleep injected into `set` so concurrent tests can
        /// deterministically widen the apply/restore race window without
        /// relying on probabilistic interleavings.
        set_delay: std::time::Duration,
    }

    impl MockController {
        fn with_devices(devices: &[(DeviceId, f32)], default: DeviceId) -> Self {
            let mut volumes = HashMap::new();
            for (id, vol) in devices {
                volumes.insert(*id, *vol);
            }
            Self {
                default: Mutex::new(default),
                volumes: Mutex::new(volumes),
                set_delay: std::time::Duration::ZERO,
            }
        }

        fn with_set_delay(mut self, delay: std::time::Duration) -> Self {
            self.set_delay = delay;
            self
        }

        fn set_default(&self, id: DeviceId) {
            *self.default.lock().unwrap() = id;
        }

        fn snapshot_of(&self, device: DeviceId) -> f32 {
            *self.volumes.lock().unwrap().get(&device).unwrap()
        }
    }

    impl SystemOutputVolume for MockController {
        fn default_device(&self) -> Result<DeviceId, VolumeError> {
            Ok(*self.default.lock().unwrap())
        }
        fn get(&self, device: DeviceId) -> Result<f32, VolumeError> {
            self.volumes
                .lock()
                .unwrap()
                .get(&device)
                .copied()
                .ok_or(VolumeError::Unsupported)
        }
        fn set(&self, device: DeviceId, level: f32) -> Result<(), VolumeError> {
            if !self.set_delay.is_zero() {
                std::thread::sleep(self.set_delay);
            }
            self.volumes
                .lock()
                .unwrap()
                .insert(device, level.clamp(0.0, 1.0));
            Ok(())
        }
    }

    fn fresh_snap() -> Mutex<Option<Snapshot>> {
        Mutex::new(None)
    }

    #[test]
    fn skipped_when_current_at_or_below_target() {
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.15)], 1);

        let outcome = apply_duck_inner(&snap, &ctrl, 0.20).expect("apply_duck");
        assert!(matches!(outcome, DuckOutcome::Skipped { .. }));
        assert!(snap.lock().unwrap().is_none(), "no snapshot when skipped");
        assert_eq!(ctrl.snapshot_of(1), 0.15, "volume unchanged");
    }

    #[test]
    fn applied_lowers_and_snapshots() {
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.80)], 1);

        let outcome = apply_duck_inner(&snap, &ctrl, 0.20).expect("apply_duck");
        match outcome {
            DuckOutcome::Applied { device, from, to } => {
                assert_eq!(device, 1);
                assert!((from - 0.80).abs() < 1e-6);
                assert!((to - 0.20).abs() < 1e-6);
            }
            other => panic!("expected Applied, got {other:?}"),
        }
        assert_eq!(ctrl.snapshot_of(1), 0.20, "device 1 ducked to target");
        let s = snap.lock().unwrap().expect("snapshot present");
        assert_eq!(s.device, 1);
        assert!((s.level - 0.80).abs() < 1e-6);
    }

    #[test]
    fn restore_recovers_snapshotted_level_and_clears_state() {
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.80)], 1);

        apply_duck_inner(&snap, &ctrl, 0.20).expect("apply");
        restore_inner(&snap, &ctrl).expect("restore");

        assert_eq!(ctrl.snapshot_of(1), 0.80, "restored to original level");
        assert!(snap.lock().unwrap().is_none(), "snapshot cleared");
    }

    #[test]
    fn restore_targets_original_device_after_default_switch() {
        // Real-world: built-in speakers ducked, then AirPods connect and
        // become default. Restore must put speakers back, not AirPods.
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.80), (2, 0.50)], 1);

        apply_duck_inner(&snap, &ctrl, 0.20).expect("apply on device 1");
        assert_eq!(ctrl.snapshot_of(1), 0.20);
        assert_eq!(ctrl.snapshot_of(2), 0.50);

        // User connects AirPods mid-dictation → default switches.
        ctrl.set_default(2);

        restore_inner(&snap, &ctrl).expect("restore");

        assert_eq!(
            ctrl.snapshot_of(1),
            0.80,
            "original device restored to its prior level"
        );
        assert_eq!(
            ctrl.snapshot_of(2),
            0.50,
            "the new default's volume is untouched by restore"
        );
    }

    #[test]
    fn restore_is_noop_when_no_snapshot_held() {
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.50)], 1);

        restore_inner(&snap, &ctrl).expect("restore should noop");
        assert_eq!(ctrl.snapshot_of(1), 0.50);
    }

    #[test]
    fn already_ducked_does_not_clobber_snapshot() {
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.80)], 1);

        apply_duck_inner(&snap, &ctrl, 0.20).expect("first apply");
        // A re-entrant duck call (e.g. retry) must not capture the now-ducked
        // 0.20 as a new snapshot, or restore would leave the user at 0.20.
        let outcome = apply_duck_inner(&snap, &ctrl, 0.20).expect("second apply");
        assert!(matches!(outcome, DuckOutcome::AlreadyDucked { .. }));

        restore_inner(&snap, &ctrl).expect("restore");
        assert_eq!(
            ctrl.snapshot_of(1),
            0.80,
            "restore returns to the *original* prior level"
        );
    }

    #[test]
    fn duck_skipped_leaves_no_snapshot_to_restore() {
        // Regression guard: a Skipped duck must never plant a snapshot that
        // would cause restore to set anything.
        let snap = fresh_snap();
        let ctrl = MockController::with_devices(&[(1, 0.10)], 1);

        apply_duck_inner(&snap, &ctrl, 0.20).expect("apply");
        restore_inner(&snap, &ctrl).expect("restore");
        assert_eq!(ctrl.snapshot_of(1), 0.10);
    }

    #[test]
    fn apply_and_restore_are_atomic_under_contention() {
        // Regression test for the apply/restore race: with the snapshot
        // mutex held only across `is_some / *snap = ...` and *not* across
        // the volume `set` call, a concurrent restore can land between the
        // two and clear the snapshot before set runs. After both complete
        // the volume is at the duck target with no snapshot — i.e. the
        // user is permanently ducked with no way to recover.
        //
        // We use a 50ms sleep injected into MockController::set to widen
        // the window deterministically: thread A starts apply, holds the
        // mutex past its set; the main thread fires restore; if the impl
        // releases the mutex before set, the test will show vol=0.20 with
        // snap=None (broken). With the mutex held across set, restore must
        // wait, and we observe one of the two consistent post-states.
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let snap = Arc::new(Mutex::new(None::<Snapshot>));
        let ctrl = Arc::new(
            MockController::with_devices(&[(1, 0.80)], 1).with_set_delay(Duration::from_millis(50)),
        );

        let snap_a = Arc::clone(&snap);
        let ctrl_a = Arc::clone(&ctrl);
        let apply_handle =
            thread::spawn(move || apply_duck_inner(snap_a.as_ref(), ctrl_a.as_ref(), 0.20));

        // Give thread A enough lead time to acquire the snapshot mutex
        // before the restore call below races to grab it.
        thread::sleep(Duration::from_millis(10));

        restore_inner(snap.as_ref(), ctrl.as_ref()).expect("restore");
        apply_handle
            .join()
            .expect("apply thread")
            .expect("apply ok");

        let vol = ctrl.snapshot_of(1);
        let snap_final = snap.lock().unwrap();

        // Consistent post-states only — no broken interleave.
        let consistent = match snap_final.is_some() {
            true => (vol - 0.20).abs() < 1e-3,  // apply won outright
            false => (vol - 0.80).abs() < 1e-3, // apply→restore both ran in order
        };
        assert!(
            consistent,
            "inconsistent post-state: vol={vol} snap={:?}",
            *snap_final
        );
    }

    // ---- Hardware-touching tests. Mutate the user's actual system volume,
    //      so they're #[ignore]d by default. Run on demand with:
    //          cargo test -p yapstack-desktop --lib system_volume:: -- --ignored
    //      from a developer's macOS box that has audio output.
    //      They also fail on a headless mac (no default output device);
    //      `default_device()` returns `Unsupported` in that case, which is
    //      correct behavior and not a test failure we want gating CI. ----

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches real system audio; run with --ignored"]
    fn hardware_get_returns_unit_interval() {
        let ctrl = controller();
        let device = ctrl.default_device().expect("default device");
        let v = ctrl.get(device).expect("get system volume");
        assert!((0.0..=1.0).contains(&v), "volume out of range: {v}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches real system audio; run with --ignored"]
    fn hardware_apply_duck_lowers_and_restore_recovers() {
        let snap = fresh_snap();
        let ctrl = controller();
        let device = ctrl.default_device().expect("default device");
        let original = ctrl.get(device).expect("get original");

        // Force a known starting point well above any plausible duck target.
        let start = 0.6_f32;
        ctrl.set(device, start).expect("set start");
        let observed_start = ctrl.get(device).expect("get observed start");

        let target = 0.2_f32;
        apply_duck_inner(&snap, ctrl.as_ref(), target).expect("apply_duck");
        let after_duck = ctrl.get(device).expect("get after duck");
        assert!(
            after_duck <= target + 0.05,
            "expected duck near {target}, got {after_duck}"
        );

        restore_inner(&snap, ctrl.as_ref()).expect("restore");
        let after_restore = ctrl.get(device).expect("get after restore");
        assert!(
            (after_restore - observed_start).abs() < 0.05,
            "expected restore near {observed_start}, got {after_restore}"
        );

        // Best-effort cleanup.
        let _ = ctrl.set(device, original);
    }
}
