pub mod audio;
pub mod config;
pub mod daemon;
pub mod error;
pub mod history;
pub mod input;
pub mod llm;
pub mod output;
pub mod ptt;
pub mod replacements;
pub mod socket;
pub mod transcription;
pub mod ui;

#[cfg(feature = "gui")]
pub mod gui;
#[cfg(feature = "gui")]
pub mod layer_shell;
#[cfg(feature = "gui")]
pub mod messages;

pub use audio::AudioRecorder;
pub use config::Config;
pub use error::TjvoxError;
pub use output::OutputManager;
pub use transcription::TranscriptionService;
pub use ui::UiManager;
