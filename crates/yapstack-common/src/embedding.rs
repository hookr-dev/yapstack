use serde::{Deserialize, Serialize};

/// IPC requests sent to `yapstack-embedding-sidecar` over stdin (one JSON
/// object per line).
///
/// Kept in its own module — and distinct from `SidecarRequest` /
/// `SidecarResponse` — so the embedding protocol can be lifted into a
/// dedicated `yapstack-embedding-common` crate later without touching
/// transcription types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EmbeddingRequest {
    /// Embed a single text. Used by both write paths (segments / dictations
    /// / notes) and the read path (`embed_query`). The sidecar truncates
    /// inputs longer than the model context (512 tokens for BGE-small).
    #[serde(rename = "embed")]
    Embed { id: u64, text: String },

    /// Embed a batch of texts in a single forward pass. Used by the
    /// backfill worker for throughput.
    #[serde(rename = "embed_batch")]
    EmbedBatch { id: u64, texts: Vec<String> },

    /// Report the model name + version the sidecar is running with. Used
    /// by the host to verify which model produced the embeddings it's
    /// about to write.
    #[serde(rename = "model_info")]
    ModelInfo { id: u64 },

    /// Graceful shutdown.
    #[serde(rename = "shutdown")]
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EmbeddingResponse {
    /// Single-text embedding result.
    #[serde(rename = "embedded")]
    Embedded { id: u64, vector: Vec<f32> },

    /// Batched embedding result. `vectors[i]` corresponds to
    /// `EmbeddingRequest::EmbedBatch.texts[i]`.
    #[serde(rename = "embedded_batch")]
    EmbeddedBatch { id: u64, vectors: Vec<Vec<f32>> },

    /// Reply to `ModelInfo`.
    #[serde(rename = "model_info")]
    ModelInfo {
        id: u64,
        /// Stable identifier of the model — e.g. `"bge-small-en-v1.5"`.
        name: String,
        /// Version pinning. Currently the fastembed-rs model variant
        /// release; recorded per embedding row so future re-embed
        /// migrations can filter on it deterministically.
        version: String,
        /// Output dimensionality. 384 for BGE-small.
        dimensions: u32,
    },

    /// Sidecar startup banner. Emitted once after stdin is wired up so the
    /// host can confirm liveness before issuing the first request.
    #[serde(rename = "ready")]
    Ready {
        name: String,
        version: String,
        dimensions: u32,
    },

    #[serde(rename = "error")]
    Error { id: u64, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_request_roundtrip() {
        let req = EmbeddingRequest::Embed {
            id: 7,
            text: "hello world".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"embed\""));
        let back: EmbeddingRequest = serde_json::from_str(&json).unwrap();
        match back {
            EmbeddingRequest::Embed { id, text } => {
                assert_eq!(id, 7);
                assert_eq!(text, "hello world");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn embed_batch_roundtrip() {
        let req = EmbeddingRequest::EmbedBatch {
            id: 9,
            texts: vec!["a".to_string(), "b".to_string()],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"embed_batch\""));
        let back: EmbeddingRequest = serde_json::from_str(&json).unwrap();
        match back {
            EmbeddingRequest::EmbedBatch { id, texts } => {
                assert_eq!(id, 9);
                assert_eq!(texts.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn embedded_response_roundtrip() {
        let resp = EmbeddingResponse::Embedded {
            id: 1,
            vector: vec![0.1, 0.2, 0.3],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: EmbeddingResponse = serde_json::from_str(&json).unwrap();
        match back {
            EmbeddingResponse::Embedded { id, vector } => {
                assert_eq!(id, 1);
                assert_eq!(vector.len(), 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn ready_response_roundtrip() {
        let resp = EmbeddingResponse::Ready {
            name: "bge-small-en-v1.5".to_string(),
            version: "1.5.0".to_string(),
            dimensions: 384,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"ready\""));
        let back: EmbeddingResponse = serde_json::from_str(&json).unwrap();
        match back {
            EmbeddingResponse::Ready { dimensions, .. } => assert_eq!(dimensions, 384),
            _ => panic!("wrong variant"),
        }
    }
}
