use std::borrow::Cow;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MixConfig {
    pub mic_gain: f32,
    pub system_gain: f32,
    pub normalize: bool,
}

impl Default for MixConfig {
    fn default() -> Self {
        Self {
            mic_gain: 0.5,
            system_gain: 0.5,
            normalize: true,
        }
    }
}

/// Applies a gain factor to all samples, returning a new buffer.
pub fn apply_gain(samples: &[f32], gain: f32) -> Vec<f32> {
    samples.iter().map(|&s| s * gain).collect()
}

/// Normalizes samples in place so the peak absolute value is 1.0.
/// Returns the scaling factor applied. If all samples are zero, returns 1.0 and does nothing.
pub fn normalize_in_place(samples: &mut [f32]) -> f32 {
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    if peak == 0.0 || peak == 1.0 {
        return 1.0;
    }

    let factor = 1.0 / peak;
    for s in samples.iter_mut() {
        *s *= factor;
    }
    factor
}

/// Scales samples down if peak exceeds 1.0. Never amplifies.
/// Returns the scaling factor applied (1.0 if no limiting needed).
pub fn limit_in_place(samples: &mut [f32]) -> f32 {
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak <= 1.0 {
        return 1.0;
    }
    let factor = 1.0 / peak;
    for s in samples.iter_mut() {
        *s *= factor;
    }
    factor
}

/// Resample audio using sinc interpolation (delegates to `yapstack_common::audio::resample`).
///
/// Returns `Ok(Cow::Borrowed)` when `from_rate == to_rate` (zero-copy).
/// Typically used to resample system audio to match the mic's sample rate
/// in Mixed capture mode when the two devices run at different rates.
pub fn resample(
    samples: &[f32],
    from_rate: u32,
    to_rate: u32,
) -> Result<Cow<'_, [f32]>, yapstack_common::audio::ResampleError> {
    yapstack_common::audio::resample(samples, from_rate, to_rate)
}

/// Converts interleaved multi-channel audio to mono by averaging channels per frame.
/// Mono input (channels == 1) borrows the original slice (zero-copy).
pub(crate) fn deinterleave_to_mono(samples: &[f32], channels: u16) -> std::borrow::Cow<'_, [f32]> {
    yapstack_common::audio::deinterleave_to_mono(samples, channels)
}

