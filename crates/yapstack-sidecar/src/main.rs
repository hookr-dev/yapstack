use std::path::PathBuf;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(feature = "whisper")]
use tracing::debug;
use tracing::{error, info};
#[cfg(feature = "whisper")]
use yapstack_common::types::TranscriptSegment;
use yapstack_common::types::{SidecarRequest, SidecarResponse};

#[cfg(feature = "whisper")]
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadParams,
};

#[cfg(feature = "whisper")]
const WHISPER_SAMPLE_RATE: u32 = 16000;

#[cfg(feature = "whisper")]
struct TranscriptionEngine {
    ctx: WhisperContext,
}

/// Convert interleaved multi-channel audio to mono by averaging channels.
/// Returns `Cow::Borrowed` when already mono.
#[cfg(feature = "whisper")]
fn to_mono(samples: &[f32], channels: u16) -> std::borrow::Cow<'_, [f32]> {
    yapstack_common::audio::deinterleave_to_mono(samples, channels)
}

/// Resample audio using sinc interpolation (delegates to `yapstack_common::audio::resample`).
/// Returns `Ok(Cow::Borrowed)` when `from_rate == to_rate`.
#[cfg(feature = "whisper")]
fn resample(
    samples: &[f32],
    from_rate: u32,
    to_rate: u32,
) -> Result<std::borrow::Cow<'_, [f32]>, yapstack_common::audio::ResampleError> {
    yapstack_common::audio::resample(samples, from_rate, to_rate)
}

/// Insert a space after sentence-ending punctuation (`.` `!` `?`) or commas when
/// immediately followed by an uppercase letter.  Fixes a known Whisper tokenizer
/// issue where adjacent sentences are joined without whitespace
/// (e.g. "First sentence.Then second" or "word,Another").
/// Preserves abbreviations like "e.g." and "U.S.A." because they are followed by
/// lowercase letters or more punctuation, not an uppercase letter.
#[cfg(any(feature = "whisper", test))]
fn normalize_spacing(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 10);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        result.push(ch);
        if ch == '.' || ch == '!' || ch == '?' || ch == ',' {
            if let Some(&next) = chars.peek() {
                if next.is_uppercase() {
                    result.push(' ');
                }
            }
        }
    }
    result
}

/// Replace common Unicode punctuation with ASCII equivalents, strip remaining
/// non-ASCII characters, and collapse runs of multiple spaces.  Whisper occasionally
/// outputs curly quotes, em-dashes, and other Unicode artifacts.
#[cfg(any(feature = "whisper", test))]
fn sanitize_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        match ch {
            '\u{2018}' | '\u{2019}' => {
                result.push('\'');
                prev_space = false;
            }
            '\u{201C}' | '\u{201D}' => {
                result.push('"');
                prev_space = false;
            }
            '\u{2014}' => {
                result.push_str("--");
                prev_space = false;
            }
            '\u{2013}' => {
                result.push('-');
                prev_space = false;
            }
            '\u{2026}' => {
                result.push_str("...");
                prev_space = false;
            }
            ' ' => {
                if !prev_space {
                    result.push(' ');
                }
                prev_space = true;
            }
            c if c.is_ascii() => {
                result.push(c);
                prev_space = false;
            }
            _ => {} // strip non-ASCII artifacts
        }
    }
    result
}

