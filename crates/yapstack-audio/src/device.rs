use std::collections::HashMap;

use cpal::traits::{DeviceTrait, HostTrait};
use yapstack_common::types::{AudioDeviceInfo, DeviceType};

use crate::error::AudioError;

type Result<T> = std::result::Result<T, AudioError>;

fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| {
        // On Windows/WASAPI, name() returns the short DeviceDesc (e.g., "Microphone").
        // The FriendlyName (e.g., "Microphone (Razer BlackShark)") is in extended().
        // On macOS, extended() is empty so we fall back to name().
        d.extended()
            .first()
            .cloned()
            .unwrap_or_else(|| d.name().to_string())
    })
}

fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

pub fn list_input_devices() -> Result<Vec<AudioDeviceInfo>> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| device_id(&d));

    // Deduplicate by name: on Windows/WASAPI, multiple endpoints share the same
    // friendly name. Keep the first seen entry, but prefer the one whose ID
    // matches the default device's ID.
    let mut seen: HashMap<String, AudioDeviceInfo> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for device in host.input_devices()? {
        let name = match device_name(&device) {
            Some(n) => n,
            None => continue,
        };
        let id = device_id(&device);
        let is_default = id.is_some() && id == default_id;

        if let Some(existing) = seen.get_mut(&name) {
            // Replace if the new entry is the default (prefer default endpoint)
            if is_default && !existing.is_default {
                *existing = AudioDeviceInfo {
                    id,
                    name: name.clone(),
                    device_type: DeviceType::Input,
                    is_default,
                };
            }
        } else {
            order.push(name.clone());
            seen.insert(
                name.clone(),
                AudioDeviceInfo {
                    id,
                    name,
                    device_type: DeviceType::Input,
                    is_default,
                },
            );
        }
    }

    // If no entry was marked as default by ID match, fall back to name-based
    // default detection (macOS doesn't always have matching IDs).
    if !seen.values().any(|d| d.is_default) {
        let default_name = host.default_input_device().and_then(|d| device_name(&d));
        if let Some(ref dn) = default_name {
            if let Some(entry) = seen.get_mut(dn) {
                entry.is_default = true;
            }
        }
    }

    Ok(order
        .into_iter()
        .filter_map(|name| seen.remove(&name))
        .collect())
}

pub fn list_output_devices() -> Result<Vec<AudioDeviceInfo>> {
    let host = cpal::default_host();
    let default_id = host.default_output_device().and_then(|d| device_id(&d));

    let mut seen: HashMap<String, AudioDeviceInfo> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for device in host.output_devices()? {
        let name = match device_name(&device) {
            Some(n) => n,
            None => continue,
        };
        let id = device_id(&device);
        let is_default = id.is_some() && id == default_id;

        if let Some(existing) = seen.get_mut(&name) {
            if is_default && !existing.is_default {
                *existing = AudioDeviceInfo {
                    id,
                    name: name.clone(),
                    device_type: DeviceType::Output,
                    is_default,
                };
            }
        } else {
            order.push(name.clone());
            seen.insert(
                name.clone(),
                AudioDeviceInfo {
                    id,
                    name,
                    device_type: DeviceType::Output,
                    is_default,
                },
            );
        }
    }

    if !seen.values().any(|d| d.is_default) {
        let default_name = host.default_output_device().and_then(|d| device_name(&d));
        if let Some(ref dn) = default_name {
            if let Some(entry) = seen.get_mut(dn) {
                entry.is_default = true;
            }
        }
    }

    Ok(order
        .into_iter()
        .filter_map(|name| seen.remove(&name))
        .collect())
}

pub fn list_all_devices() -> Result<Vec<AudioDeviceInfo>> {
    let mut devices = list_input_devices()?;
    devices.extend(list_output_devices()?);
    Ok(devices)
}

pub fn default_input_device() -> Result<AudioDeviceInfo> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoDevicesAvailable)?;
    let name = device
        .description()
        .map(|d| {
            d.extended()
                .first()
                .cloned()
                .unwrap_or_else(|| d.name().to_string())
        })
        .map_err(|e| AudioError::DeviceInit(e.to_string()))?;
    let id = device_id(&device);

    Ok(AudioDeviceInfo {
        id,
        name,
        device_type: DeviceType::Input,
        is_default: true,
    })
}

/// Resolves an input device by ID first, then by name, then falls back to system default.
pub(crate) fn resolve_input_device(id: Option<&str>, name: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();

    // Try ID-based lookup first (most reliable on Windows)
    if let Some(target_id) = id {
        if let Ok(parsed) = target_id.parse::<cpal::DeviceId>() {
            if let Some(device) = host.device_by_id(&parsed) {
                return Ok(device);
            }
        }
        // ID didn't parse/match — try it as a device name (handles frontend d.id ?? d.name pattern)
        if let Some(device) = host
            .input_devices()?
            .find(|d| device_name(d).as_deref() == Some(target_id))
        {
            return Ok(device);
        }
    }

    // Try name-based lookup
    if let Some(target_name) = name {
        if let Some(device) = host
            .input_devices()?
            .find(|d| device_name(d).as_deref() == Some(target_name))
        {
            return Ok(device);
        }
        // Name didn't match — fall through to default
    }

    // Fall back to system default
    if id.is_some() || name.is_some() {
        // Caller specified something but we couldn't find it — still fall back
        // rather than erroring, since the user's device may have been unplugged.
    }

    host.default_input_device()
        .ok_or(AudioError::NoDevicesAvailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires audio hardware
    fn test_list_input_devices() {
        let devices = list_input_devices().unwrap();
        assert!(!devices.is_empty(), "expected at least one input device");
        for device in &devices {
            assert_eq!(device.device_type, DeviceType::Input);
        }
    }

    #[test]
    #[ignore] // Requires audio hardware
    fn test_default_input_device() {
        let device = default_input_device().unwrap();
        assert!(device.is_default);
        assert_eq!(device.device_type, DeviceType::Input);
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)] // WASAPI COM cleanup crashes on CI
    fn test_resolve_nonexistent_device_by_name() {
        // Falls back to default when name doesn't match
        let result = resolve_input_device(None, Some("nonexistent_device_xyz_12345"));
        // Should succeed (falls back to system default) or fail with NoDevicesAvailable
        // depending on hardware — just verify it doesn't panic
        let _ = result;
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)]
    fn test_resolve_nonexistent_device_by_id() {
        // Falls back to default when ID doesn't match
        let result = resolve_input_device(Some("InvalidHost:fake_id"), None);
        let _ = result;
    }

    #[test]
    #[cfg_attr(target_os = "windows", ignore)]
    fn test_resolve_default_device() {
        // Both None → system default
        let result = resolve_input_device(None, None);
        let _ = result;
    }

    #[test]
    #[ignore] // Requires audio hardware
    fn test_resolve_input_device_id_as_name_fallback() {
        // When `id` is actually a device name string (because frontend sent d.id ?? d.name
        // and d.id was null), resolve_input_device should match by name after ID parse fails.
        let devices = list_input_devices().unwrap();
        assert!(!devices.is_empty(), "need at least one input device");

        let target = &devices[0];
        // Pass the device *name* as the `id` parameter, with `name` = None
        let result = resolve_input_device(Some(&target.name), None);
        assert!(result.is_ok(), "should resolve device name passed as id");

        let resolved = result.unwrap();
        let resolved_name = device_name(&resolved);
        assert_eq!(
            resolved_name.as_deref(),
            Some(target.name.as_str()),
            "resolved device should match the target by name"
        );
    }
}
