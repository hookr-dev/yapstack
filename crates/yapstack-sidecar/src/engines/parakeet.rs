//! Parakeet TDT-v3 backend (parakeet-rs 0.3 + ONNX Runtime via `ort`).
//!
//! Loads the multilingual TDT model from a directory containing
//! `encoder-model.onnx`, `encoder-model.onnx.data`, `decoder_joint-model.onnx`,
//! and `vocab.txt`.
//!
//! Execution provider:
//! - With the `coreml` feature on (default for Apple targets via the
//!   build script), uses ort-coreml with `ComputeUnits::CPUAndGPU` and a
//!   per-app cache directory to avoid the ~5 s recompile on every spawn.
//!   parakeet-rs falls back to CPU automatically (`error_on_failure()`)
//!   if a given user's model can't run on CoreML.
//! - Otherwise, runs on CPU.

use std::path::{Path, PathBuf};

use parakeet_rs::sortformer::{Sortformer, SpeakerSegment};
use parakeet_rs::{ParakeetTDT, TimedToken, TimestampMode, Transcriber};
use tracing::{debug, info, warn};
use yapstack_common::types::TranscriptSegment;

use crate::engines::{
    normalize_spacing, sanitize_text, should_include_segment, TranscribeOpts, TranscriptionBackend,
    TranscriptionOutput,
};

const SORTFORMER_SAMPLE_RATE: u32 = 16000;
/// parakeet-rs's `transcribe_samples` rejects anything other than 16 kHz mono
/// despite the docstring claiming auto-resampling — empirically confirmed
/// against parakeet-rs 0.3.5 with TDT v3. We resample on our side using the
/// shared rubato-based helper.
const PARAKEET_SAMPLE_RATE: u32 = 16000;

/// Silence gap (seconds) that forces a new segment boundary when grouping
/// Parakeet's word-level tokens. Roughly matches Whisper's per-segment cadence.
const SEGMENT_GAP_SECS: f32 = 0.5;

/// Soft cap (seconds) on a single segment's span — once a segment runs this
/// long without hitting a silence gap, force a break for UI readability.
const MAX_SEGMENT_SECS: f32 = 12.0;

pub struct ParakeetBackend {
    model: Option<ParakeetTDT>,
    /// Path to the Sortformer ONNX file, set at sidecar startup via
    /// `--sortformer-model`. `None` disables diarization regardless of
    /// per-request flags.
    sortformer_model_path: Option<PathBuf>,
    /// Lazily initialized on the first diarization request — model load
    /// takes seconds and is wasted when diarization is off.
    sortformer: Option<Sortformer>,
    /// Optional CoreML model-cache directory. When set and the `coreml`
    /// feature is enabled, parakeet-rs caches compiled CoreML graphs here
    /// so model loads after the first one are sub-second.
    coreml_cache_dir: Option<PathBuf>,
}

impl ParakeetBackend {
    pub fn new(sortformer_model_path: Option<PathBuf>, coreml_cache_dir: Option<PathBuf>) -> Self {
        Self {
            model: None,
            sortformer_model_path,
            sortformer: None,
            coreml_cache_dir,
        }
    }

    fn ensure_sortformer(&mut self) -> Result<&mut Sortformer, String> {
        if self.sortformer.is_none() {
            let path = self
                .sortformer_model_path
                .as_ref()
                .ok_or("sortformer model not provided to sidecar")?;
            info!(
                "loading sortformer diarization model from {}",
                path.display()
            );
            let s = Sortformer::new(path)
                .map_err(|e| format!("failed to load sortformer model: {e}"))?;
            self.sortformer = Some(s);
        }
        Ok(self.sortformer.as_mut().unwrap())
    }
}