/// Normalize text for repetition detection by inserting spaces around punctuation.
/// "Yeah.Yeah.Yeah." → "Yeah . Yeah . Yeah ." so whitespace-based splitting catches loops.
#[cfg(any(feature = "whisper", test))]
fn normalize_for_repetition(text: &str) -> String {
    let mut result = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if ch.is_ascii_punctuation() && ch != '\'' {
            if !result.ends_with(' ') {
                result.push(' ');
            }
            result.push(ch);
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Detect if text contains excessive repetition (e.g., "the the the the").
#[cfg(any(feature = "whisper", test))]
fn has_excessive_repetition(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 3 {
        return false;
    }

    // Check for single-word repetition (3+ consecutive identical words)
    let mut consecutive = 1;
    for pair in words.windows(2) {
        if pair[0].eq_ignore_ascii_case(pair[1]) {
            consecutive += 1;
            if consecutive >= 3 {
                return true;
            }
        } else {
            consecutive = 1;
        }
    }

    // Check for phrase repetition: if the first N words repeat to fill the text
    for phrase_len in 1..=words.len() / 2 {
        let phrase = &words[..phrase_len];
        let all_repeat = words.chunks(phrase_len).all(|chunk| {
            chunk
                .iter()
                .zip(phrase.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        });
        if all_repeat && words.len() / phrase_len >= 3 {
            return true;
        }
    }

    false
}

/// Returns true if a transcript segment should be included in the output.
/// Filters out empty text, special tokens, low-confidence segments,
/// known Whisper hallucination patterns at marginal confidence, and
/// excessive word/phrase repetition.
#[cfg(any(feature = "whisper", test))]
fn should_include_segment(text: &str, confidence: f32) -> bool {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return false;
    }

    // Reject segments with no alphanumeric content (pure punctuation/symbols)
    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }

    // Skip special tokens like [BLANK_AUDIO], [MUSIC], etc.
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return false;
    }

    // Reject segments with excessive word/phrase repetition
    // Check both raw text and normalized (punctuation-spaced) text to catch
    // loops like "Yeah.Yeah.Yeah." that have no whitespace separators.
    if has_excessive_repetition(trimmed) {
        info!("repetition filtered: {:?}", trimmed);
        return false;
    }
    let normalized = normalize_for_repetition(trimmed);
    if has_excessive_repetition(&normalized) {
        info!("repetition filtered (normalized): {:?}", trimmed);
        return false;
    }

    // Drop low-confidence segments (high no_speech_probability)
    if confidence < 0.4 {
        return false;
    }

    // Always reject YouTube/podcast hallucination artifacts regardless of confidence
    if yapstack_common::hallucination::is_always_reject(trimmed) {
        return false;
    }

    // Filter marginal patterns (single-word fillers) at low confidence only
    if confidence < 0.6 && yapstack_common::hallucination::is_marginal_reject(trimmed) {
        return false;
    }

    true
}

