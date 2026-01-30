use serde::{Deserialize, Serialize};

/// Default fallback sample rate when no buffer is active.
pub const DEFAULT_SAMPLE_RATE: u32 = 16000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_capture_history")]
    pub capture_history_seconds: f32,
}

fn default_capture_history() -> f32 {
    180.0
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_history_seconds: default_capture_history(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_audio_config() {
        let config = AudioConfig::default();
        assert!((config.capture_history_seconds - 180.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_audio_config_serde_roundtrip() {
        let config = AudioConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AudioConfig = serde_json::from_str(&json).unwrap();
        assert!(
            (deserialized.capture_history_seconds - config.capture_history_seconds).abs()
                < f32::EPSILON
        );
    }
}
