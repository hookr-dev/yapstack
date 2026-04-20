//! Engine abstraction layer for the transcription sidecar.
//!
//! Each backend (Whisper, Parakeet) implements [`TranscriptionBackend`].
//! The sidecar is spawned with `--engine whisper|parakeet` and constructs
//! the matching backend; the IPC dispatch loop in `main.rs` is engine-agnostic.

#[cfg(any(feature = "whisper", feature = "parakeet"))]
use std::path::Path;

#[cfg(any(feature = "whisper", feature = "parakeet"))]
use yapstack_common::types::TranscriptSegment;

#[cfg(feature = "whisper")]
pub mod whisper;

#[cfg(feature = "parakeet")]
pub mod parakeet;

/// Per-request transcription options. Engines ignore options they don't
/// support (e.g. Parakeet ignores `initial_prompt`; Whisper ignores
/// `diarization`).
#[cfg(any(feature = "whisper", feature = "parakeet"))]
#[derive(Debug, Clone, Copy)]
pub struct TranscribeOpts<'a> {
    pub language: Option<&'a str>,
    pub initial_prompt: Option<&'a str>,
    pub single_segment: Option<bool>,
    /// Honored by Parakeet via Sortformer; ignored by Whisper.
    #[allow(dead_code)]
    pub diarization: bool,
}

#[cfg(any(feature = "whisper", feature = "parakeet"))]
#[derive(Debug, Clone)]
pub struct TranscriptionOutput {
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
    pub duration_ms: u64,
}

/// Read a WAV file, downmix to mono, and resample to 16 kHz. Both Whisper
/// and Parakeet require this exact input shape.
#[cfg(any(feature = "whisper", feature = "parakeet"))]
pub(crate) fn read_wav_as_mono_16k(audio_path: &Path) -> Result<Vec<f32>, String> {
    let mut reader =
        hound::WavReader::open(audio_path).map_err(|e| format!("failed to open WAV: {e}"))?;
    let spec = reader.spec();
    let raw: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
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
    let mono = yapstack_common::audio::deinterleave_to_mono(&raw, spec.channels);
    let resampled = yapstack_common::audio::resample(&mono, spec.sample_rate, 16_000)
        .map_err(|e| format!("resample to 16kHz failed: {e}"))?;
    Ok(resampled.into_owned())
}

/// What every transcription backend must implement. Engine-specific
/// configuration (Whisper VAD model, Parakeet decoder cache, etc.) is
/// passed to the backend's constructor — not through this trait.
#[cfg(any(feature = "whisper", feature = "parakeet"))]
pub trait TranscriptionBackend: Send {
    /// (Re)load the primary transcription model from disk. May be called
    /// more than once per process if the user switches models.
    fn load_model(&mut self, model_path: &Path) -> Result<(), String>;

    /// Transcribe the WAV file at `audio_path`. The backend is responsible
    /// for any required resampling/channel conversion. Takes `&mut self`
    /// because some backends (Parakeet) mutate decoder state per call.
    fn transcribe(
        &mut self,
        audio_path: &Path,
        opts: TranscribeOpts<'_>,
    ) -> Result<TranscriptionOutput, String>;
}

// ---------- Shared text post-processing ----------
//
// These helpers were originally Whisper-specific. They are mild enough
// that running Parakeet output through them is harmless (Parakeet rarely
// produces YouTube-artifact hallucinations or unicode quote marks, but
// the filters are no-ops on clean text).

/// Insert a space after sentence-ending punctuation (`.` `!` `?`) or commas
/// when immediately followed by an uppercase letter. Fixes joined sentences
/// like "First sentence.Then second" while preserving "e.g." abbreviations
/// (which are followed by lowercase).
#[cfg(any(feature = "whisper", feature = "parakeet", test))]
pub(crate) fn normalize_spacing(text: &str) -> String {
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
/// non-ASCII characters, and collapse runs of multiple spaces.
#[cfg(any(feature = "whisper", feature = "parakeet", test))]
pub(crate) fn sanitize_text(text: &str) -> String {
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

/// Normalize text for repetition detection: insert spaces around punctuation
/// so "Yeah.Yeah.Yeah." becomes "Yeah . Yeah . Yeah ." and whitespace-based
/// splitting catches the loop.
#[cfg(any(feature = "whisper", feature = "parakeet", test))]
pub(crate) fn normalize_for_repetition(text: &str) -> String {
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

/// Detect excessive repetition (`"the the the the"` or `"thank you thank you …"`).
#[cfg(any(feature = "whisper", feature = "parakeet", test))]
pub(crate) fn has_excessive_repetition(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 3 {
        return false;
    }

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

/// Whether a transcript segment should be kept. Filters out empty text,
/// special tokens (`[BLANK_AUDIO]`), low-confidence segments, known YouTube
/// hallucination patterns, and excessive repetition.
#[cfg(any(feature = "whisper", feature = "parakeet", test))]
pub(crate) fn should_include_segment(text: &str, confidence: f32) -> bool {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return false;
    }

    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return false;
    }

    if has_excessive_repetition(trimmed) {
        tracing::info!("repetition filtered: {:?}", trimmed);
        return false;
    }
    let normalized = normalize_for_repetition(trimmed);
    if has_excessive_repetition(&normalized) {
        tracing::info!("repetition filtered (normalized): {:?}", trimmed);
        return false;
    }

    if confidence < 0.4 {
        return false;
    }

    if yapstack_common::hallucination::is_always_reject(trimmed) {
        return false;
    }

    if confidence < 0.6 && yapstack_common::hallucination::is_marginal_reject(trimmed) {
        return false;
    }

    true
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
        assert!(!should_include_segment("Thank you.", 0.6));
        assert!(!should_include_segment("Thank you.", 0.59));
        assert!(should_include_segment("Yeah", 0.6));
        assert!(!should_include_segment("Yeah", 0.59));
    }

    #[test]
    fn special_tokens_excluded() {
        assert!(!should_include_segment("[MUSIC]", 0.9));
        assert!(!should_include_segment("[NOISE]", 0.9));
    }

    #[test]
    fn hallucination_patterns_case_insensitive() {
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
        assert!(!should_include_segment("Yeah", 0.5));
        assert!(!should_include_segment("yeah.", 0.5));
        assert!(!should_include_segment("Okay.", 0.5));
        assert!(!should_include_segment("Um", 0.5));
        assert!(!should_include_segment("So", 0.5));
        assert!(!should_include_segment("Right.", 0.5));
    }

    #[test]
    fn punctuation_joined_repetition_detected() {
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
        assert_eq!(normalize_spacing("ok,let me"), "ok,let me");
    }

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
        assert_eq!(sanitize_text("text\u{200B}here"), "texthere");
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

    #[test]
    fn punctuation_only_segments_excluded() {
        assert!(!should_include_segment("...", 0.9));
        assert!(!should_include_segment("---", 0.9));
        assert!(!should_include_segment(",", 0.9));
        assert!(!should_include_segment("\u{266A}", 0.9));
    }

    #[test]
    fn text_with_punctuation_still_included() {
        assert!(should_include_segment("Hello, world.", 0.9));
        assert!(should_include_segment("Wait, what?", 0.9));
    }
}
