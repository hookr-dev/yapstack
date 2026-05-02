use serde::Serialize;
use specta::Type;

/// Unified error type for all Tauri commands.
///
/// Serializes to `{ "kind": "...", "message": "..." }` via `#[serde(tag = "kind")]`.
/// Auto-generated TypeScript types via specta.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind")]
pub enum CommandError {
    Audio { message: String },
    Transcription { message: String },
    Embedding { message: String },
    NotInitialized { message: String },
    InvalidInput { message: String },
    NotFound { message: String },
    Internal { message: String },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Audio { message } => write!(f, "audio: {message}"),
            Self::Transcription { message } => write!(f, "transcription: {message}"),
            Self::Embedding { message } => write!(f, "embedding: {message}"),
            Self::NotInitialized { message } => write!(f, "not initialized: {message}"),
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::NotFound { message } => write!(f, "not found: {message}"),
            Self::Internal { message } => write!(f, "internal: {message}"),
        }
    }
}

impl From<yapstack_audio::AudioError> for CommandError {
    fn from(e: yapstack_audio::AudioError) -> Self {
        Self::Audio {
            message: e.to_string(),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<yapstack_transcription::TranscriptionError> for CommandError {
    fn from(e: yapstack_transcription::TranscriptionError) -> Self {
        Self::Transcription {
            message: e.to_string(),
        }
    }
}

impl From<yapstack_embedding::EmbeddingError> for CommandError {
    fn from(e: yapstack_embedding::EmbeddingError) -> Self {
        Self::Embedding {
            message: e.to_string(),
        }
    }
}

impl From<rusqlite::Error> for CommandError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal {
            message: format!("sqlite: {e}"),
        }
    }
}

impl From<std::io::Error> for CommandError {
    fn from(e: std::io::Error) -> Self {
        Self::Internal {
            message: e.to_string(),
        }
    }
}

/// Validate that a session ID is safe for use in file path construction.
/// Rejects path traversal characters and empty IDs.
pub fn validate_session_id(id: &str) -> Result<(), CommandError> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(CommandError::InvalidInput {
            message: "invalid session ID".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_error_serialize() {
        let err = CommandError::Audio {
            message: "device failed".to_string(),
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "Audio");
        assert_eq!(json["message"], "device failed");
    }

    #[test]
    fn test_command_error_from_audio_error() {
        let audio_err = yapstack_audio::AudioError::NoBufferAvailable;
        let cmd_err: CommandError = audio_err.into();
        assert!(matches!(cmd_err, CommandError::Audio { .. }));
    }
}
