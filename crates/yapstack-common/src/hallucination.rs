//! Shared hallucination detection for transcription output.
//!
//! Two-tier pattern system, **engine-gated** because Whisper and Parakeet have
//! different hallucination characteristics:
//!
//! - **Whisper** is trained on a large amount of YouTube-adjacent audio and
//!   reliably hallucinates canned phrases (`"thank you"`, `"bye"`, `"subscribe"`,
//!   `"the end"`, etc.) on silence or near-silence. The aggressive always-reject
//!   list keeps real Whisper sessions clean and is intentionally preserved.
//! - **Parakeet** does not exhibit the same hallucination pattern, so the
//!   aggressive Whisper list incorrectly drops legitimate one-word answers and
//!   polite phrases ("thank you" at the end of a turn, "bye" on hang-up). For
//!   Parakeet those single/short entries are demoted to the marginal tier
//!   (rejected only at confidence < 0.6), and the longer YouTube-specific
//!   phrases stay in always-reject for both engines (they're unambiguous
//!   artifact regardless of model).
//!
//! All matching is case-insensitive with trailing punctuation stripped.

use crate::types::EngineKind;

/// Strip trailing punctuation so "Thank you!" matches "thank you".
pub fn strip_trailing_punctuation(text: &str) -> &str {
    text.trim_end_matches(['.', '!', '?', ',', ';', ':'])
}

/// Whisper's full always-reject list. YouTube/podcast artifacts that Whisper
/// hallucinates on silence — never real dictation content for Whisper.
const WHISPER_ALWAYS_REJECT: &[&str] = &[
    "thank you",
    "thanks for watching",
    "thanks for listening",
    "thank you for watching",
    "thank you for watching please subscribe",
    "thanks for watching please subscribe",
    "bye",
    "you",
    "the end",
    "subscribe",
    "like and subscribe",
    "please subscribe",
    "see you next time",
    "subtitles by the amara org community",
    "subtitles by the amara.org community",
    "...",
];

/// Parakeet's narrower always-reject list. Only the unambiguous YouTube-style
/// canned outros stay; single-word and short polite entries move to the
/// marginal tier so legitimate end-of-turn "thank you" / "bye" survive.
const PARAKEET_ALWAYS_REJECT: &[&str] = &[
    "thanks for watching",
    "thanks for listening",
    "thank you for watching",
    "thank you for watching please subscribe",
    "thanks for watching please subscribe",
    "like and subscribe",
    "please subscribe",
    "see you next time",
    "subtitles by the amara org community",
    "subtitles by the amara.org community",
    "...",
];

/// Whisper's marginal-reject list — single-word fillers that are real speech
/// at high confidence, hallucinations at low confidence.
const WHISPER_MARGINAL_REJECT: &[&str] = &[
    "yeah", "yes", "no", "okay", "oh", "hmm", "uh", "um", "so", "right",
];

/// Parakeet's marginal-reject list — Whisper's marginals plus the entries
/// demoted from Whisper's always-reject. Real speech at high confidence,
/// likely artifact at low confidence.
const PARAKEET_MARGINAL_REJECT: &[&str] = &[
    "yeah",
    "yes",
    "no",
    "okay",
    "oh",
    "hmm",
    "uh",
    "um",
    "so",
    "right",
    // Demoted from Whisper's always-reject because Parakeet doesn't
    // hallucinate them and they're real speech in real conversations.
    "you",
    "bye",
    "the end",
    "subscribe",
    "thank you",
];

fn always_reject_for(engine: EngineKind) -> &'static [&'static str] {
    match engine {
        EngineKind::Whisper => WHISPER_ALWAYS_REJECT,
        EngineKind::Parakeet => PARAKEET_ALWAYS_REJECT,
    }
}

fn marginal_reject_for(engine: EngineKind) -> &'static [&'static str] {
    match engine {
        EngineKind::Whisper => WHISPER_MARGINAL_REJECT,
        EngineKind::Parakeet => PARAKEET_MARGINAL_REJECT,
    }
}

/// Returns `true` if the text matches an always-reject hallucination pattern
/// for the given engine. Strips trailing punctuation and compares case-insensitively.
pub fn is_always_reject(text: &str, engine: EngineKind) -> bool {
    let trimmed = strip_trailing_punctuation(text.trim());
    let lower = trimmed.to_lowercase();
    always_reject_for(engine).contains(&lower.as_str())
}

