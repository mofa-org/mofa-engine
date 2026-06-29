//! Provider backend implementations.

pub mod ollama;
pub mod openai_compat;

pub use ollama::OllamaProvider;
pub use openai_compat::OpenAiCompatProvider;
