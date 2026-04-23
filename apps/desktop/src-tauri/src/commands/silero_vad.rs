//! Silero VAD wrapper for the live-transcription loop.
//!
//! Replaces the hand-rolled RMS energy detector (`peek_energy_rms`) with a
//! neural speech detector. Silero is trained on speech specifically, so it
//! doesn't false-trigger on music, keyboard clicks, fans, or HVAC, and it
//! catches quiet / distant speech that sits below an RMS threshold.
//!
//! Used by both Whisper and Parakeet live sessions. The only things that
//! stay engine-specific are the *timing* knobs (silence_duration,
//! pre_roll, poll_interval, max_chunk_duration) in `VadTuning` — those
//! are independent of what produces the speech signal.
//!
//! Performance: Silero V5 inference on CPU is ~1 ms per 32 ms frame on an
//! Apple-Silicon core, well under our 100 ms Parakeet poll interval. The
//! model is bundled in the binary via `silero::BUNDLED_MODEL` (≈2 MB).

use silero::{SampleRate, Session, SessionOptions, StreamState};
use tracing::warn;

/// Speech-probability threshold at or above which a frame is considered
/// speech (Silero's V5 default, validated against the upstream Python
/// reference in the crate's own tests).
pub const SPEECH_THRESHOLD: f32 = 0.5;

/// End-of-speech threshold; paired with `SPEECH_THRESHOLD` to give
/// hysteresis and prevent flapping between states on short dips.
pub const SILENCE_THRESHOLD: f32 = 0.35;

/// Target sample rate for Silero VAD. The model only supports 8 kHz and
/// 16 kHz; we use 16 kHz for better accuracy.
pub const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;

/// Shared ONNX session. A single `Session` can feed multiple per-source
/// `StreamState`s sequentially (the session is `Send` but not `Sync`, so
/// callers must hold a mutable reference across a single call).
pub struct SileroVad {
    session: Session,
}

impl SileroVad {
    /// Load the bundled Silero V5 model with default graph-optimization
    /// settings.
    pub fn new() -> Result<Self, silero::Error> {
        let options = SessionOptions::default();
        let session = Session::bundled_with_options(options)?;
        Ok(Self { session })
    }

    /// Feed new audio samples (at `source_sample_rate`) into `stream`'s
    /// rolling Silero state and return the *latest* speech probability
    /// produced during the call, or `None` if no full 32 ms frame landed
    /// yet (remaining samples stay buffered inside the stream state for
    /// the next call).
    ///
    /// For VAD decisions we only care about the most-recent probability
    /// in each poll window — we don't need to aggregate intra-poll
    /// probabilities. Silero's own recurrent state smooths across frames.
    pub fn score_stream(
        &mut self,
        stream: &mut StreamState,
        samples: &[f32],
        source_sample_rate: u32,
    ) -> Option<f32> {
        if samples.is_empty() {
            return None;
        }
        let resampled = resample_to_16k(samples, source_sample_rate);
        if resampled.is_empty() {
            return None;
        }
        let mut latest: Option<f32> = None;
        if let Err(e) = self
            .session
            .process_stream(stream, &resampled, |p| latest = Some(p))
        {
            // Don't crash the live loop on a transient inference error —
            // the next poll tick will retry with fresh audio.
            warn!("silero inference failed: {e}");
            return None;
        }
        latest
    }

    /// Batch-score a full historical buffer (backfill use case). Returns
    /// one probability per 32 ms Silero frame (512 samples at 16 kHz).
    ///
    /// Creates a fresh `StreamState` internally — this is the right shape
    /// for backfill where we process a standalone window with no prior
    /// context. For live streaming use `score_stream` with a caller-owned
    /// `StreamState` so the LSTM memory persists across polls.
    pub fn score_all(&mut self, samples: &[f32], source_sample_rate: u32) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }
        let resampled = resample_to_16k(samples, source_sample_rate);
        if resampled.is_empty() {
            return Vec::new();
        }
        let mut stream = StreamState::new(SampleRate::Rate16k);
        let mut probs: Vec<f32> = Vec::new();
        if let Err(e) = self
            .session
            .process_stream(&mut stream, &resampled, |p| probs.push(p))
        {
            warn!("silero batch inference failed: {e}");
        }
        probs
    }
}

/// Silero V5 at 16 kHz emits one probability per 512-sample chunk
/// (≈32 ms). Exposed so callers mapping frame indices back to original
/// sample positions (e.g. backfill chunk boundaries) share the constant.
pub const FRAME_DURATION_SECS: f32 = 0.032;

