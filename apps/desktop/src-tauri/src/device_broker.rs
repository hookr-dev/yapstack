//! Always-on device-change broker.
//!
//! Owns the receiving end of the audio crate's runtime-agnostic
//! [`DeviceEventSink`]. Lives in the Tauri layer (not the audio crate)
//! because it needs `tokio` and an [`AppHandle`] to emit events to the
//! frontend — the audio crate is intentionally synchronous and
//! runtime-agnostic.
//!
//! Spawned once during Tauri `setup()` and runs for the app's lifetime.
//! Reacts to:
//!
//! * `kAudioHardwarePropertyDevices` (hardware add/remove) →
//!   re-enumerate, emit `devices-changed`.
//! * Default-input / default-output / default-system-output changes →
//!   emit one `default-device-changed` per kind that fired within the
//!   debounce window.
//!
//! Events arriving in a 250 ms window are coalesced — a bluetooth
//! handshake typically fires `Devices`, then `DefaultOutput`, then
//! sometimes `DefaultSystemOutput` in rapid succession; the broker
//! emits one consolidated set of FE events at the trailing edge.
//!
//! Phase 4: emits FE events. Phase 6 adds restart routing.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tracing::{debug, info, warn};
use yapstack_audio::system::device_watcher::{DeviceEvent, DeviceEventSink};

use crate::commands::audio::AudioDeviceInfoDto;
use crate::commands::audio::AudioManagerState;
use crate::commands::live_transcription::{RestartIntent, RestartIntentInbox};

/// Coalescing window for bursty Core Audio listener events. 250 ms is
/// short enough that the FE sees device changes almost instantly and
/// long enough to absorb the typical bluetooth-handshake event burst
/// (Devices → DefaultOutput → DefaultSystemOutput within ~50 ms).
const DEBOUNCE_MS: u64 = 250;

/// Which OS default the [`DefaultDeviceChangedPayload`] describes.
/// `Output` is the media route; `SystemOutput` is the alerts/UI route
/// (`kAudioHardwarePropertyDefaultSystemOutputDevice`). They can change
/// independently on macOS.
#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultDeviceKindDto {
    Input,
    Output,
    SystemOutput,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DefaultDeviceChangedPayload {
    pub kind: DefaultDeviceKindDto,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
}

/// Spawn the device-change broker on the Tauri async runtime. Returns
/// immediately; the broker runs until the shutdown signal flips to
/// `true` or the audio manager drops its watchers.
pub fn spawn(
    app_handle: AppHandle,
    audio_state: AudioManagerState,
    restart_intent_inbox: RestartIntentInbox,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    tauri::async_runtime::spawn(async move {
        let (tx, mut rx) = mpsc::unbounded_channel::<DeviceEvent>();

        // Build the sink. The closure runs on the Core Audio listener
        // thread; it must not block or .await, so it only forwards into
        // the unbounded channel and returns.
        let sink_tx = tx.clone();
        let sink: DeviceEventSink = Arc::new(move |event| {
            let _ = sink_tx.send(event);
        });
        {
            let manager = audio_state.lock().await;
            manager.subscribe_device_events(Some(sink));
        }
        drop(tx);

        info!("device broker: started");

        loop {
            tokio::select! {
                maybe_event = rx.recv() => match maybe_event {
                    Some(first_event) => {
                        let mut state = CollapsedEvents::default();
                        state.merge(first_event);

                        // Drain any events that arrive within the
                        // debounce window before deciding what to emit.
                        let deadline = Instant::now() + Duration::from_millis(DEBOUNCE_MS);
                        loop {
                            match tokio::time::timeout_at(deadline, rx.recv()).await {
                                Ok(Some(next)) => state.merge(next),
                                Ok(None) => {
                                    // Sink dropped — handled by the outer
                                    // loop; flush what we have first.
                                    debug!("device broker: channel closed mid-debounce");
                                    break;
                                }
                                Err(_) => break, // window elapsed
                            }
                        }

                        flush(&app_handle, &audio_state, &restart_intent_inbox, state).await;
                    }
                    None => {
                        info!("device broker: sink detached, exiting");
                        break;
                    }
                },
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        let manager = audio_state.lock().await;
                        manager.subscribe_device_events(None);
                        info!("device broker: shutdown signal received, exiting");
                        break;
                    }
                }
            }
        }
    });
}

