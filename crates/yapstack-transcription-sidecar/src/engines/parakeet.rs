//! Parakeet TDT-v3 backend (parakeet-rs 0.3 + ONNX Runtime via `ort`).
//!
//! Sortformer-based speaker diarization no longer lives here — it runs as
//! a shared post-pass at the dispatcher level (see `crate::diarization`)
//! so Whisper sessions get the same treatment.

use std::path::{Path, PathBuf};

use parakeet_rs::{ParakeetTDT, TimedToken, TimestampMode, Transcriber};
use tracing::{debug, info, warn};
use yapstack_common::types::TranscriptSegment;

use crate::engines::{
    normalize_spacing, sanitize_text, should_include_segment, EngineInfo, TranscribeOpts,
    TranscriptionBackend, TranscriptionOutput,
};

/// parakeet-rs's `transcribe_samples` rejects anything other than 16 kHz mono
/// despite docs claiming auto-resampling (verified against parakeet-rs 0.3.5).
const PARAKEET_SAMPLE_RATE: u32 = 16000;

const SEGMENT_GAP_SECS: f32 = 0.5;
const MAX_SEGMENT_SECS: f32 = 12.0;

pub struct ParakeetBackend {
    model: Option<ParakeetTDT>,
    coreml_cache_dir: Option<PathBuf>,
    /// Set after each successful `load_model` so `engine_info()` can
    /// report what's actually running. Cleared if reload fails.
    last_engine_info: Option<EngineInfo>,
}

impl ParakeetBackend {
    pub fn new(coreml_cache_dir: Option<PathBuf>) -> Self {
        Self {
            model: None,
            coreml_cache_dir,
            last_engine_info: None,
        }
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

        let cache_dir = self.coreml_cache_dir.as_deref();
        let choice = AccelChoice::from_env();
        // Auto: skip CoreML when external `.onnx.data` is present (known ORT
        // bug `model_path must not be empty`). The skip ONLY applies when
        // CoreML is the auto-selected EP — WebGPU runs through a different
        // ORT codepath that doesn't try to serialize the partitioned subgraph
        // back through the model path, so it's safe with external data.
        // User-forced choices are honored regardless (so a hand-set
        // `YAPSTACK_PARAKEET_ACCEL=coreml` will still try CoreML even on a
        // fp32 bundle, fail, and hit the fallback chain below).
        let exec_config = match choice {
            AccelChoice::Auto if auto_skip_due_to_external_data(model_path) => None,
            _ => build_exec_config(choice, cache_dir),
        };
        let accel_attempted = exec_config.is_some();
        let attempted_label = if accel_attempted {
            resolved_accel_label(choice)
        } else {
            "cpu"
        };
        info!(
            marker = "live_accel_choice",
            engine = "parakeet",
            requested = ?choice,
            attempted = attempted_label,
            model_dir = %model_path.display(),
            "parakeet acceleration resolved"
        );
        // `actual_label` records which EP actually took the model — it
        // diverges from `attempted_label` only on the fallback path below.
        let (model, actual_label) = match ParakeetTDT::from_pretrained(model_path, exec_config) {
            Ok(m) => (m, attempted_label.to_string()),
            Err(e) if accel_attempted => {
                warn!(
                    marker = "live_accel_fallback",
                    engine = "parakeet",
                    requested = attempted_label,
                    error = %e,
                    "accelerator load failed; falling back to CPU \
                     (set YAPSTACK_PARAKEET_ACCEL=cpu to suppress this attempt)"
                );
                let m = ParakeetTDT::from_pretrained(model_path, None)
                    .map_err(|e2| format!("failed to load parakeet model (CPU fallback): {e2}"))?;
                (m, "cpu".to_string())
            }
            Err(e) => return Err(format!("failed to load parakeet model: {e}")),
        };
        self.model = Some(model);
        self.last_engine_info = Some(EngineInfo {
            accel: actual_label,
            model_dir: model_path.display().to_string(),
        });
        info!(
            "parakeet TDT model loaded in {} ms",
            load_start.elapsed().as_millis()
        );
        Ok(())
    }

    fn engine_info(&self) -> Option<EngineInfo> {
        self.last_engine_info.clone()
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        opts: TranscribeOpts<'_>,
    ) -> Result<TranscriptionOutput, String> {
        let model = self.model.as_mut().ok_or("no model loaded")?;

        let audio_seconds = samples.len() as f32 / PARAKEET_SAMPLE_RATE as f32;
        debug!(
            "parakeet transcribe: {} samples ({:.2}s) at 16kHz mono",
            samples.len(),
            audio_seconds
        );

        _ = opts.language;
        _ = opts.initial_prompt;
        _ = opts.single_segment;
        _ = opts.diarization;

        let start = std::time::Instant::now();
        // parakeet-rs takes ownership of the sample buffer; clone the
        // dispatcher-owned slice so the same buffer can be reused for the
        // diarization post-pass.
        let result = model
            .transcribe_samples(
                samples.to_vec(),
                PARAKEET_SAMPLE_RATE,
                1,
                Some(TimestampMode::Words),
            )
            .map_err(|e| format!("parakeet transcription failed: {e}"))?;
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
            // parakeet-rs's `TimedToken` has no logprob/confidence field — the
            // library doesn't expose one. We pass 1.0 so the shared
            // `should_include_segment` contract still works, and accept the
            // consequence: the confidence-gated branches (marginal-reject at
            // < 0.6, repetition filter at < 0.7) never fire for Parakeet
            // output. Parakeet's narrower always-reject list was designed as
            // the sole filter precisely because Parakeet doesn't hallucinate
            // the YouTube artifacts Whisper does. If a future parakeet-rs
            // release exposes token probabilities we should route a derived
            // confidence here so the lower tiers start firing.
            let confidence = 1.0_f32;
            let normalized = normalize_spacing(seg.text.trim());
            let sanitized = sanitize_text(&normalized);
            if !should_include_segment(
                &sanitized,
                confidence,
                yapstack_common::types::EngineKind::Parakeet,
            ) {
                info!(len = sanitized.chars().count(), "parakeet segment filtered");
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

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(TranscriptionOutput {
            text,
            segments: filtered,
            duration_ms,
        })
    }
}

fn has_external_data_file(model_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(model_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.ends_with(".onnx.data") || name.ends_with(".onnx_data")
    })
}

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

