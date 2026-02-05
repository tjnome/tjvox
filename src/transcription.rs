use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::config::TranscriptionConfig;
use crate::error::TjvoxError;

const HF_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Valid Whisper model names that can be downloaded
const VALID_MODELS: &[&str] = &["tiny", "base", "small", "medium", "large-v3-turbo"];

/// Transcription service that handles Whisper model loading and audio transcription
pub struct TranscriptionService {
    config: TranscriptionConfig,
    context: Option<WhisperContext>,
    model_path: PathBuf,
}

/// Validate that a model name is safe and known
fn validate_model_name(name: &str) -> Result<()> {
    // Check for empty name
    if name.is_empty() {
        return Err(TjvoxError::Config("Model name cannot be empty".to_string()).into());
    }

    // Check for path traversal attempts
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(TjvoxError::Config(
            format!("Invalid model name '{}': contains path separators", name)
        ).into());
    }

    // Check for null bytes
    if name.contains('\0') {
        return Err(TjvoxError::Config(
            "Invalid model name: contains null bytes".to_string()
        ).into());
    }

    // Check against known valid models
    if !VALID_MODELS.contains(&name) {
        warn!("Model '{}' is not in the known model list: {:?}", name, VALID_MODELS);
        // We don't error here because users might use custom models
        // But we log a warning
    }

    Ok(())
}

impl TranscriptionService {
    pub fn new(config: &TranscriptionConfig) -> Result<Self> {
        // Validate model name
        validate_model_name(&config.model)?;
        
        // Validate models_dir doesn't contain path traversal
        if config.models_dir.contains("..") {
            return Err(TjvoxError::Config(
                "models_dir cannot contain path traversal sequences".to_string()
            ).into());
        }
        
        let model_filename = format!("ggml-{}.bin", config.model);
        let model_path = PathBuf::from(&config.models_dir).join(&model_filename);

        Ok(Self {
            config: config.clone(),
            context: None,
            model_path,
        })
    }

    pub async fn ensure_model(&self) -> Result<()> {
        if self.model_path.exists() {
            debug!("Model already exists: {}", self.model_path.display());
            return Ok(());
        }

        let model_filename = format!("ggml-{}.bin", self.config.model);
        let url = format!("{}/{}", HF_BASE_URL, model_filename);

        info!("Downloading model '{}' from {}", self.config.model, url);

        // Ensure models directory exists
        if let Some(parent) = self.model_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                TjvoxError::ModelDownload(format!(
                    "Failed to create models directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // Run blocking HTTP request in spawn_blocking to avoid blocking async runtime
        let url_clone = url.clone();
        let model_name = self.config.model.clone();
        let model_path = self.model_path.clone();
        
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let response = reqwest::blocking::get(&url_clone).map_err(|e| {
                TjvoxError::ModelDownload(format!("Failed to download model: {}", e))
            })?;

            if !response.status().is_success() {
                return Err(TjvoxError::ModelDownload(format!(
                    "HTTP {} when downloading model '{}'. Available models: tiny, base, small, medium, large-v3-turbo",
                    response.status(),
                    model_name
                )).into());
            }

            let bytes = response.bytes().map_err(|e| {
                TjvoxError::ModelDownload(format!("Failed to read model data: {}", e))
            })?;

            Ok(bytes.to_vec())
        }).await.map_err(|e| {
            TjvoxError::ModelDownload(format!("Download task failed: {}", e))
        })??;

        info!(
            "Downloaded {} bytes, saving to {}",
            bytes.len(),
            model_path.display()
        );

        tokio::fs::write(&model_path, &bytes).await.map_err(|e| {
            TjvoxError::ModelDownload(format!(
                "Failed to write model to {}: {}",
                model_path.display(),
                e
            ))
        })?;

        info!("Model '{}' downloaded successfully", self.config.model);
        Ok(())
    }

    pub async fn load_model(&mut self) -> Result<()> {
        if self.context.is_some() {
            debug!("Model already loaded");
            return Ok(());
        }

        self.ensure_model().await?;

        info!("Loading whisper model from {}", self.model_path.display());

        let model_path = self.model_path.clone();
        let ctx = tokio::task::spawn_blocking(move || {
            WhisperContext::new_with_params(
                model_path.to_str().ok_or_else(|| {
                    TjvoxError::ModelLoad("Invalid model path encoding".to_string())
                })?,
                WhisperContextParameters::default(),
            )
            .map_err(|e| TjvoxError::ModelLoad(format!("Failed to load whisper model: {}", e)))
        })
        .await
        .map_err(|e| TjvoxError::ModelLoad(format!("Model load task failed: {}", e)))??;

        self.context = Some(ctx);
        info!("Whisper model loaded successfully");
        Ok(())
    }

