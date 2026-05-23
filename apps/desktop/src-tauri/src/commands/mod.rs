pub mod audio;
pub mod capture;
pub mod dictation;
pub mod error;
pub mod live_transcription;
pub mod logs;
pub mod permissions;
pub mod silero_vad;
pub mod system_volume;
pub mod transcription;
pub mod transcription_scheduler;

use serde::Serialize;
use specta::Type;
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Type)]
pub struct HealthStatus {
    pub status: String,
    pub version: String,
}

#[tauri::command]
#[specta::specta]
pub fn health_check() -> HealthStatus {
    HealthStatus {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_autostart_enabled(app: tauri::AppHandle) -> Result<bool, error::CommandError> {
    let manager = app.state::<tauri_plugin_autostart::AutoLaunchManager>();
    manager
        .is_enabled()
        .map_err(|e| error::CommandError::Internal {
            message: e.to_string(),
        })
}

#[tauri::command]
#[specta::specta]
pub fn set_autostart_enabled(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), error::CommandError> {
    let manager = app.state::<tauri_plugin_autostart::AutoLaunchManager>();
    if enabled {
        manager.enable().map_err(|e| error::CommandError::Internal {
            message: e.to_string(),
        })
    } else {
        manager
            .disable()
            .map_err(|e| error::CommandError::Internal {
                message: e.to_string(),
            })
    }
}

#[tauri::command]
#[specta::specta]
pub async fn show_overlay_panel(
    app: tauri::AppHandle,
    label: String,
) -> Result<(), error::CommandError> {
    #[cfg(target_os = "macos")]
    {
        let app_clone = app.clone();
        app.run_on_main_thread(move || {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app_clone.get_webview_panel(&label) {
                panel.show();
            }
        })
        .map_err(|e| error::CommandError::Internal {
            message: e.to_string(),
        })?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(window) = app.get_webview_window(&label) {
            window.set_always_on_top(true).ok();
            window.show().map_err(|e| error::CommandError::Internal {
                message: e.to_string(),
            })?;
        }
    }
    Ok(())
}

/// Global cursor position in physical pixels, in screen coordinate space.
/// Used by the Insight overlay to implement region-based click-through:
/// the JS polls this and toggles click-through based on whether the cursor
/// falls inside the overlay's header strip or its body.
#[tauri::command]
#[specta::specta]
pub async fn get_cursor_position(app: tauri::AppHandle) -> Result<(f64, f64), error::CommandError> {
    let pos = app
        .cursor_position()
        .map_err(|e| error::CommandError::Internal {
            message: e.to_string(),
        })?;
    Ok((pos.x, pos.y))
}

/// Toggle click-through (mouse-event pass-through) on an overlay window.
/// On macOS we *must* call `set_ignores_mouse_events` on the underlying
/// NSPanel — Tauri's JS `setIgnoreCursorEvents` routes through the
/// pre-conversion NSWindow handle and silently no-ops after
/// `tauri-nspanel` swaps in its panel subclass.
#[tauri::command]
#[specta::specta]
pub async fn set_overlay_ignore_cursor_events(
    app: tauri::AppHandle,
    label: String,
    ignore: bool,
) -> Result<(), error::CommandError> {
    #[cfg(target_os = "macos")]
    {
        let app_clone = app.clone();
        app.run_on_main_thread(move || {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app_clone.get_webview_panel(&label) {
                panel.set_ignores_mouse_events(ignore);
            }
        })
        .map_err(|e| error::CommandError::Internal {
            message: e.to_string(),
        })?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(window) = app.get_webview_window(&label) {
            window
                .set_ignore_cursor_events(ignore)
                .map_err(|e| error::CommandError::Internal {
                    message: e.to_string(),
                })?;
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn hide_overlay_panel(
    app: tauri::AppHandle,
    label: String,
) -> Result<(), error::CommandError> {
    #[cfg(target_os = "macos")]
    {
        let app_clone = app.clone();
        app.run_on_main_thread(move || {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app_clone.get_webview_panel(&label) {
                panel.hide();
            }
        })
        .map_err(|e| error::CommandError::Internal {
            message: e.to_string(),
        })?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(window) = app.get_webview_window(&label) {
            window.set_always_on_top(false).ok();
            window.hide().map_err(|e| error::CommandError::Internal {
                message: e.to_string(),
            })?;
        }
    }
    Ok(())
}
