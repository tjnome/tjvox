use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;
use tracing::debug;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub audio: AudioConfig,
    pub transcription: TranscriptionConfig,
    pub whisper: WhisperConfig,
    pub output: OutputConfig,
    pub ui: UiConfig,
    #[serde(default)]
    pub overlay: OverlayConfig,
    #[serde(default)]
    pub replacements: ReplacementsConfig,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u8,
    pub format: String,
    pub temp_dir: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TranscriptionConfig {
    pub model: String,
    #[serde(default = "default_models_dir")]
    pub models_dir: String,
    pub language: Option<String>,
    pub threads: Option<u32>,
    #[serde(default)]
    pub remove_filler_words: bool,
}

fn default_models_dir() -> String {
    dirs::data_dir()
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".local/share"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        })
        .join("tjvox/models")
        .to_string_lossy()
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WhisperMode {
    Hot,
    #[default]
    Cold,
}

impl std::fmt::Display for WhisperMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhisperMode::Hot => write!(f, "hot"),
            WhisperMode::Cold => write!(f, "cold"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperConfig {
    #[serde(default)]
    pub mode: WhisperMode,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputConfig {
    pub delay_ms: u64,
    #[serde(default = "default_paste_delay")]
    pub paste_delay_ms: u64,
    #[serde(default)]
    pub append_trailing_space: bool,
    #[serde(default = "default_output_method")]
    pub method: String,
}

fn default_paste_delay() -> u64 {
    50
}

fn default_output_method() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UiConfig {
    pub show_notifications: bool,
    pub notification_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OverlayConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_overlay_width")]
    pub width: i32,
    #[serde(default = "default_overlay_height")]
    pub height: i32,
    #[serde(default = "default_overlay_position")]
    pub position: String,
    #[serde(default = "default_overlay_opacity")]
    pub opacity: f64,
}

fn default_true() -> bool {
    true
}

fn default_overlay_width() -> i32 {
    280
}

fn default_overlay_height() -> i32 {
    50
}

fn default_overlay_position() -> String {
    "bottom-center".to_string()
}

fn default_overlay_opacity() -> f64 {
    0.85
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 280,
            height: 50,
            position: "bottom-center".to_string(),
            opacity: 0.85,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReplacementsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_replacements_file")]
    pub file: String,
}

impl Default for ReplacementsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            file: default_replacements_file(),
        }
    }
}

fn default_replacements_file() -> String {
    dirs::config_dir()
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        })
        .join("tjvox/replacements.toml")
        .to_string_lossy()
        .to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HistoryConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_entries")]
    pub max_entries: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 1000,
        }
    }
}

fn default_max_entries() -> u32 {
    1000
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct InputConfig {
    #[serde(default)]
    pub ptt_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_llm_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_llm_prompt")]
    pub prompt: String,
    #[serde(default = "default_llm_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_llm_endpoint() -> String {
    "http://localhost:11434/v1/chat/completions".to_string()
}

fn default_llm_model() -> String {
    "llama3".to_string()
}

fn default_llm_prompt() -> String {
    "Fix grammar and punctuation. Output only the corrected text.".to_string()
}

fn default_llm_timeout_ms() -> u64 {
    5000
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_llm_endpoint(),
            api_key: String::new(),
            model: default_llm_model(),
            prompt: default_llm_prompt(),
            timeout_ms: default_llm_timeout_ms(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: AudioConfig {
                sample_rate: 16000,
                channels: 1,
                format: "wav".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("tjvox")
                    .to_string_lossy()
                    .to_string(),
            },
            transcription: TranscriptionConfig {
                model: "base".to_string(),
                models_dir: default_models_dir(),
                language: Some("en".to_string()),
                threads: None,
                remove_filler_words: false,
            },
            whisper: WhisperConfig {
                mode: WhisperMode::Cold,
            },
            output: OutputConfig {
                delay_ms: 100,
                paste_delay_ms: 50,
                append_trailing_space: true,
                method: "auto".to_string(),
            },
            ui: UiConfig {
                show_notifications: true,
                notification_timeout_ms: 3000,
            },
            overlay: OverlayConfig::default(),
            replacements: ReplacementsConfig::default(),
            history: HistoryConfig::default(),
            input: InputConfig::default(),
            llm: LlmConfig::default(),
        }
    }
}

impl Config {
    pub async fn load(path: &Path) -> Result<Self> {
        // Validate config path
        Self::validate_config_path(path)?;
        
        if !path.exists() {
            debug!("Config file not found at {:?}, creating default", path);
            let config = Self::default();
            config.save(path).await?;
            return Ok(config);
        }

        let content = fs::read_to_string(path).await?;
        let config: Config = toml::from_str(&content)?;
        
        // Validate loaded config values
        config.validate()?;
        
        Ok(config)
    }

    pub async fn save(&self, path: &Path) -> Result<()> {
        // Validate before saving
        self.validate()?;
        
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let content = toml::to_string_pretty(self)?;
        fs::write(path, content).await?;
        Ok(())
    }
    
