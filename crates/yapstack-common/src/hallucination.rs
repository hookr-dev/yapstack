//! Shared hallucination detection for Whisper transcription output.
//!
//! Two-tier pattern system:
//! - **Always-reject**: YouTube/podcast artifacts that are never real dictation content.
//!   Rejected regardless of confidence.
//! - **Marginal-reject**: Single-word fillers that are real speech sometimes.
//!   Only rejected at low confidence (< 0.6).
//!
//! All matching is case-insensitive with trailing punctuation stripped.

/// Strip trailing punctuation so "Thank you!" matches "thank you".
pub fn strip_trailing_punctuation(text: &str) -> &str {
    text.trim_end_matches(['.', '!', '?', ',', ';', ':'])
}

/// YouTube/podcast artifacts — never real dictation content.
/// All lowercase, no trailing punctuation.
const ALWAYS_REJECT_PATTERNS: &[&str] = &[
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

/// Single-word fillers — real speech at high confidence, hallucinations at low confidence.
/// All lowercase, no trailing punctuation.
const MARGINAL_REJECT_PATTERNS: &[&str] = &[
    "yeah", "yes", "no", "okay", "oh", "hmm", "uh", "um", "so", "right",
];

/// Returns `true` if the text matches an always-reject hallucination pattern.
/// Strips trailing punctuation and compares case-insensitively.
pub fn is_always_reject(text: &str) -> bool {
    let trimmed = strip_trailing_punctuation(text.trim());
    let lower = trimmed.to_lowercase();
    ALWAYS_REJECT_PATTERNS.contains(&lower.as_str())
}

/// Returns `true` if the text matches a marginal-reject hallucination pattern.
/// Strips trailing punctuation and compares case-insensitively.
pub fn is_marginal_reject(text: &str) -> bool {
    let trimmed = strip_trailing_punctuation(text.trim());
    let lower = trimmed.to_lowercase();
    MARGINAL_REJECT_PATTERNS.contains(&lower.as_str())
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

    // --- is_always_reject ---

    #[test]
    fn always_reject_exact() {
        assert!(is_always_reject("thank you"));
        assert!(is_always_reject("Thanks for watching"));
        assert!(is_always_reject("Bye"));
        assert!(is_always_reject("Subscribe"));
        assert!(is_always_reject("The end"));
    }

    #[test]
    fn always_reject_with_punctuation() {
        assert!(is_always_reject("Thank you."));
        assert!(is_always_reject("Thank you!"));
        assert!(is_always_reject("Thank you?"));
        assert!(is_always_reject("Thanks for watching."));
        assert!(is_always_reject("Bye!"));
    }

    #[test]
    fn always_reject_case_insensitive() {
        assert!(is_always_reject("THANK YOU"));
        assert!(is_always_reject("THANKS FOR WATCHING"));
        assert!(is_always_reject("BYE"));
    }

    #[test]
    fn always_reject_with_whitespace() {
        assert!(is_always_reject("  Thank you.  "));
        assert!(is_always_reject(" Bye "));
    }

    #[test]
    fn always_reject_longer_sentence_passes() {
        assert!(!is_always_reject(
            "Thank you for your help with the project"
        ));
        assert!(!is_always_reject("You should try this"));
    }

    #[test]
    fn always_reject_normal_text_passes() {
        assert!(!is_always_reject("Hello, how are you?"));
        assert!(!is_always_reject("Let me think about that"));
    }

    // --- is_marginal_reject ---

    #[test]
    fn marginal_reject_exact() {
        assert!(is_marginal_reject("yeah"));
        assert!(is_marginal_reject("Yes"));
        assert!(is_marginal_reject("okay"));
        assert!(is_marginal_reject("Um"));
    }

    #[test]
    fn marginal_reject_with_punctuation() {
        assert!(is_marginal_reject("yeah."));
        assert!(is_marginal_reject("Okay."));
        assert!(is_marginal_reject("Right!"));
    }

    #[test]
    fn marginal_reject_not_always_reject() {
        assert!(!is_marginal_reject("thank you"));
        assert!(!is_marginal_reject("Bye"));
    }

    #[test]
    fn marginal_reject_normal_text_passes() {
        assert!(!is_marginal_reject("Yeah that sounds good"));
    }
}