/// Tracks which kinds of events fired during a debounce window. Booleans
/// rather than counts because emitting the same FE event twice for the
/// same window adds no signal — the FE re-reads OS state on receipt.
#[derive(Default, Debug)]
struct CollapsedEvents {
    device_list: bool,
    default_input: bool,
    default_output: bool,
    default_system_output: bool,
}

impl CollapsedEvents {
    fn merge(&mut self, event: DeviceEvent) {
        match event {
            DeviceEvent::DeviceListChanged => self.device_list = true,
            DeviceEvent::DefaultInputChanged => self.default_input = true,
            DeviceEvent::DefaultOutputChanged => self.default_output = true,
            DeviceEvent::DefaultSystemOutputChanged => self.default_system_output = true,
        }
    }
}

async fn flush(
    app_handle: &AppHandle,
    audio_state: &AudioManagerState,
    inbox: &RestartIntentInbox,
    state: CollapsedEvents,
) {
    // FE event emission — distinct kinds so the UI can show what
    // actually changed.
    if state.device_list {
        emit_devices_changed(app_handle);
    }
    if state.default_input {
        emit_default_changed(app_handle, DefaultDeviceKindDto::Input);
    }
    if state.default_output {
        emit_default_changed(app_handle, DefaultDeviceKindDto::Output);
    }
    if state.default_system_output {
        emit_default_changed(app_handle, DefaultDeviceKindDto::SystemOutput);
    }

    // Restart dispatch — Output and SystemOutput coalesce into one
    // System restart. Each dispatch is `IsAlive`-gated to absorb the
    // AirPods/Bluetooth revert window.
    if state.default_input {
        dispatch_mic_restart(audio_state, inbox).await;
    }
    if state.default_output || state.default_system_output {
        dispatch_system_restart(audio_state, inbox).await;
    }
}

/// Resolve the current default-input UID, gate on `is_device_alive`
/// (one re-check at +250 ms if the device isn't yet alive), then route
/// the restart through the live-transcription inbox if a session is
/// running, or call `AudioManager::restart_mic` directly if a non-live
/// Capture is active.
///
/// Skips entirely when the user explicitly picked a non-default Mic
/// that is still alive — `DefaultInputChanged` only matters for that
/// user when their device disappears; that case arrives as a
/// `DeviceListChanged` instead and is handled by FE reconciliation
/// resetting `selectedMicDeviceId` to follow-default. No-op when no
/// Capture is active.
async fn dispatch_mic_restart(audio_state: &AudioManagerState, inbox: &RestartIntentInbox) {
    // Snapshot bound state under the lock; release before doing any
    // sleep/gate work so we don't block other async actors.
    let (bound_is_default, bound_uid, mic_active) = {
        let manager = audio_state.lock().await;
        let status = manager.status();
        (
            manager.mic_bound_is_default(),
            manager.mic_device_id().map(|s| s.to_string()),
            status.mic_active,
        )
    };

    if !bound_is_default {
        // Explicit pick. Honor it as long as the device is still alive.
        let still_alive = bound_uid
            .as_deref()
            .map(strip_cpal_prefix)
            .map(yapstack_audio::device::is_device_alive)
            .unwrap_or(false);
        if still_alive {
            debug!(
                "device broker: explicit Mic ({:?}) still alive — ignoring DefaultInputChanged",
                bound_uid
            );
            return;
        }
    }

    let default_uid = yapstack_audio::device::default_input_device()
        .ok()
        .and_then(|info| info.id);
    alive_gate(default_uid.as_deref()).await;

    let sent = try_send_intent(inbox, RestartIntent::Mic);
    if sent {
        debug!("device broker: routed Mic restart to live loop");
        return;
    }

    if !mic_active {
        debug!("device broker: no live loop and Mic not active — skipping restart");
        return;
    }
    let mut manager = audio_state.lock().await;
    match manager.restart_mic() {
        Ok(report) => info!(
            "device broker: direct Mic restart ({:?}, bound={:?})",
            report.outcome, report.bound_device_name
        ),
        Err(e) => warn!("device broker: direct Mic restart failed: {}", e),
    }
}