/// Stable string label for the accelerator that an `AccelChoice` resolves
/// to under the current cfg. Matches what `auto_exec_config` actually
/// builds for `AccelChoice::Auto`. Used for the `ModelLoaded.accel`
/// telemetry payload — kept lowercase + ASCII for grep-friendliness.
fn resolved_accel_label(choice: AccelChoice) -> &'static str {
    match choice {
        AccelChoice::Cpu => "cpu",
        AccelChoice::Auto => {
            #[cfg(all(target_os = "macos", feature = "webgpu"))]
            {
                "webgpu"
            }
            #[cfg(all(target_os = "macos", not(feature = "webgpu"), feature = "coreml"))]
            {
                "coreml"
            }
            #[cfg(not(target_os = "macos"))]
            {
                "cpu"
            }
            #[cfg(all(target_os = "macos", not(feature = "webgpu"), not(feature = "coreml")))]
            {
                "cpu"
            }
        }
        #[cfg(feature = "coreml")]
        AccelChoice::CoreMl => "coreml",
        #[cfg(feature = "webgpu")]
        AccelChoice::WebGpu => "webgpu",
    }
}

/// Returns true when the model dir contains an external `.onnx.data` file
/// AND the auto-selected accelerator would be a CoreML-style EP that
/// stumbles on the "model_path must not be empty" partitioning bug. With
/// the new Apple Silicon default of WebGPU, this only fires when CoreML
/// is the only EP compiled in (legacy/fallback build) — which means the
/// fp32 bundle on those builds keeps its CPU-only behavior.
fn auto_skip_due_to_external_data(model_path: &Path) -> bool {
    if !has_external_data_file(model_path) {
        return false;
    }
    // If WebGPU is compiled in, Auto will pick WebGPU over CoreML on Apple
    // Silicon — and WebGPU is safe with external data, so don't skip.
    #[cfg(all(target_os = "macos", feature = "webgpu"))]
    {
        return false;
    }
    #[cfg(not(all(target_os = "macos", feature = "webgpu")))]
    {
        // Auto would pick CoreML on Apple Silicon (legacy / no-webgpu build)
        // OR fall through to CPU on other platforms. Keep the skip on
        // CoreML-only Apple builds; harmless no-op elsewhere.
        true
    }
}

fn build_exec_config(
    choice: AccelChoice,
    cache_dir: Option<&Path>,
) -> Option<parakeet_rs::ExecutionConfig> {
    info!("parakeet accelerator choice: {:?}", choice);
    match choice {
        AccelChoice::Cpu => {
            _ = cache_dir;
            None
        }
        AccelChoice::Auto => auto_exec_config(cache_dir),
        #[cfg(feature = "coreml")]
        AccelChoice::CoreMl => build_coreml_config(cache_dir),
        #[cfg(feature = "webgpu")]
        AccelChoice::WebGpu => build_webgpu_config(),
    }
}

/// Apple Silicon defaults to WebGPU (Dawn → Metal). parakeet-rs's source
/// notes that CoreML EP currently runs slower than CPU on Parakeet because
/// the ONNX graphs have dynamic input shapes (CoreML claims the nodes but
/// runs them on CPU with overhead), and parakeet-rs hardcodes
/// `ComputeUnits::CPUAndGPU` rather than `CPUAndNeuralEngine` — so CoreML
/// is not the path to ANE acceleration today. WebGPU bypasses that issue
/// at the cost of going through Metal instead of Apple's CoreML compiler.
/// Other targets fall through to CPU; Windows CUDA arrives in a follow-up.
fn auto_exec_config(cache_dir: Option<&Path>) -> Option<parakeet_rs::ExecutionConfig> {
    #[cfg(all(target_os = "macos", feature = "webgpu"))]
    {
        _ = cache_dir;
        return build_webgpu_config();
    }
    #[cfg(all(target_os = "macos", not(feature = "webgpu"), feature = "coreml"))]
    {
        return build_coreml_config(cache_dir);
    }
    #[cfg(not(target_os = "macos"))]
    {
        _ = cache_dir;
        None
    }
    #[cfg(all(target_os = "macos", not(feature = "webgpu"), not(feature = "coreml")))]
    {
        _ = cache_dir;
        None
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

struct GroupedSegment {
    text: String,
    start_secs: f32,
    end_secs: f32,
}

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
}