impl TranscriptionBackend for ParakeetBackend {
    fn load_model(&mut self, model_path: &Path) -> Result<(), String> {
        if !model_path.is_dir() {
            return Err(format!(
                "parakeet model path must be a directory containing encoder-model.onnx and \
                 decoder_joint-model.onnx, got: {}",
                model_path.display()
            ));
        }
        let load_start = std::time::Instant::now();
        info!(
            "loading parakeet TDT model from {} (coreml_cache={:?})",
            model_path.display(),
            self.coreml_cache_dir.as_deref()
        );

        // ORT+CoreML deterministically fails on models with external `.onnx.data`
        // initializer files (the `model_path must not be empty` error path during
        // CoreML cache write). The Auto policy skips CoreML up front when we can
        // see them on disk — saves ~600 ms of doomed load attempt and the noisy
        // ERROR log on every spawn. The user can still force CoreML or WebGPU
        // via `YAPSTACK_PARAKEET_ACCEL`; we honor that even on incompatible
        // models so the failure mode is observable.
        let cache_dir = self.coreml_cache_dir.as_deref();
        let choice = AccelChoice::from_env();
        let exec_config = match choice {
            AccelChoice::Auto if has_external_data_file(model_path) => {
                if cache_dir.is_some() {
                    info!(
                        "external `.onnx.data` initializer present — Auto policy \
                         skipping CoreML and loading on CPU"
                    );
                }
                None
            }
            _ => build_exec_config(cache_dir),
        };
        let accel_attempted = exec_config.is_some();
        let model = match ParakeetTDT::from_pretrained(model_path, exec_config) {
            Ok(m) => m,
            // Defensive: if the chosen accelerator fails at load (CoreML on a
            // model with external initializers, WebGPU not available, etc.),
            // fall back to plain CPU rather than refusing to load entirely.
            Err(e) if accel_attempted => {
                warn!(
                    "accelerator load failed ({e}); falling back to CPU \
                     (set YAPSTACK_PARAKEET_ACCEL=cpu to suppress this attempt)"
                );
                ParakeetTDT::from_pretrained(model_path, None)
                    .map_err(|e2| format!("failed to load parakeet model (CPU fallback): {e2}"))?
            }
            Err(e) => return Err(format!("failed to load parakeet model: {e}")),
        };
        self.model = Some(model);
        info!(
            "parakeet TDT model loaded in {} ms",
            load_start.elapsed().as_millis()
        );
        Ok(())
    }

    fn transcribe(
        &mut self,
        audio_path: &Path,
        opts: TranscribeOpts<'_>,
    ) -> Result<TranscriptionOutput, String> {
        let model = self.model.as_mut().ok_or("no model loaded")?;

        let mut reader =
            hound::WavReader::open(audio_path).map_err(|e| format!("failed to open WAV: {e}"))?;
        let spec = reader.spec();

        let raw_samples: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
            reader
                .samples::<f32>()
                .map(|s| s.map_err(|e| format!("sample read error: {e}")))
                .collect::<Result<Vec<f32>, String>>()?
        } else {
            reader
                .samples::<i16>()
                .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
                .map(|s| s.map_err(|e| format!("sample read error: {e}")))
                .collect::<Result<Vec<f32>, String>>()?
        };

        let audio_seconds = raw_samples.len() as f32 / spec.sample_rate as f32;
        debug!(
            "parakeet transcribe: {} samples ({:.2}s) at {}Hz x {}ch (diarization={})",
            raw_samples.len(),
            audio_seconds,
            spec.sample_rate,
            spec.channels,
            opts.diarization
        );

        // parakeet-rs requires 16 kHz mono. The shared resample helper is a
        // no-op (Cow::Borrowed) when the input is already at the target rate.
        let mono = yapstack_common::audio::deinterleave_to_mono(&raw_samples, spec.channels);
        let resampled =
            yapstack_common::audio::resample(&mono, spec.sample_rate, PARAKEET_SAMPLE_RATE)
                .map_err(|e| format!("resample to 16kHz for parakeet failed: {e}"))?;
        let model_input: Vec<f32> = resampled.into_owned();

        let start = std::time::Instant::now();
        let result = model
            .transcribe_samples(
                model_input,
                PARAKEET_SAMPLE_RATE,
                1,
                Some(TimestampMode::Words),
            )
            .map_err(|e| format!("parakeet transcription failed: {e}"))?;
        let _ = opts.language; // TDT v3 auto-detects; explicit hint not yet exposed by parakeet-rs.
        let _ = opts.initial_prompt; // TDT decoder has no prompt input.
        let _ = opts.single_segment; // We always group below per silence gaps.
        let elapsed_ms = start.elapsed().as_millis();
        let rtfx = if elapsed_ms > 0 {
            (audio_seconds * 1000.0) / elapsed_ms as f32
        } else {
            f32::INFINITY
        };
        info!(
            "parakeet transcribe: {} tokens in {} ms ({:.1}x realtime)",
            result.tokens.len(),
            elapsed_ms,
            rtfx
        );