async fn dispatch_system_restart(audio_state: &AudioManagerState, inbox: &RestartIntentInbox) {
    let default_uid = yapstack_audio::device::default_output_device()
        .ok()
        .and_then(|info| info.id);
    alive_gate(default_uid.as_deref()).await;

    let sent = try_send_intent(inbox, RestartIntent::System);
    if sent {
        debug!("device broker: routed System restart to live loop");
        return;
    }

    let mut manager = audio_state.lock().await;
    let status = manager.status();
    if !status.system_audio_active {
        debug!("device broker: no live loop and System audio not active — skipping restart");
        return;
    }
    match manager.restart_system_audio() {
        Ok(report) => info!(
            "device broker: direct System restart ({:?}, bound={:?})",
            report.outcome, report.bound_device_name
        ),
        Err(e) => warn!("device broker: direct System restart failed: {}", e),
    }
}

/// Best-effort send into the live-loop's restart-intent inbox. Returns
/// `true` if a sender was present and the send queued; `false` if no
/// session is running or the receiver has been dropped (caller falls
/// back to `AudioManager::restart_*`).
fn try_send_intent(inbox: &RestartIntentInbox, intent: RestartIntent) -> bool {
    let guard = match inbox.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            warn!("device broker: restart-intent inbox poisoned — recovering");
            poisoned.into_inner()
        }
    };
    match guard.as_ref() {
        Some(sender) => sender.send(intent).is_ok(),
        None => false,
    }
}

/// Strip cpal's macOS `DeviceId` prefix (`"CoreAudio:"`) so the bare
/// `kAudioDevicePropertyDeviceUID` string can be passed to
/// `yapstack_audio::device::is_device_alive`.
fn strip_cpal_prefix(uid: &str) -> &str {
    uid.strip_prefix("CoreAudio:").unwrap_or(uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn collapsed_events_default_all_false() {
        let s = CollapsedEvents::default();
        assert!(!s.device_list);
        assert!(!s.default_input);
        assert!(!s.default_output);
        assert!(!s.default_system_output);
    }

    #[test]
    fn collapsed_events_merge_sets_each_kind() {
        let mut s = CollapsedEvents::default();
        s.merge(DeviceEvent::DeviceListChanged);
        s.merge(DeviceEvent::DefaultInputChanged);
        s.merge(DeviceEvent::DefaultOutputChanged);
        s.merge(DeviceEvent::DefaultSystemOutputChanged);
        assert!(s.device_list);
        assert!(s.default_input);
        assert!(s.default_output);
        assert!(s.default_system_output);
    }

    #[test]
    fn collapsed_events_merge_is_idempotent_per_kind() {
        // Repeated events of the same kind during the debounce window
        // collapse to a single emit pass.
        let mut s = CollapsedEvents::default();
        s.merge(DeviceEvent::DefaultInputChanged);
        s.merge(DeviceEvent::DefaultInputChanged);
        s.merge(DeviceEvent::DefaultInputChanged);
        assert!(s.default_input);
        assert!(!s.default_output);
        assert!(!s.default_system_output);
        assert!(!s.device_list);
    }

    #[test]
    fn collapsed_events_merge_preserves_independent_kinds() {
        let mut s = CollapsedEvents::default();
        s.merge(DeviceEvent::DefaultOutputChanged);
        s.merge(DeviceEvent::DeviceListChanged);
        // Output and DeviceList both fired; the other two stayed clean.
        assert!(s.default_output);
        assert!(s.device_list);
        assert!(!s.default_input);
        assert!(!s.default_system_output);
    }

    #[test]
    fn strip_cpal_prefix_removes_known_prefix() {
        assert_eq!(strip_cpal_prefix("CoreAudio:BuiltInMic"), "BuiltInMic");
    }

    #[test]
    fn strip_cpal_prefix_leaves_other_strings_alone() {
        // Non-cpal-format strings (raw UID, empty, foreign prefix) must
        // pass through unchanged so callers can pipe them straight into
        // is_device_alive's fail-open path.
        assert_eq!(strip_cpal_prefix("BuiltInMic"), "BuiltInMic");
        assert_eq!(strip_cpal_prefix(""), "");
        assert_eq!(strip_cpal_prefix("Wasapi:something"), "Wasapi:something");
    }

    #[test]
    fn try_send_intent_returns_false_for_empty_inbox() {
        let inbox: RestartIntentInbox = Arc::new(StdMutex::new(None));
        assert!(!try_send_intent(&inbox, RestartIntent::Mic));
        assert!(!try_send_intent(&inbox, RestartIntent::System));
    }

    #[test]
    fn try_send_intent_routes_to_populated_inbox() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RestartIntent>();
        let inbox: RestartIntentInbox = Arc::new(StdMutex::new(Some(tx)));

        assert!(try_send_intent(&inbox, RestartIntent::Mic));
        assert!(try_send_intent(&inbox, RestartIntent::System));

        // Both intents arrived in order.
        let first = rx.try_recv().expect("first intent buffered");
        let second = rx.try_recv().expect("second intent buffered");
        assert!(matches!(first, RestartIntent::Mic));
        assert!(matches!(second, RestartIntent::System));
    }

    #[test]
    fn try_send_intent_returns_false_when_receiver_was_dropped() {
        // The live loop ended (receiver dropped) but the inbox still
        // holds a stale sender. send() returns Err, broker falls back
        // to direct restart.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<RestartIntent>();
        drop(rx);
        let inbox: RestartIntentInbox = Arc::new(StdMutex::new(Some(tx)));

        assert!(!try_send_intent(&inbox, RestartIntent::Mic));
    }
}