#[cfg(feature = "whisper")]
impl TranscriptionEngine {
    fn load(model_path: &std::path::Path) -> Result<Self, String> {
        let mut ctx_params = WhisperContextParameters::default();
        ctx_params.flash_attn(true);

        info!(
            "whisper context: use_gpu={}, flash_attn={}, gpu_device={}",
            ctx_params.use_gpu, ctx_params.flash_attn, ctx_params.gpu_device
        );

        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or("invalid model path")?,
            ctx_params,
        )
        .map_err(|e| format!("failed to load model: {e}"))?;
        Ok(Self { ctx })
    }

    fn transcribe(
        &self,
        audio_path: &std::path::Path,
        language: Option<&str>,
        initial_prompt: Option<&str>,
        single_segment: Option<bool>,
        vad_model_path: Option<&std::path::Path>,
    ) -> Result<(String, Vec<TranscriptSegment>, u64), String> {
        // Read WAV file
        let mut reader =
            hound::WavReader::open(audio_path).map_err(|e| format!("failed to open WAV: {e}"))?;

        let spec = reader.spec();

        // Convert to f32 samples (interleaved if multi-channel)
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

        // Whisper expects 16 kHz mono — convert if needed (zero-copy when already mono/16kHz)
        let mono = to_mono(&raw_samples, spec.channels);
        let samples = resample(&mono, spec.sample_rate, WHISPER_SAMPLE_RATE)
            .map_err(|e| format!("resample to 16kHz failed: {e}"))?;

        let start = std::time::Instant::now();

        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        if let Some(lang) = language {
            params.set_language(Some(lang));
        }
        if let Some(prompt) = initial_prompt {
            params.set_initial_prompt(prompt);
        }
        params.set_n_threads(n_threads);
        params.set_temperature(0.0);
        params.set_no_speech_thold(0.45);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);

        // Temperature fallback: start greedy (0.0), increment by 0.2 on failure.
        // whisper.cpp retries when a segment has high entropy or low avg logprob,
        // recovering speech from noisy/ambiguous audio (Zoom artifacts, typing, AC).
        params.set_temperature_inc(0.2);

        // Entropy threshold — reject segments with repetitive/compressible text
        // Default is 2.4; lowering to 2.0 catches repetition loops while keeping real speech
        params.set_entropy_thold(2.0);

        // Log probability threshold — use whisper.cpp default (-1.0).
        // Previously -0.5 which was too aggressive, causing legitimate segments
        // to be rejected before reaching our post-filter.
        params.set_logprob_thold(-1.0);

        // Adaptive single_segment: for short chunks (<10s), force single segment to
        // avoid over-segmentation. For longer chunks (e.g. 30s force-chunks), let
        // Whisper segment naturally to preserve sentence boundaries.
        let audio_duration_s = samples.len() as f32 / WHISPER_SAMPLE_RATE as f32;
        let use_single_segment = single_segment.unwrap_or(audio_duration_s < 10.0);
        params.set_single_segment(use_single_segment);

        // Cap decoder output per segment to prevent runaway repetition loops.
        // Normal 30s speech is well under 200 tokens. Higher limit needed when
        // single_segment=false to allow natural multi-sentence segmentation.
        params.set_max_tokens(if use_single_segment { 100 } else { 200 });

        debug!(
            "whisper params: n_threads={}, best_of=1, temperature=0.0, temperature_inc=0.2, no_speech_thold=0.45, entropy_thold=2.0, logprob_thold=-1.0, single_segment={}, max_tokens={}",
            n_threads,
            use_single_segment,
            if use_single_segment { 100 } else { 200 },
        );

        // Disable cross-segment context — we already pass curated initial_prompt
        // (350 chars of prior transcript) from the live transcription controller.
        // When false, Whisper feeds its own prior decoded text back as context,
        // creating a feedback loop that amplifies hallucinations.
        params.set_no_context(true);

        // Enable Silero VAD if model is available — whisper.cpp skips non-speech
        // regions before decoding, which is the strongest defense against hallucinations.
        if let Some(vad_path) = vad_model_path {
            if let Some(vad_str) = vad_path.to_str() {
                params.set_vad_model_path(Some(vad_str));
                let mut vad_params = WhisperVadParams::new();
                vad_params.set_threshold(0.5);
                vad_params.set_min_speech_duration(250);
                vad_params.set_min_silence_duration(100);
                vad_params.set_speech_pad(30);
                params.set_vad_params(vad_params);
                params.enable_vad(true);
                info!("VAD enabled for transcription");
            } else {
                info!("VAD model path contains non-UTF-8 characters, skipping VAD");
            }
        }

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("failed to create state: {e}"))?;

        state
            .full(params, &samples)
            .map_err(|e| format!("transcription failed: {e}"))?;

        let num_segments = state.full_n_segments();
        let mut text = String::new();
        let mut segments = Vec::new();

        for i in 0..num_segments {
            let segment = state
                .get_segment(i)
                .ok_or_else(|| format!("segment {i} out of bounds"))?;
            let segment_text = segment
                .to_str()
                .map_err(|e| format!("failed to get segment text: {e}"))?
                .to_string();
            let start_ts = segment.start_timestamp();
            let end_ts = segment.end_timestamp();
            let confidence = 1.0 - segment.no_speech_probability();

            if !should_include_segment(&segment_text, confidence) {
                info!(
                    "hallucination filtered: {:?} (confidence: {:.2})",
                    segment_text.trim(),
                    confidence
                );
                continue;
            }

            let normalized = normalize_spacing(segment_text.trim());
            let sanitized = sanitize_text(&normalized);
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&sanitized);

            segments.push(TranscriptSegment {
                start_ms: (start_ts * 10) as u64,
                end_ms: (end_ts * 10) as u64,
                text: sanitized,
                confidence,
            });
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok((text, segments, duration_ms))
    }
}

async fn send_response(response: &SidecarResponse) -> Result<(), Box<dyn std::error::Error>> {
    let mut json = serde_json::to_string(response)?;
    json.push('\n');
    let mut stdout = tokio::io::stdout();
    stdout.write_all(json.as_bytes()).await?;
    stdout.flush().await?;
    Ok(())
}