        let segments = group_tokens_into_segments(&result.tokens);
        let mut text = String::new();
        let mut filtered: Vec<TranscriptSegment> = Vec::with_capacity(segments.len());
        for seg in segments {
            // Parakeet doesn't expose per-segment confidence; assume high
            // (the model rarely produces YouTube-style hallucinations).
            let confidence = 1.0_f32;
            let normalized = normalize_spacing(seg.text.trim());
            let sanitized = sanitize_text(&normalized);
            if !should_include_segment(&sanitized, confidence) {
                info!("parakeet segment filtered: {:?}", sanitized);
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&sanitized);
            filtered.push(TranscriptSegment {
                start_ms: (seg.start_secs * 1000.0).round() as u64,
                end_ms: (seg.end_secs * 1000.0).round() as u64,
                text: sanitized,
                confidence,
                speaker_id: None,
            });
        }

        // Optional diarization pass.
        if opts.diarization {
            match self.run_diarization(&raw_samples, spec.sample_rate, spec.channels) {
                Ok(speaker_segs) => {
                    debug!(
                        "sortformer produced {} speaker segments",
                        speaker_segs.len()
                    );
                    assign_speakers(&mut filtered, &speaker_segs);
                }
                Err(e) => {
                    warn!("diarization failed; returning transcript without speaker IDs: {e}");
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(TranscriptionOutput {
            text,
            segments: filtered,
            duration_ms,
        })
    }
}

impl ParakeetBackend {
    /// Resample to 16 kHz mono if needed, then run Sortformer.
    fn run_diarization(
        &mut self,
        raw_samples: &[f32],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Vec<SpeakerSegment>, String> {
        let mono = yapstack_common::audio::deinterleave_to_mono(raw_samples, channels);
        let resampled =
            yapstack_common::audio::resample(&mono, sample_rate, SORTFORMER_SAMPLE_RATE)
                .map_err(|e| format!("resample to 16kHz for diarization failed: {e}"))?;
        let audio: Vec<f32> = resampled.into_owned();
        let sortformer = self.ensure_sortformer()?;
        sortformer
            .diarize(audio, SORTFORMER_SAMPLE_RATE, 1)
            .map_err(|e| format!("sortformer diarize failed: {e}"))
    }
}

/// True iff `model_dir` contains any `.onnx.data` (or `.onnx_data`) sidecar
/// file — the deterministic signature that ORT+CoreML will fail with the
/// `model_path must not be empty` initializer error.
fn has_external_data_file(model_dir: &Path) -> bool {
    let entries = match std::fs::read_dir(model_dir) {
        Ok(e) => e,
        Err(e) => {
            debug!(
                "has_external_data_file: read_dir({}) failed: {e}",
                model_dir.display()
            );
            return false;
        }
    };
    let mut found = false;
    for entry in entries.flatten() {
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        debug!("has_external_data_file: saw {}", name);
        if name.ends_with(".onnx.data") || name.ends_with(".onnx_data") {
            found = true;
        }
    }
    found
}

/// Which Parakeet execution provider to use. Controlled by the
/// `YAPSTACK_PARAKEET_ACCEL` env var; default is `Auto` which today means
/// "CoreML if no external `.onnx.data` files, else CPU".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccelChoice {
    Auto,
    Cpu,
    #[cfg(feature = "coreml")]
    CoreMl,
    #[cfg(feature = "webgpu")]
    WebGpu,
}

impl AccelChoice {
    fn from_env() -> Self {
        let raw = std::env::var("YAPSTACK_PARAKEET_ACCEL").unwrap_or_default();
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Self::Auto,
            "cpu" => Self::Cpu,
            #[cfg(feature = "coreml")]
            "coreml" => Self::CoreMl,
            #[cfg(not(feature = "coreml"))]
            "coreml" => {
                warn!("YAPSTACK_PARAKEET_ACCEL=coreml ignored: feature not compiled in");
                Self::Auto
            }
            #[cfg(feature = "webgpu")]
            "webgpu" => Self::WebGpu,
            #[cfg(not(feature = "webgpu"))]
            "webgpu" => {
                warn!("YAPSTACK_PARAKEET_ACCEL=webgpu ignored: feature not compiled in");
                Self::Auto
            }
            other => {
                warn!("YAPSTACK_PARAKEET_ACCEL={other:?} unrecognised; defaulting to Auto");
                Self::Auto
            }
        }
    }
}

/// Build the parakeet-rs `ExecutionConfig` for `from_pretrained`.
/// Returns `None` to mean "let parakeet-rs use its CPU default".
fn build_exec_config(cache_dir: Option<&Path>) -> Option<parakeet_rs::ExecutionConfig> {
    let choice = AccelChoice::from_env();
    info!("parakeet accelerator choice: {:?}", choice);
    match choice {
        AccelChoice::Cpu => None,
        AccelChoice::Auto => {
            // Auto: prefer CoreML when compiled in (the load_model preflight
            // skips it for known-incompatible models). Otherwise CPU.
            #[cfg(feature = "coreml")]
            {
                build_coreml_config(cache_dir)
            }
            #[cfg(not(feature = "coreml"))]
            {
                let _ = cache_dir;
                None
            }
        }
        #[cfg(feature = "coreml")]
        AccelChoice::CoreMl => build_coreml_config(cache_dir),
        #[cfg(feature = "webgpu")]
        AccelChoice::WebGpu => build_webgpu_config(),
    }
}

#[cfg(feature = "coreml")]
fn build_coreml_config(cache_dir: Option<&Path>) -> Option<parakeet_rs::ExecutionConfig> {
    use parakeet_rs::{ExecutionConfig, ExecutionProvider};
    let mut cfg = ExecutionConfig::default().with_execution_provider(ExecutionProvider::CoreML);
    if let Some(dir) = cache_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!(
                "failed to create CoreML cache dir {} ({e}); proceeding without cache",
                dir.display()
            );
        } else {
            info!("CoreML cache dir: {}", dir.display());
            cfg = cfg.with_coreml_cache_dir(dir);
        }
    } else {
        info!("CoreML enabled, no cache dir provided — recompile cost on every spawn");
    }
    Some(cfg)
}

