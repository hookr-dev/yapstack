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

                        flush(&app_handle, state).await;
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

async fn flush(app_handle: &AppHandle, state: CollapsedEvents) {
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
