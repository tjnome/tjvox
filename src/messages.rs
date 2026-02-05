use crate::config::WhisperMode;

#[derive(Debug, Clone)]
pub enum DaemonMsg {
    StateChanged(RecordingState),
    Amplitude(f32),
    WhisperModeChanged(WhisperMode),
    ModelChanged(String),
    ModelLoading,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum GuiMsg {
    ToggleRecording,
    SetWhisperMode(WhisperMode),
    SetModel(String),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Recording,
    Transcribing,
    Typing,
}