#[cfg(feature = "webgpu")]
fn build_webgpu_config() -> Option<parakeet_rs::ExecutionConfig> {
    use parakeet_rs::{ExecutionConfig, ExecutionProvider};
    info!("WebGPU EP requested (Metal under the hood on macOS)");
    Some(ExecutionConfig::default().with_execution_provider(ExecutionProvider::WebGPU))
}

/// Assign each transcript segment a `speaker_id` based on the maximum-overlap
/// speaker range from Sortformer. Segments with no overlap (e.g. between
/// detected speech regions) keep `speaker_id = None`.
fn assign_speakers(segments: &mut [TranscriptSegment], speakers: &[SpeakerSegment]) {
    for seg in segments {
        let mut best: Option<(u64, u8)> = None;
        for sp in speakers {
            // Sortformer ranges are in samples at 16kHz; convert to ms.
            let sp_start_ms = sp.start * 1000 / SORTFORMER_SAMPLE_RATE as u64;
            let sp_end_ms = sp.end * 1000 / SORTFORMER_SAMPLE_RATE as u64;
            let overlap_start = seg.start_ms.max(sp_start_ms);
            let overlap_end = seg.end_ms.min(sp_end_ms);
            if overlap_end > overlap_start {
                let overlap = overlap_end - overlap_start;
                if best.is_none_or(|(b, _)| overlap > b) {
                    best = Some((overlap, sp.speaker_id as u8));
                }
            }
        }
        if let Some((_, id)) = best {
            seg.speaker_id = Some(id);
        }
    }
}

/// Aggregated word-level tokens that look like a "Whisper segment" to the
/// rest of the pipeline. Internal — not part of the IPC protocol.
struct GroupedSegment {
    text: String,
    start_secs: f32,
    end_secs: f32,
}