/// Bluetooth/AirPods absorbs the default-device change before the route
/// is fully alive. If `is_device_alive` reports false, give macOS one
/// more debounce window to settle, then fall through. We don't block
/// indefinitely — if the device still isn't alive after the second
/// check, the restart attempt itself surfaces the failure via
/// `stream-health`.
async fn alive_gate(uid: Option<&str>) {
    let Some(uid) = uid else {
        return;
    };
    let bare = strip_cpal_prefix(uid);
    if yapstack_audio::device::is_device_alive(bare) {
        return;
    }
    debug!(
        "device broker: target {} not yet alive — waiting {}ms before restart",
        bare, DEBOUNCE_MS
    );
    tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
}

fn emit_devices_changed(app_handle: &AppHandle) {
    let inputs = match yapstack_audio::device::list_input_devices() {
        Ok(v) => v,
        Err(e) => {
            warn!("device broker: list_input_devices failed: {}", e);
            return;
        }
    };
    let outputs = match yapstack_audio::device::list_output_devices() {
        Ok(v) => v,
        Err(e) => {
            warn!("device broker: list_output_devices failed: {}", e);
            return;
        }
    };
    let mut all = inputs;
    all.extend(outputs);
    let payload: Vec<AudioDeviceInfoDto> = all.into_iter().map(AudioDeviceInfoDto::from).collect();
    debug!("device broker: emitting devices-changed ({} devices)", payload.len());
    if let Err(e) = app_handle.emit("devices-changed", payload) {
        warn!("device broker: emit devices-changed failed: {}", e);
    }
}

fn emit_default_changed(app_handle: &AppHandle, kind: DefaultDeviceKindDto) {
    let resolved = match kind {
        DefaultDeviceKindDto::Input => yapstack_audio::device::default_input_device(),
        // The cpal host has one notion of "default output"; the alerts
        // route (`kAudioHardwarePropertyDefaultSystemOutputDevice`) isn't
        // exposed separately. We surface the kind distinctly so the FE
        // can show *what* the user changed, even though both resolve
        // through the same cpal call.
        DefaultDeviceKindDto::Output | DefaultDeviceKindDto::SystemOutput => {
            yapstack_audio::device::default_output_device()
        }
    };
    let (device_id, device_name) = match resolved {
        Ok(info) => (info.id, Some(info.name)),
        Err(e) => {
            debug!("device broker: resolving default {:?} failed: {}", kind, e);
            (None, None)
        }
    };
    let payload = DefaultDeviceChangedPayload {
        kind,
        device_id,
        device_name,
    };
    debug!("device broker: emitting default-device-changed {:?}", payload);
    if let Err(e) = app_handle.emit("default-device-changed", payload) {
        warn!("device broker: emit default-device-changed failed: {}", e);
    }
}
