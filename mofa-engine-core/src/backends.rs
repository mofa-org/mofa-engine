//! Provider backend implementations.

pub mod local_tts;
pub mod ollama;
pub mod openai_compat;

pub use local_tts::LocalTtsProvider;
pub use ollama::OllamaProvider;
pub use openai_compat::OpenAiCompatProvider;