/// Returns `true` if the text matches a marginal-reject hallucination pattern
/// for the given engine. Strips trailing punctuation and compares case-insensitively.
pub fn is_marginal_reject(text: &str, engine: EngineKind) -> bool {
    let trimmed = strip_trailing_punctuation(text.trim());
    let lower = trimmed.to_lowercase();
    marginal_reject_for(engine).contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_trailing_punctuation ---

    #[test]
    fn strip_period() {
        assert_eq!(strip_trailing_punctuation("hello."), "hello");
    }

    #[test]
    fn strip_exclamation() {
        assert_eq!(strip_trailing_punctuation("hello!"), "hello");
    }

    #[test]
    fn strip_question() {
        assert_eq!(strip_trailing_punctuation("hello?"), "hello");
    }

    #[test]
    fn strip_multiple() {
        assert_eq!(strip_trailing_punctuation("hello!?"), "hello");
    }

    #[test]
    fn strip_none() {
        assert_eq!(strip_trailing_punctuation("hello"), "hello");
    }

    #[test]
    fn strip_preserves_internal_punctuation() {
        assert_eq!(
            strip_trailing_punctuation("e.g. something"),
            "e.g. something"
        );
    }

    // --- Whisper: aggressive always-reject (preserves dictation-era behavior) ---

    #[test]
    fn whisper_always_reject_short_artifacts() {
        assert!(is_always_reject("thank you", EngineKind::Whisper));
        assert!(is_always_reject("Thanks for watching", EngineKind::Whisper));
        assert!(is_always_reject("Bye", EngineKind::Whisper));
        assert!(is_always_reject("Subscribe", EngineKind::Whisper));
        assert!(is_always_reject("The end", EngineKind::Whisper));
        assert!(is_always_reject("you", EngineKind::Whisper));
    }

    #[test]
    fn whisper_always_reject_with_punctuation() {
        assert!(is_always_reject("Thank you.", EngineKind::Whisper));
        assert!(is_always_reject("Bye!", EngineKind::Whisper));
    }

    #[test]
    fn whisper_always_reject_case_insensitive() {
        assert!(is_always_reject("THANK YOU", EngineKind::Whisper));
        assert!(is_always_reject("BYE", EngineKind::Whisper));
    }

    #[test]
    fn whisper_always_reject_normal_text_passes() {
        assert!(!is_always_reject(
            "Thank you for your help with the project",
            EngineKind::Whisper
        ));
        assert!(!is_always_reject(
            "Hello, how are you?",
            EngineKind::Whisper
        ));
    }

    // --- Parakeet: short polite entries are NOT always-rejected ---

    #[test]
    fn parakeet_does_not_always_reject_short_polite_entries() {
        // These are the entries that get incorrectly dropped on Parakeet today.
        // Demoted to marginal so high-confidence Parakeet output survives.
        assert!(!is_always_reject("thank you", EngineKind::Parakeet));
        assert!(!is_always_reject("Thank you.", EngineKind::Parakeet));
        assert!(!is_always_reject("Bye", EngineKind::Parakeet));
        assert!(!is_always_reject("Subscribe", EngineKind::Parakeet));
        assert!(!is_always_reject("The end", EngineKind::Parakeet));
        assert!(!is_always_reject("you", EngineKind::Parakeet));
    }

    #[test]
    fn parakeet_still_always_rejects_youtube_outros() {
        // Long unambiguous YouTube canned phrases are still always-rejected
        // even on Parakeet — they're never real speech regardless of engine.
        assert!(is_always_reject(
            "Thanks for watching",
            EngineKind::Parakeet
        ));
        assert!(is_always_reject(
            "thank you for watching please subscribe",
            EngineKind::Parakeet
        ));
        assert!(is_always_reject(
            "subtitles by the amara.org community",
            EngineKind::Parakeet
        ));
    }

    // --- Marginal: low-confidence drops apply to both engines ---

    #[test]
    fn whisper_marginal_filler_words() {
        assert!(is_marginal_reject("yeah", EngineKind::Whisper));
        assert!(is_marginal_reject("Okay.", EngineKind::Whisper));
        assert!(is_marginal_reject("Right!", EngineKind::Whisper));
    }

    #[test]
    fn parakeet_marginal_includes_demoted_whisper_always() {
        // Demoted entries land in Parakeet's marginal tier so low-confidence
        // versions still get dropped (preserving some hallucination defense).
        assert!(is_marginal_reject("thank you", EngineKind::Parakeet));
        assert!(is_marginal_reject("Bye", EngineKind::Parakeet));
        assert!(is_marginal_reject("you", EngineKind::Parakeet));
    }

    #[test]
    fn marginal_normal_text_passes_either_engine() {
        assert!(!is_marginal_reject(
            "Yeah that sounds good",
            EngineKind::Whisper
        ));
        assert!(!is_marginal_reject(
            "Yeah that sounds good",
            EngineKind::Parakeet
        ));
    }
}
