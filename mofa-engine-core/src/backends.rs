//! Provider backend implementations.

pub(crate) mod cloud_video_gen;
pub(crate) mod liter_llm;
pub(crate) mod local_asr;
pub(crate) mod local_image_gen;
pub(crate) mod local_tts;
pub(crate) mod local_video_gen;
pub(crate) mod ollama;
pub(crate) mod openai_compat;
pub(crate) mod system_tts;

pub(crate) use cloud_video_gen::CloudVideoGenProvider;
pub(crate) use liter_llm::LiterLLMProvider;
pub(crate) use local_asr::LocalAsrProvider;
pub(crate) use local_image_gen::LocalImageGenProvider;
pub(crate) use local_tts::LocalTtsProvider;
pub(crate) use local_video_gen::LocalVideoGenProvider;
pub(crate) use ollama::OllamaProvider;
pub(crate) use openai_compat::OpenAiCompatProvider;
pub(crate) use system_tts::SystemTtsProvider;