/// Group word-level [`TimedToken`]s into segments at silence gaps and at the
/// soft per-segment time cap, mirroring Whisper's per-segment cadence so the
/// downstream UI can render either engine identically.
fn group_tokens_into_segments(tokens: &[TimedToken]) -> Vec<GroupedSegment> {
    let mut out: Vec<GroupedSegment> = Vec::new();
    let mut current: Option<GroupedSegment> = None;

    for tok in tokens {
        if tok.text.is_empty() {
            continue;
        }
        match current.as_mut() {
            None => {
                current = Some(GroupedSegment {
                    text: tok.text.trim().to_string(),
                    start_secs: tok.start,
                    end_secs: tok.end,
                });
            }
            Some(seg) => {
                let gap = tok.start - seg.end_secs;
                let span = tok.end - seg.start_secs;
                let break_for_silence = gap > SEGMENT_GAP_SECS;
                let break_for_length = span > MAX_SEGMENT_SECS;
                if break_for_silence || break_for_length {
                    out.push(current.take().unwrap());
                    current = Some(GroupedSegment {
                        text: tok.text.trim().to_string(),
                        start_secs: tok.start,
                        end_secs: tok.end,
                    });
                } else {
                    let needs_space = !seg.text.is_empty()
                        && !tok
                            .text
                            .trim_start()
                            .starts_with(['.', ',', '!', '?', ';', ':', ')']);
                    if needs_space {
                        seg.text.push(' ');
                    }
                    seg.text.push_str(tok.text.trim());
                    seg.end_secs = tok.end;
                }
            }
        }
    }
    if let Some(seg) = current {
        out.push(seg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(text: &str, start: f32, end: f32) -> TimedToken {
        TimedToken {
            text: text.to_string(),
            start,
            end,
        }
    }

    #[test]
    fn groups_single_phrase_without_gap() {
        let segs = group_tokens_into_segments(&[tok("Hello", 0.0, 0.4), tok("world", 0.5, 0.9)]);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "Hello world");
        assert_eq!(segs[0].start_secs, 0.0);
        assert_eq!(segs[0].end_secs, 0.9);
    }

    #[test]
    fn breaks_segment_on_long_silence() {
        let segs = group_tokens_into_segments(&[
            tok("First", 0.0, 0.3),
            tok("phrase", 0.4, 0.7),
            // 1.5s silence gap — well above SEGMENT_GAP_SECS
            tok("Second", 2.2, 2.5),
            tok("phrase", 2.6, 2.9),
        ]);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "First phrase");
        assert_eq!(segs[1].text, "Second phrase");
    }

    #[test]
    fn breaks_segment_at_length_cap() {
        // Synthesize a long monologue: one token per second for 14s with no gaps.
        let toks: Vec<TimedToken> = (0..14)
            .map(|i| tok("word", i as f32, (i + 1) as f32 - 0.05))
            .collect();
        let segs = group_tokens_into_segments(&toks);
        assert!(segs.len() >= 2, "expected length cap to force a break");
    }

    #[test]
    fn punctuation_does_not_get_extra_space() {
        let segs = group_tokens_into_segments(&[
            tok("Hello", 0.0, 0.3),
            tok(",", 0.31, 0.32),
            tok("world", 0.4, 0.7),
            tok(".", 0.71, 0.72),
        ]);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "Hello, world.");
    }

    #[test]
    fn empty_token_input_yields_no_segments() {
        let segs = group_tokens_into_segments(&[]);
        assert!(segs.is_empty());
    }

    fn ts(start_ms: u64, end_ms: u64) -> TranscriptSegment {
        TranscriptSegment {
            start_ms,
            end_ms,
            text: "x".to_string(),
            confidence: 1.0,
            speaker_id: None,
        }
    }

    fn sp(start_secs: f32, end_secs: f32, id: usize) -> SpeakerSegment {
        SpeakerSegment {
            start: (start_secs * SORTFORMER_SAMPLE_RATE as f32) as u64,
            end: (end_secs * SORTFORMER_SAMPLE_RATE as f32) as u64,
            speaker_id: id,
        }
    }

    #[test]
    fn assigns_speaker_with_full_overlap() {
        let mut segs = vec![ts(0, 1000), ts(2000, 3000)];
        let speakers = vec![sp(0.0, 1.5, 0), sp(1.6, 3.2, 1)];
        assign_speakers(&mut segs, &speakers);
        assert_eq!(segs[0].speaker_id, Some(0));
        assert_eq!(segs[1].speaker_id, Some(1));
    }

    #[test]
    fn assigns_speaker_with_max_overlap_when_segments_straddle() {
        // Transcript segment 0..2000ms straddles two speakers; speaker 1 owns more of it.
        let mut segs = vec![ts(0, 2000)];
        let speakers = vec![sp(0.0, 0.5, 0), sp(0.5, 2.0, 1)];
        assign_speakers(&mut segs, &speakers);
        assert_eq!(segs[0].speaker_id, Some(1));
    }

    #[test]
    fn leaves_speaker_none_when_no_overlap() {
        let mut segs = vec![ts(5000, 6000)];
        let speakers = vec![sp(0.0, 1.0, 0)];
        assign_speakers(&mut segs, &speakers);
        assert_eq!(segs[0].speaker_id, None);
    }

    #[test]
    fn assign_speakers_with_empty_diarization_is_noop() {
        let mut segs = vec![ts(0, 1000), ts(1000, 2000)];
        assign_speakers(&mut segs, &[]);
        assert!(segs.iter().all(|s| s.speaker_id.is_none()));
    }
}