    /// Validate configuration values
    pub fn validate(&self) -> Result<()> {
        use crate::error::TjvoxError;
        
        // Validate audio config
        if self.audio.sample_rate == 0 {
            return Err(TjvoxError::Config("sample_rate must be greater than 0".to_string()).into());
        }
        if self.audio.channels == 0 {
            return Err(TjvoxError::Config("channels must be greater than 0".to_string()).into());
        }
        
        // Validate temp_dir doesn't contain path traversal
        if self.audio.temp_dir.contains("..") {
            return Err(TjvoxError::Config(
                "temp_dir cannot contain path traversal sequences".to_string()
            ).into());
        }
        
        // Validate output config
        if self.output.paste_delay_ms > 10000 {
            return Err(TjvoxError::Config(
                "paste_delay_ms cannot exceed 10000ms".to_string()
            ).into());
        }
        
        // Validate overlay config
        if self.overlay.width < 50 || self.overlay.width > 1000 {
            return Err(TjvoxError::Config(
                "overlay width must be between 50 and 1000".to_string()
            ).into());
        }
        if self.overlay.height < 20 || self.overlay.height > 200 {
            return Err(TjvoxError::Config(
                "overlay height must be between 20 and 200".to_string()
            ).into());
        }
        if self.overlay.opacity < 0.0 || self.overlay.opacity > 1.0 {
            return Err(TjvoxError::Config(
                "overlay opacity must be between 0.0 and 1.0".to_string()
            ).into());
        }
        
        // Validate LLM config (only when enabled)
        if self.llm.enabled {
            if self.llm.endpoint.is_empty() {
                return Err(TjvoxError::Config(
                    "LLM endpoint cannot be empty when enabled".to_string()
                ).into());
            }
            if self.llm.model.is_empty() {
                return Err(TjvoxError::Config(
                    "LLM model cannot be empty when enabled".to_string()
                ).into());
            }
            if self.llm.timeout_ms < 1000 || self.llm.timeout_ms > 30000 {
                return Err(TjvoxError::Config(
                    "LLM timeout_ms must be between 1000 and 30000".to_string()
                ).into());
            }
        }

        // Validate history config
        if self.history.max_entries == 0 {
            return Err(TjvoxError::Config(
                "max_entries must be greater than 0".to_string()
            ).into());
        }
        if self.history.max_entries > 100000 {
            return Err(TjvoxError::Config(
                "max_entries cannot exceed 100000".to_string()
            ).into());
        }
        
        Ok(())
    }
    
    /// Validate that a config path is safe
    fn validate_config_path(path: &Path) -> Result<()> {
        use crate::error::TjvoxError;
        
        // Check for path traversal
        let path_str = path.to_string_lossy();
        if path_str.contains("..") {
            return Err(TjvoxError::Config(
                "Config path cannot contain path traversal sequences".to_string()
            ).into());
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_config_default_values() {
        let config = Config::default();
        assert_eq!(config.audio.sample_rate, 16000);
        assert_eq!(config.audio.channels, 1);
        assert_eq!(config.transcription.model, "base");
        assert!(config.overlay.enabled);
        assert_eq!(config.overlay.width, 280);
    }

    #[tokio::test]
    async fn test_config_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.toml");
        
        let config = Config::default();
        config.save(&config_path).await.unwrap();
        
        let loaded = Config::load(&config_path).await.unwrap();
        assert_eq!(loaded.audio.sample_rate, config.audio.sample_rate);
        assert_eq!(loaded.transcription.model, config.transcription.model);
    }

    #[tokio::test]
    async fn test_config_validation_invalid_sample_rate() {
        let mut config = Config::default();
        config.audio.sample_rate = 0;
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_config_validation_invalid_overlay_size() {
        let mut config = Config::default();
        config.overlay.width = 10; // Too small
        assert!(config.validate().is_err());
        
        config.overlay.width = 500;
        config.overlay.height = 500; // Too big
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_config_validation_invalid_opacity() {
        let mut config = Config::default();
        config.overlay.opacity = 1.5;
        assert!(config.validate().is_err());
        
        config.overlay.opacity = -0.5;
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_config_validation_path_traversal() {
        let mut config = Config::default();
        config.audio.temp_dir = "/tmp/../etc".to_string();
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_config_validation_valid_values() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_llm_config_defaults() {
        let config = LlmConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.endpoint, "http://localhost:11434/v1/chat/completions");
        assert_eq!(config.model, "llama3");
        assert_eq!(config.timeout_ms, 5000);
        assert!(config.api_key.is_empty());
    }

    #[tokio::test]
    async fn test_llm_validation_disabled_skips_checks() {
        let mut config = Config::default();
        config.llm.enabled = false;
        config.llm.endpoint = String::new(); // Would fail if enabled
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_llm_validation_empty_endpoint() {
        let mut config = Config::default();
        config.llm.enabled = true;
        config.llm.endpoint = String::new();
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_llm_validation_empty_model() {
        let mut config = Config::default();
        config.llm.enabled = true;
        config.llm.model = String::new();
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_llm_validation_timeout_too_low() {
        let mut config = Config::default();
        config.llm.enabled = true;
        config.llm.timeout_ms = 500;
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_llm_validation_timeout_too_high() {
        let mut config = Config::default();
        config.llm.enabled = true;
        config.llm.timeout_ms = 60000;
        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_llm_validation_valid_enabled() {
        let mut config = Config::default();
        config.llm.enabled = true;
        assert!(config.validate().is_ok());
    }
}
