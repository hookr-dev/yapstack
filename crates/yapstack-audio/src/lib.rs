pub mod capture;
pub mod device;
pub mod error;
pub mod export;
pub mod manager;
pub mod mic;
pub mod mixer;
pub mod ring_buffer;
pub(crate) mod stream;
pub mod system;

pub use capture::{BufferPositions, SeparateExtraction};
pub use error::AudioError;
pub use export::SessionWavWriter;
pub use manager::AudioManager;
pub use mixer::MixConfig;
pub use ring_buffer::{AudioRingBuffer, RingBufferInfo, SharedAudioRingBuffer};

/// The actual stream configuration used by a device after negotiation.
#[derive(Debug, Clone, Copy)]
pub struct DeviceStreamConfig {
    pub sample_rate: u32,
    pub channels: u16,
}
