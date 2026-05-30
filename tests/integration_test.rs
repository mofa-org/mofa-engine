//! Integration tests for MoFA Engine.

use mofa_kernel::*;

#[test]
fn test_model_type_parse() {
    assert_eq!("llm".parse::<ModelType>().unwrap(), ModelType::Llm);
    assert_eq!("tts".parse::<ModelType>().unwrap(), ModelType::Tts);
    assert_eq!("asr".parse::<ModelType>().unwrap(), ModelType::Asr);
    assert_eq!("image_gen".parse::<ModelType>().unwrap(), ModelType::ImageGen);
    assert_eq!("video-gen".parse::<ModelType>().unwrap(), ModelType::VideoGen);
    assert!("invalid".parse::<ModelType>().is_err());
}

#[test]
fn test_model_type_display() {
    assert_eq!(ModelType::Llm.to_string(), "llm");
    assert_eq!(ModelType::Tts.to_string(), "tts");
}

#[test]
fn test_model_input_serialization() {
    let input = ModelInput {
        text: Some("hello".to_string()),
        file: None,
        base64: None,
        prompt: Some("system prompt".to_string()),
        params: None,
    };
    let json = serde_json::to_string(&input).unwrap();
    assert!(json.contains("hello"));
    assert!(!json.contains("file"));
}

#[test]
fn test_run_request_deserialization() {
    let json = r#"{"type":"llm","input":{"text":"hello"},"hint":{"next":"tts"}}"#;
    let req: RunRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.model_type.as_deref(), Some("llm"));
    assert_eq!(req.input.text.as_deref(), Some("hello"));
    assert_eq!(req.hint.as_ref().unwrap().next.as_deref(), Some("tts"));
}
