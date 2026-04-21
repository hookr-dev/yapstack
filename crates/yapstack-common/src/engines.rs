//! Engine capability catalogue.
//!
//! Source of truth for which transcription engines exist, what languages
//! each supports, and which optional features they expose. Used by:
//!
//! - the sidecar to validate `Transcribe` requests against the engine it
//!   was spawned with,
//! - the Tauri command layer to compose engine descriptors with downloaded
//!   model metadata when serving the frontend,
//! - the frontend (indirectly via that command) to drive cascading
//!   engine → model → language dropdowns.
//!
//! Model lists live in `yapstack-transcription` (their download URLs and
//! sizes don't belong here). The frontend joins the two.

use crate::types::EngineKind;

/// Capabilities of a single transcription engine.
pub struct EngineDescriptor {
    pub kind: EngineKind,
    pub display_name: &'static str,
    /// BCP-47 / ISO-639-1 codes the engine can transcribe. The first entry
    /// is the engine's primary language.
    pub languages: &'static [&'static str],
    /// True when the engine supports an optional speaker-diarization pass
    /// (today: Parakeet via Sortformer).
    pub supports_diarization: bool,
    /// True when the engine accepts an `initial_prompt` for cross-chunk
    /// continuity (today: Whisper only).
    pub supports_initial_prompt: bool,
}

impl EngineDescriptor {
    pub fn supports_language(&self, code: &str) -> bool {
        self.languages.iter().any(|l| l.eq_ignore_ascii_case(code))
    }
}

/// All engines Yapstack can dispatch to. Order is the UI presentation order.
pub fn engine_catalogue() -> &'static [EngineDescriptor] {
    &CATALOGUE
}

pub fn descriptor(kind: EngineKind) -> &'static EngineDescriptor {
    engine_catalogue()
        .iter()
        .find(|d| d.kind == kind)
        .expect("every EngineKind variant has a catalogue entry")
}

static CATALOGUE: [EngineDescriptor; 2] = [
    EngineDescriptor {
        kind: EngineKind::Whisper,
        display_name: "Whisper",
        languages: WHISPER_LANGUAGES,
        supports_diarization: false,
        supports_initial_prompt: true,
    },
    EngineDescriptor {
        kind: EngineKind::Parakeet,
        display_name: "Parakeet",
        languages: PARAKEET_TDT_V3_LANGUAGES,
        supports_diarization: true,
        supports_initial_prompt: false,
    },
];

/// Whisper supports 99 languages. Source: openai-whisper tokenizer
/// (`whisper/tokenizer.py`). Codes are ISO-639-1 (a few are 639-3 like
/// `yue`, `jw`); kept in the same order Whisper uses internally.
static WHISPER_LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su",
];

/// Parakeet TDT-0.6b-v3 supports 25 European languages.
/// Source: nvidia/parakeet-tdt-0.6b-v3 model card on HuggingFace.
/// Refine when the Parakeet backend lands by reading the tokenizer's
/// declared language set.
static PARAKEET_TDT_V3_LANGUAGES: &[&str] = &[
    "en", "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "ga", "it", "lv", "lt",
    "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "uk",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_has_one_entry_per_engine_kind() {
        assert!(engine_catalogue()
            .iter()
            .any(|d| d.kind == EngineKind::Whisper));
        assert!(engine_catalogue()
            .iter()
            .any(|d| d.kind == EngineKind::Parakeet));
    }

    #[test]
    fn whisper_catalogue_matches_99_language_count() {
        let d = descriptor(EngineKind::Whisper);
        assert_eq!(d.languages.len(), 99, "Whisper publishes 99 languages");
        assert!(d.supports_initial_prompt);
        assert!(!d.supports_diarization);
        assert!(d.supports_language("en"));
        assert!(d.supports_language("ja"));
    }

    #[test]
    fn parakeet_catalogue_matches_25_language_count() {
        let d = descriptor(EngineKind::Parakeet);
        assert_eq!(
            d.languages.len(),
            25,
            "Parakeet TDT v3 publishes 25 languages"
        );
        assert!(d.supports_diarization);
        assert!(!d.supports_initial_prompt);
        assert!(d.supports_language("en"));
        assert!(d.supports_language("de"));
        // Whisper covers Japanese; Parakeet TDT v3 does not.
        assert!(!d.supports_language("ja"));
    }

    #[test]
    fn descriptor_lookup_is_total() {
        for kind in [EngineKind::Whisper, EngineKind::Parakeet] {
            assert_eq!(descriptor(kind).kind, kind);
        }
    }
}