/// Mixes mic and system audio to a single mono buffer.
///
/// Both inputs must already be mono (single-channel). Use [`deinterleave_to_mono`]
/// first if either input is multi-channel.
///
/// Each input is scaled by its respective gain, then summed sample-by-sample.
/// The shorter input is zero-padded to match the longer.
/// If `normalize` is true, the result is normalized so the peak is 1.0.
pub fn mix_to_mono(mic: &[f32], system: &[f32], config: &MixConfig) -> Vec<f32> {
    let len = mic.len().max(system.len());
    let mut out = vec![0.0f32; len];

    for (i, sample) in out.iter_mut().enumerate() {
        let m = mic.get(i).copied().unwrap_or(0.0) * config.mic_gain;
        let s = system.get(i).copied().unwrap_or(0.0) * config.system_gain;
        *sample = m + s;
    }

    if config.normalize {
        normalize_in_place(&mut out);
    } else {
        limit_in_place(&mut out);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mix_equal_length() {
        let mic = vec![0.5, -0.5, 0.25];
        let system = vec![0.5, 0.5, -0.25];
        let config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: false,
        };
        let result = mix_to_mono(&mic, &system, &config);
        assert_eq!(result.len(), 3);
        assert!((result[0] - 1.0).abs() < f32::EPSILON);
        assert!((result[1] - 0.0).abs() < f32::EPSILON);
        assert!((result[2] - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_mix_different_lengths() {
        let mic = vec![1.0, 0.5];
        let system = vec![0.5, 0.5, 0.5, 0.5];
        let config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: false,
        };
        let result = mix_to_mono(&mic, &system, &config);
        assert_eq!(result.len(), 4);
        // Raw sums: [1.5, 1.0, 0.5, 0.5], peak = 1.5
        // After limiting (factor = 1/1.5 ≈ 0.6667): [1.0, 0.6667, 0.3333, 0.3333]
        let factor = 1.0 / 1.5;
        assert!((result[0] - 1.0).abs() < 0.001);
        assert!((result[1] - 1.0 * factor).abs() < 0.001);
        assert!((result[2] - 0.5 * factor).abs() < 0.001);
    }

    #[test]
    fn test_gain_application() {
        let samples = vec![1.0, -1.0, 0.5];
        let result = apply_gain(&samples, 0.5);
        assert!((result[0] - 0.5).abs() < f32::EPSILON);
        assert!((result[1] + 0.5).abs() < f32::EPSILON);
        assert!((result[2] - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn test_normalization() {
        let mut samples = vec![0.25, -0.5, 0.1];
        let factor = normalize_in_place(&mut samples);
        assert!((factor - 2.0).abs() < f32::EPSILON);
        assert!((samples[0] - 0.5).abs() < f32::EPSILON);
        assert!((samples[1] + 1.0).abs() < f32::EPSILON);
        assert!((samples[2] - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn test_empty_inputs() {
        let config = MixConfig::default();
        let result = mix_to_mono(&[], &[], &config);
        assert!(result.is_empty());
    }

    #[test]
    fn test_default_config() {
        let config = MixConfig::default();
        assert!((config.mic_gain - 0.5).abs() < f32::EPSILON);
        assert!((config.system_gain - 0.5).abs() < f32::EPSILON);
        assert!(config.normalize);
    }

    #[test]
    fn test_noop_normalization_all_zero() {
        let mut samples = vec![0.0, 0.0, 0.0];
        let factor = normalize_in_place(&mut samples);
        assert!((factor - 1.0).abs() < f32::EPSILON);
        assert!(samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_noop_normalization_already_peak() {
        let mut samples = vec![1.0, -0.5, 0.0];
        let factor = normalize_in_place(&mut samples);
        assert!((factor - 1.0).abs() < f32::EPSILON);
        assert!((samples[0] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_deinterleave_stereo_to_mono() {
        // Stereo: [L0, R0, L1, R1] -> mono: [(L0+R0)/2, (L1+R1)/2]
        let stereo = vec![0.8, 0.2, -0.4, 0.6];
        let mono = deinterleave_to_mono(&stereo, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.5).abs() < f32::EPSILON);
        assert!((mono[1] - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_deinterleave_mono_passthrough() {
        let samples = vec![0.1, 0.2, 0.3];
        let result = deinterleave_to_mono(&samples, 1);
        assert_eq!(&*result, &samples[..]);
    }

    #[test]
    fn test_deinterleave_empty() {
        let result = deinterleave_to_mono(&[], 2);
        assert!(result.is_empty());
    }

    #[test]
    fn test_deinterleave_quad_channel() {
        // 4 channels: [c0, c1, c2, c3] per frame
        let quad = vec![0.4, 0.8, 0.0, -0.4];
        let mono = deinterleave_to_mono(&quad, 4);
        assert_eq!(mono.len(), 1);
        // (0.4 + 0.8 + 0.0 + -0.4) / 4 = 0.8 / 4 = 0.2
        assert!((mono[0] - 0.2).abs() < f32::EPSILON);
    }

    // --- resample tests ---

    #[test]
    fn test_resample_identity() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let result = resample(&samples, 48000, 48000).unwrap();
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(&*result, &samples[..]);
    }

    #[test]
    fn test_resample_empty() {
        let result = resample(&[], 96000, 48000).unwrap();
        assert!(matches!(result, Cow::Borrowed(_)));
        assert!(result.is_empty());
    }

    #[test]
    fn test_resample_downsample_2x() {
        // 96kHz → 48kHz: should halve the number of samples
        let samples: Vec<f32> = (0..9600).map(|i| (i as f32) / 9600.0).collect();
        let result = resample(&samples, 96000, 48000).unwrap();
        assert!(matches!(result, Cow::Owned(_)));
        // Sinc resampler may have slight length variance from edge effects
        let expected_len: i32 = 4800;
        assert!(
            (result.len() as i32 - expected_len).abs() < 100,
            "expected ~{expected_len}, got {}",
            result.len()
        );
    }

    #[test]
    fn test_resample_upsample_3x() {
        // 16kHz → 48kHz: should triple the number of samples
        // Use 1s of audio (16000 samples) so sinc filter edge effects are negligible
        let samples: Vec<f32> = (0..16000).map(|i| (i as f32) / 16000.0).collect();
        let result = resample(&samples, 16000, 48000).unwrap();
        assert!(matches!(result, Cow::Owned(_)));
        let expected_len: i32 = 48000;
        assert!(
            (result.len() as i32 - expected_len).abs() < 500,
            "expected ~{expected_len}, got {}",
            result.len()
        );
    }

    #[test]
    fn test_resample_preserves_dc() {
        // Constant signal should remain constant after resampling
        let samples = vec![0.5f32; 9600];
        let result = resample(&samples, 96000, 48000).unwrap();
        // Skip edges due to sinc resampler startup/shutdown transients
        let interior = &result[50..result.len().saturating_sub(50)];
        for &s in interior {
            assert!(
                (s - 0.5).abs() < 0.01,
                "expected ~0.5 after resample, got {}",
                s
            );
        }
    }

    #[test]
    fn test_mix_with_normalization() {
        let mic = vec![0.4, -0.2];
        let system = vec![0.1, 0.3];
        let config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: true,
        };
        let result = mix_to_mono(&mic, &system, &config);
        // Before normalize: [0.5, 0.1], peak = 0.5
        // After normalize: [1.0, 0.2]
        assert!((result[0] - 1.0).abs() < f32::EPSILON);
        assert!((result[1] - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_limit_in_place_scales_down_when_over() {
        let mut samples = vec![1.5, -0.75, 0.3];
        let factor = limit_in_place(&mut samples);
        assert!((factor - 1.0 / 1.5).abs() < 0.001);
        assert!((samples[0] - 1.0).abs() < 0.001);
        assert!((samples[1] - (-0.5)).abs() < 0.001);
        assert!((samples[2] - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_limit_in_place_no_change_when_within_range() {
        let mut samples = vec![0.8, -0.5, 0.3];
        let original = samples.clone();
        let factor = limit_in_place(&mut samples);
        assert!((factor - 1.0).abs() < f32::EPSILON);
        assert_eq!(samples, original);
    }

    #[test]
    fn test_limit_in_place_no_change_at_exactly_one() {
        let mut samples = vec![1.0, -0.5, 0.0];
        let original = samples.clone();
        let factor = limit_in_place(&mut samples);
        assert!((factor - 1.0).abs() < f32::EPSILON);
        assert_eq!(samples, original);
    }

    #[test]
    fn test_mix_limiting_when_sum_exceeds_one() {
        // Both sources hot: mic=0.7, system=0.6 → sum=1.3 per sample
        let mic = vec![0.7; 100];
        let system = vec![0.6; 100];
        let config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: false,
        };
        let result = mix_to_mono(&mic, &system, &config);
        // Peak should be limited to 1.0
        for &s in &result {
            assert!(s <= 1.0, "sample {} exceeds 1.0", s);
            assert!(s >= -1.0, "sample {} below -1.0", s);
        }
        // All samples should be 1.3 * (1/1.3) = 1.0
        assert!((result[0] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_mix_no_limiting_when_within_range() {
        let mic = vec![0.3; 100];
        let system = vec![0.2; 100];
        let config = MixConfig {
            mic_gain: 1.0,
            system_gain: 1.0,
            normalize: false,
        };
        let result = mix_to_mono(&mic, &system, &config);
        // Sum = 0.5, no limiting needed
        for &s in &result {
            assert!((s - 0.5).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_resample_zero_from_rate() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let result = resample(&samples, 0, 48000).unwrap();
        // Should return input unchanged (borrowed)
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(&*result, &samples[..]);
    }

    #[test]
    fn test_resample_zero_to_rate() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let result = resample(&samples, 48000, 0).unwrap();
        // Should return input unchanged (borrowed)
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(&*result, &samples[..]);
    }
}
