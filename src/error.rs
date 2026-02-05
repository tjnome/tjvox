use thiserror::Error;

#[derive(Error, Debug)]
pub enum TjvoxError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Transcription error: {0}")]
    Transcription(String),

    #[error("Output injection error: {0}")]
    Output(String),

    #[error("UI error: {0}")]
    Ui(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Model download error: {0}")]
    ModelDownload(String),

    #[error("Model load error: {0}")]
    ModelLoad(String),

    #[error("LLM processing error: {0}")]
    Llm(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_config() {
        let err = TjvoxError::Config("bad value".to_string());
        assert_eq!(err.to_string(), "Configuration error: bad value");
    }

    #[test]
    fn test_error_display_transcription() {
        let err = TjvoxError::Transcription("failed".to_string());
        assert_eq!(err.to_string(), "Transcription error: failed");
    }

    #[test]
    fn test_error_display_output() {
        let err = TjvoxError::Output("paste failed".to_string());
        assert_eq!(err.to_string(), "Output injection error: paste failed");
    }

    #[test]
    fn test_error_display_ui() {
        let err = TjvoxError::Ui("notification error".to_string());
        assert_eq!(err.to_string(), "UI error: notification error");
    }

    #[test]
    fn test_error_display_model_download() {
        let err = TjvoxError::ModelDownload("HTTP 404".to_string());
        assert_eq!(err.to_string(), "Model download error: HTTP 404");
    }

    #[test]
    fn test_error_display_model_load() {
        let err = TjvoxError::ModelLoad("corrupt file".to_string());
        assert_eq!(err.to_string(), "Model load error: corrupt file");
    }

    #[test]
    fn test_error_display_llm() {
        let err = TjvoxError::Llm("timeout".to_string());
        assert_eq!(err.to_string(), "LLM processing error: timeout");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: TjvoxError = io_err.into();
        assert!(err.to_string().contains("file not found"));
    }
}