/// Per-source streaming state. One instance per audio source (mic, system).
pub struct SileroSource {
    pub stream: StreamState,
    /// Ring-buffer read cursor for VAD-only audio consumption. Independent
    /// of the chunk-extraction cursors (`vad.cursor`, `vad.speech_start_pos`)
    /// so reading for VAD doesn't disturb chunk boundaries.
    pub read_pos: usize,
    /// Most recent probability produced by Silero. Sticks between polls
    /// that don't accumulate a full 32 ms frame so `poll_vad` keeps
    /// seeing a value rather than toggling to `None`.
    pub last_probability: Option<f32>,
}

impl SileroSource {
    pub fn new(initial_read_pos: usize) -> Self {
        Self {
            stream: StreamState::new(SampleRate::Rate16k),
            read_pos: initial_read_pos,
            last_probability: None,
        }
    }

    /// Reset the recurrent state (e.g. after a long silence that invalidates
    /// the LSTM's context).
    #[allow(dead_code)] // kept for future prompt-decay-style resets
    pub fn reset(&mut self) {
        self.stream.reset();
        self.last_probability = None;
    }
}

/// Resample `src` (at `src_rate` Hz) down to 16 kHz using point-sampling
/// with a fractional index. For VAD this is adequate — Silero is robust
/// to mild aliasing, and proper anti-alias filtering would add CPU for
/// no accuracy gain on a speech-detection task.
///
/// Returns an empty vec when the ratio would collapse the output to zero
/// samples (e.g. extremely short input).
fn resample_to_16k(src: &[f32], src_rate: u32) -> Vec<f32> {
    if src.is_empty() {
        return Vec::new();
    }
    if src_rate == TARGET_SAMPLE_RATE_HZ {
        return src.to_vec();
    }
    let ratio = src_rate as f64 / TARGET_SAMPLE_RATE_HZ as f64;
    let out_len = (src.len() as f64 / ratio).floor() as usize;
    if out_len == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let idx = (i as f64 * ratio) as usize;
        // Clamp: floor() rounding at the tail can push idx one past the
        // last sample on the final iteration; take the last sample then.
        let idx = idx.min(src.len() - 1);
        out.push(src[idx]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_passthrough_at_16k() {
        let src: Vec<f32> = (0..512).map(|i| i as f32 * 0.001).collect();
        let out = resample_to_16k(&src, 16_000);
        assert_eq!(out, src);
    }

    #[test]
    fn resample_48k_to_16k_is_3x_decimation() {
        // 48k → 16k is a 3× decimation; output length should be input / 3
        // (plus/minus one for floor rounding).
        let src: Vec<f32> = (0..3_000).map(|i| i as f32).collect();
        let out = resample_to_16k(&src, 48_000);
        assert!(
            out.len() >= 999 && out.len() <= 1_001,
            "expected ~1000 samples, got {}",
            out.len()
        );
        // First output sample = first input sample
        assert_eq!(out[0], 0.0);
        // Every successive output corresponds to input[i*3]
        for (i, &v) in out.iter().take(10).enumerate() {
            assert_eq!(v, (i * 3) as f32);
        }
    }

    #[test]
    fn resample_44100_to_16k_is_approx_2_75x() {
        // 44100 → 16000, ratio ~2.75625
        let src: Vec<f32> = vec![1.0; 4_410]; // 100 ms at 44.1 kHz
        let out = resample_to_16k(&src, 44_100);
        // Should produce ~1600 samples (100 ms at 16 kHz)
        assert!(
            out.len() >= 1_598 && out.len() <= 1_602,
            "expected ~1600 samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resample_empty_returns_empty() {
        let out = resample_to_16k(&[], 48_000);
        assert!(out.is_empty());
    }

    #[test]
    fn resample_too_short_returns_empty() {
        // Two 48 kHz samples can't produce a 16 kHz output (ratio = 3,
        // out_len = floor(2 / 3) = 0)
        let out = resample_to_16k(&[0.1, 0.2], 48_000);
        assert!(out.is_empty());
    }

    #[test]
    fn silero_session_loads_and_returns_probability_for_silence() {
        // Smoke test: bundled model loads and produces a valid probability.
        let Ok(mut vad) = SileroVad::new() else {
            // ort runtime not available on this host (e.g. musl CI builder).
            // Mark the test as a soft skip by returning; we can't conditionally
            // #[ignore] at runtime, but the bundled session is expected to
            // work on every supported dev platform.
            eprintln!("skipping: Silero session init failed (ort unavailable)");
            return;
        };
        let mut source = SileroSource::new(0);
        // 100 ms of silence at 16 kHz = 1600 samples
        let silence = vec![0.0f32; 1_600];
        let prob = vad.score_stream(&mut source.stream, &silence, 16_000);
        let prob = prob.expect("should produce >=1 probability for 100ms input");
        assert!(
            (0.0..=1.0).contains(&prob),
            "probability must be in [0,1], got {prob}"
        );
        // Silence should score well below the speech threshold.
        assert!(
            prob < SPEECH_THRESHOLD,
            "pure silence should not cross speech threshold; got {prob}"
        );
    }
}