async fn send_error(id: u64, message: String) {
    let response = SidecarResponse::Error { id, message };
    if let Err(e) = send_response(&response).await {
        error!("failed to send error response: {}", e);
    }
}

#[tokio::main]
async fn main() {
    // Set up tracing to stderr (stdout is for IPC)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Parse command line args for initial model path and VAD model path
    let args: Vec<String> = std::env::args().collect();
    let mut initial_model_path: Option<PathBuf> = None;
    let mut vad_model_path: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--model" && i + 1 < args.len() {
            initial_model_path = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if args[i] == "--vad-model" && i + 1 < args.len() {
            vad_model_path = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else {
            i += 1;
        }
    }

    if let Some(ref vad_path) = vad_model_path {
        info!("VAD model path: {}", vad_path.display());
    }

    #[cfg(feature = "whisper")]
    let mut engine: Option<TranscriptionEngine> = None;

    #[cfg(feature = "whisper")]
    if let Some(model_path) = &initial_model_path {
        info!("loading initial model: {}", model_path.display());
        match TranscriptionEngine::load(model_path) {
            Ok(e) => {
                info!("model loaded successfully");
                engine = Some(e);
            }
            Err(e) => {
                error!("failed to load initial model: {}", e);
            }
        }
    }

    #[cfg(not(feature = "whisper"))]
    if initial_model_path.is_some() {
        error!("whisper feature not enabled, cannot load model");
    }

    info!("sidecar ready, reading from stdin");

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                info!("stdin closed, shutting down");
                break;
            }
            Err(e) => {
                error!("error reading stdin: {}", e);
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: SidecarRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                error!("failed to parse request: {}: {}", e, line);
                continue;
            }
        };

        match request {
            SidecarRequest::Shutdown => {
                info!("shutdown requested");
                break;
            }

            SidecarRequest::LoadModel { id, model_path } => {
                info!("loading model: {}", model_path.display());

                #[cfg(feature = "whisper")]
                {
                    match TranscriptionEngine::load(&model_path) {
                        Ok(e) => {
                            engine = Some(e);
                            let response = SidecarResponse::ModelLoaded { id };
                            if let Err(e) = send_response(&response).await {
                                error!("failed to send response: {}", e);
                            }
                        }
                        Err(e) => {
                            send_error(id, format!("failed to load model: {e}")).await;
                        }
                    }
                }

                #[cfg(not(feature = "whisper"))]
                {
                    let _ = model_path;
                    send_error(id, "whisper feature not enabled".to_string()).await;
                }
            }

            SidecarRequest::Transcribe {
                id,
                audio_path,
                language,
                initial_prompt,
                single_segment,
            } => {
                info!("transcribing: {}", audio_path.display());

                #[cfg(feature = "whisper")]
                {
                    match &engine {
                        Some(eng) => match eng.transcribe(
                            &audio_path,
                            language.as_deref(),
                            initial_prompt.as_deref(),
                            single_segment,
                            vad_model_path.as_deref(),
                        ) {
                            Ok((text, segments, duration_ms)) => {
                                let response = SidecarResponse::Transcription {
                                    id,
                                    text,
                                    segments,
                                    duration_ms,
                                };
                                if let Err(e) = send_response(&response).await {
                                    error!("failed to send response: {}", e);
                                }
                            }
                            Err(e) => {
                                send_error(id, e).await;
                            }
                        },
                        None => {
                            send_error(id, "no model loaded".to_string()).await;
                        }
                    }
                }

                #[cfg(not(feature = "whisper"))]
                {
                    let _ = (audio_path, language, initial_prompt, single_segment);
                    send_error(id, "whisper feature not enabled".to_string()).await;
                }
            }
        }
    }

    info!("sidecar exiting");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_text_high_confidence_included() {
        assert!(should_include_segment("Hello, how are you?", 0.9));
    }

    #[test]
    fn thank_you_low_confidence_excluded() {
        assert!(!should_include_segment("Thank you.", 0.3));
    }

    #[test]
    fn thank_you_high_confidence_excluded() {
        // Always-reject: "thank you" is a YouTube artifact, rejected at any confidence
        assert!(!should_include_segment("Thank you.", 0.8));
        assert!(!should_include_segment("Thank you!", 0.95));
        assert!(!should_include_segment("Thank you?", 0.9));
        assert!(!should_include_segment("Thank you", 0.99));
    }

    #[test]
    fn empty_text_excluded() {
        assert!(!should_include_segment("", 0.9));
        assert!(!should_include_segment("   ", 0.9));
    }

    #[test]
    fn blank_audio_token_excluded() {
        assert!(!should_include_segment("[BLANK_AUDIO]", 0.9));
    }

    #[test]
    fn you_always_excluded() {
        // "you" is an always-reject pattern — rejected at any confidence
        assert!(!should_include_segment("you", 0.5));
        assert!(!should_include_segment("you", 0.9));
    }

    #[test]
    fn longer_sentence_with_you_included() {
        assert!(should_include_segment("You should try this", 0.7));
    }

    #[test]
    fn below_confidence_threshold_excluded() {
        assert!(!should_include_segment("Some random text", 0.35));
    }

    #[test]
    fn hallucination_at_boundary_confidence() {
        // "Thank you" is always-reject, so excluded at any confidence
        assert!(!should_include_segment("Thank you.", 0.6));
        assert!(!should_include_segment("Thank you.", 0.59));
        // Marginal patterns: at exactly 0.6, should be included (filter is < 0.6)
        assert!(should_include_segment("Yeah", 0.6));
        // Just below 0.6, marginal should be excluded
        assert!(!should_include_segment("Yeah", 0.59));
    }

    #[test]
    fn special_tokens_excluded() {
        assert!(!should_include_segment("[MUSIC]", 0.9));
        assert!(!should_include_segment("[NOISE]", 0.9));
    }

    #[test]
    fn hallucination_patterns_case_insensitive() {
        // Always-reject patterns work at any confidence
        assert!(!should_include_segment("THANK YOU.", 0.9));
        assert!(!should_include_segment("Bye.", 0.9));
        assert!(!should_include_segment("Subscribe.", 0.9));
    }

    #[test]
    fn single_word_repetition_detected() {
        assert!(has_excessive_repetition("the the the the"));
        assert!(has_excessive_repetition("hello hello hello"));
    }

    #[test]
    fn phrase_repetition_detected() {
        assert!(has_excessive_repetition("thank you thank you thank you"));
        assert!(has_excessive_repetition("I think I think I think"));
    }

    #[test]
    fn normal_text_no_repetition() {
        assert!(!has_excessive_repetition("Hello, how are you today?"));
        assert!(!has_excessive_repetition("The meeting went well"));
    }

    #[test]
    fn short_text_not_flagged() {
        assert!(!has_excessive_repetition("the the"));
        assert!(!has_excessive_repetition("ok"));
    }

    #[test]
    fn repetition_filtered_by_should_include() {
        assert!(!should_include_segment("the the the the", 0.9));
        assert!(!should_include_segment(
            "thank you thank you thank you",
            0.9
        ));
    }

    #[test]
    fn conversational_fillers_not_filtered_at_high_confidence() {
        // Conversational fillers at high confidence are real speech — they pass through.
        // At marginal confidence (< 0.6) single-word fillers are now filtered as
        // likely hallucinations, but at 0.7+ they're kept.
        assert!(should_include_segment("So", 0.7));
        assert!(should_include_segment("Okay.", 0.7));
        assert!(should_include_segment("Uh", 0.7));
        assert!(should_include_segment("Um", 0.7));
        assert!(should_include_segment("Hmm.", 0.7));
        assert!(should_include_segment("You know", 0.7));
        assert!(should_include_segment("I mean", 0.7));
        assert!(should_include_segment("Yeah", 0.7));
        assert!(should_include_segment("Right.", 0.7));
    }

    #[test]
    fn filler_patterns_filtered_at_marginal_confidence() {
        // Single-word fillers at marginal confidence (< 0.6) are likely hallucinations
        assert!(!should_include_segment("Yeah", 0.5));
        assert!(!should_include_segment("yeah.", 0.5));
        assert!(!should_include_segment("Okay.", 0.5));
        assert!(!should_include_segment("Um", 0.5));
        assert!(!should_include_segment("So", 0.5));
        assert!(!should_include_segment("Right.", 0.5));
    }

    #[test]
    fn punctuation_joined_repetition_detected() {
        // "Yeah.Yeah.Yeah." has no whitespace — normalization catches it
        assert!(!should_include_segment("Yeah.Yeah.Yeah.", 0.9));
        assert!(!should_include_segment("No.No.No.No.", 0.9));
        assert!(!should_include_segment("Okay,Okay,Okay,", 0.9));
    }

    #[test]
    fn normalize_spacing_basic() {
        assert_eq!(normalize_spacing("Hello.World"), "Hello. World");
        assert_eq!(
            normalize_spacing("First sentence.Then second"),
            "First sentence. Then second"
        );
    }

    #[test]
    fn normalize_spacing_multiple_punctuation() {
        assert_eq!(
            normalize_spacing("What?Really!Yes.OK"),
            "What? Really! Yes. OK"
        );
    }

    #[test]
    fn normalize_spacing_already_correct() {
        assert_eq!(normalize_spacing("Hello. World"), "Hello. World");
        assert_eq!(normalize_spacing("Normal text here"), "Normal text here");
    }

    #[test]
    fn normalize_spacing_preserves_abbreviations() {
        // Lowercase after period = abbreviation, don't add space
        assert_eq!(normalize_spacing("e.g. something"), "e.g. something");
        assert_eq!(normalize_spacing("i.e. this"), "i.e. this");
        assert_eq!(normalize_spacing("3.5 million"), "3.5 million");
    }

    #[test]
    fn normalize_spacing_empty() {
        assert_eq!(normalize_spacing(""), "");
    }

    #[test]
    fn normalize_spacing_comma_before_uppercase() {
        assert_eq!(normalize_spacing("first,Second"), "first, Second");
        assert_eq!(normalize_spacing("ok,let me"), "ok,let me"); // lowercase = no change
    }

    // sanitize_text tests

    #[test]
    fn sanitize_text_maps_unicode_punctuation() {
        assert_eq!(
            sanitize_text("He said \u{201C}hello\u{201D}"),
            "He said \"hello\""
        );
        assert_eq!(sanitize_text("it\u{2019}s fine"), "it's fine");
        assert_eq!(sanitize_text("wait\u{2014}what"), "wait--what");
        assert_eq!(sanitize_text("1\u{2013}2"), "1-2");
        assert_eq!(sanitize_text("hmm\u{2026}"), "hmm...");
    }

    #[test]
    fn sanitize_text_strips_non_ascii_artifacts() {
        assert_eq!(sanitize_text("Hello \u{266A} world"), "Hello world");
        assert_eq!(sanitize_text("text\u{200B}here"), "texthere"); // zero-width space
    }

    #[test]
    fn sanitize_text_collapses_double_spaces() {
        assert_eq!(sanitize_text("hello  world"), "hello world");
        assert_eq!(sanitize_text("a   b   c"), "a b c");
    }

    #[test]
    fn sanitize_text_empty() {
        assert_eq!(sanitize_text(""), "");
    }

    #[test]
    fn sanitize_text_ascii_passthrough() {
        assert_eq!(sanitize_text("Normal text here."), "Normal text here.");
    }

    // punctuation-only filtering tests

    #[test]
    fn punctuation_only_segments_excluded() {
        assert!(!should_include_segment("...", 0.9));
        assert!(!should_include_segment("---", 0.9));
        assert!(!should_include_segment(",", 0.9));
        assert!(!should_include_segment("\u{266A}", 0.9)); // non-ASCII symbol only
    }

    #[test]
    fn text_with_punctuation_still_included() {
        assert!(should_include_segment("Hello, world.", 0.9));
        assert!(should_include_segment("Wait, what?", 0.9));
    }
}
