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
//!   emit `default-device-changed`; later phases route a restart intent
//!   through the live-transcription loop.
//!
//! Phase 3: subscribes the sink and logs incoming events. Phase 4 adds
//! debounce + Tauri event emission. Phase 6 adds restart routing.

use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info};
use yapstack_audio::system::device_watcher::{DeviceEvent, DeviceEventSink};

use crate::commands::audio::AudioManagerState;

/// Spawn the device-change broker on the Tauri async runtime. Returns
/// immediately; the broker runs until the shutdown signal flips to
/// `true` or the audio manager drops its watchers.
pub fn spawn(
    #[allow(unused_variables)] app_handle: AppHandle,
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
        // Drop the local tx now that the sink owns its own clone via the
        // closure capture. rx.recv() will yield None only after the
        // watcher detaches the sink (on shutdown below).
        drop(tx);

        info!("device broker: started");

        loop {
            tokio::select! {
                maybe_event = rx.recv() => match maybe_event {
                    Some(event) => {
                        // Phase 3: log only. Debounce + FE emission lands
                        // in Phase 4.
                        debug!("device broker: received {:?}", event);
                    }
                    None => {
                        // All senders dropped — only happens if the
                        // watcher detached its sink and our local tx was
                        // dropped. Treat as graceful exit.
                        info!("device broker: sink detached, exiting");
                        break;
                    }
                },
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        // Detach the sink so the listener thread doesn't
                        // keep pushing into a soon-to-be-dropped channel.
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