    pub fn unload_model(&mut self) {
        if self.context.is_some() {
            info!("Unloading whisper model");
            self.context = None;
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.context.is_some()
    }

    pub async fn transcribe(&mut self, audio_path: &Path) -> Result<String> {
        info!("Transcribing: {}", audio_path.display());

        // Load model if not already loaded
        if self.context.is_none() {
            self.load_model().await?;
        }

        let samples = self.read_audio(audio_path)?;

        let ctx = self.context.as_ref().ok_or_else(|| {
            TjvoxError::Transcription("Model not loaded".to_string())
        })?;

        let mut state = ctx.create_state().map_err(|e| {
            TjvoxError::Transcription(format!("Failed to create whisper state: {}", e))
        })?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Configure language
        if let Some(ref lang) = self.config.language {
            params.set_language(Some(lang));
        }

        // Configure threads
        let threads = self.config.threads.unwrap_or_else(|| {
            let cpus = num_cpus::get() as u32;
            std::cmp::max(1, std::cmp::min(8, cpus.saturating_sub(2)))
        });
        params.set_n_threads(threads as i32);

        // Low temperature for deterministic output
        params.set_temperature(0.2);

        // Disable printing to stdout
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state.full(params, &samples).map_err(|e| {
            TjvoxError::Transcription(format!("Whisper transcription failed: {}", e))
        })?;

        let num_segments = state.full_n_segments();

        let mut text = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(segment_text) = segment.to_str() {
                    text.push_str(segment_text);
                }
            }
        }

        let result = text.trim().to_string();
        info!("Transcription completed: {} chars", result.len());
        Ok(result)
    }

    fn read_audio(&self, audio_path: &Path) -> Result<Vec<f32>> {
        let reader = hound::WavReader::open(audio_path).map_err(|e| {
            TjvoxError::Transcription(format!(
                "Failed to open WAV file {}: {}",
                audio_path.display(),
                e
            ))
        })?;

        let spec = reader.spec();
        debug!(
            "WAV: {} Hz, {} channels, {:?}, {} bits",
            spec.sample_rate, spec.channels, spec.sample_format, spec.bits_per_sample
        );

        // Read samples as f32
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .into_samples::<f32>()
                .filter_map(|s| s.ok())
                .collect(),
            hound::SampleFormat::Int => {
                let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .filter_map(|s| s.ok())
                    .map(|s| s as f32 / max_val)
                    .collect()
            }
        };

        // Convert to mono if stereo
        let mono = if spec.channels > 1 {
            samples
                .chunks(spec.channels as usize)
                .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
                .collect()
        } else {
            samples
        };

        // Resample to 16kHz if needed
        let resampled = if spec.sample_rate != 16000 {
            warn!(
                "Audio is {} Hz, resampling to 16000 Hz (simple linear)",
                spec.sample_rate
            );
            Self::resample(&mono, spec.sample_rate, 16000)
        } else {
            mono
        };

        debug!("Audio loaded: {} samples at 16kHz", resampled.len());
        Ok(resampled)
    }

    fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        let ratio = from_rate as f64 / to_rate as f64;
        let output_len = (input.len() as f64 / ratio) as usize;
        let mut output = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_idx = i as f64 * ratio;
            let idx = src_idx as usize;
            let frac = src_idx - idx as f64;

            let sample = if idx + 1 < input.len() {
                input[idx] as f64 * (1.0 - frac) + input[idx + 1] as f64 * frac
            } else if idx < input.len() {
                input[idx] as f64
            } else {
                0.0
            };

            output.push(sample as f32);
        }

        output
    }

    /// Pre-warm the model by running a dummy transcription on silence.
    /// This warms up whisper.cpp's internal buffers.
    pub async fn prewarm(&mut self) -> Result<()> {
        if self.context.is_none() {
            self.load_model().await?;
        }

        info!("Pre-warming whisper model...");

        let ctx = self.context.as_ref().ok_or_else(|| {
            TjvoxError::Transcription("Model not loaded for prewarm".to_string())
        })?;

        let mut state = ctx.create_state().map_err(|e| {
            TjvoxError::Transcription(format!("Failed to create whisper state for prewarm: {}", e))
        })?;

        // 1 second of silence at 16kHz
        let silence: Vec<f32> = vec![0.0; 16000];
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(1);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let _ = state.full(params, &silence);
        info!("Whisper model pre-warmed");
        Ok(())
    }
}
