use anyhow::Result;
use tokio::fs;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};

use crate::audio::AudioRecorder;
use crate::config::{Config, WhisperMode};
use crate::history::HistoryStore;
use crate::llm::LlmProcessor;
use crate::output::OutputManager;
use crate::replacements::ReplacementEngine;
use crate::socket::{SocketCommand, SocketServer};
use crate::transcription::TranscriptionService;
use crate::ui::UiManager;

#[cfg(feature = "gui")]
use crate::messages::{DaemonMsg, GuiMsg, RecordingState};

#[derive(Debug, Clone, PartialEq)]
pub enum DaemonState {
    Idle,
    Recording,
    Transcribing,
    Typing,
}

impl std::fmt::Display for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonState::Idle => write!(f, "idle"),
            DaemonState::Recording => write!(f, "recording"),
            DaemonState::Transcribing => write!(f, "transcribing"),
            DaemonState::Typing => write!(f, "typing"),
        }
    }
}

pub struct Daemon {
    config: Config,
    state: DaemonState,
    recorder: Option<AudioRecorder>,
    transcriber: TranscriptionService,
    ui: UiManager,
    whisper_mode: WhisperMode,
    amplitude_tx: Option<std::sync::mpsc::Sender<f32>>,
    replacement_engine: Option<ReplacementEngine>,
    llm_processor: Option<LlmProcessor>,
    history: Option<HistoryStore>,
    recording_start: Option<std::time::Instant>,
    #[cfg(feature = "gui")]
    gui_rx: Option<async_channel::Receiver<GuiMsg>>,
    #[cfg(feature = "gui")]
    daemon_txs: Vec<async_channel::Sender<DaemonMsg>>,
}

impl Daemon {
    pub async fn new(config: Config) -> Result<Self> {
        let ui = UiManager::with_config(&config.ui);
        let transcriber = TranscriptionService::new(&config.transcription)?;
        let whisper_mode = config.whisper.mode;

        // Load replacement engine if enabled
        let replacement_engine = if config.replacements.enabled {
            let path = std::path::PathBuf::from(&config.replacements.file);
            match ReplacementEngine::load(&path) {
                Ok(engine) => Some(engine),
                Err(e) => {
                    warn!("Failed to load replacements: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Build LLM processor if enabled
        let llm_processor = if config.llm.enabled {
            match LlmProcessor::new(&config.llm) {
                Ok(processor) => Some(processor),
                Err(e) => {
                    warn!("Failed to create LLM processor: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Open history store if enabled
        let history = if config.history.enabled {
            let db_path = dirs::data_dir()
                .unwrap_or_else(|| {
                    std::env::var("HOME")
                        .map(|h| std::path::PathBuf::from(h).join(".local/share"))
                        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
                })
                .join("tjvox/history.db");
            match HistoryStore::open(&db_path, config.history.max_entries) {
                Ok(store) => Some(store),
                Err(e) => {
                    warn!("Failed to open history store: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Write PID file to user-private runtime directory
        let pid = std::process::id();
        let uid = unsafe { libc::getuid() };
        let pid_file = std::path::PathBuf::from(format!("/run/user/{}/tjvox.pid", uid));
        fs::write(&pid_file, pid.to_string()).await?;

        Ok(Self {
            config,
            state: DaemonState::Idle,
            recorder: None,
            transcriber,
            ui,
            whisper_mode,
            amplitude_tx: None,
            replacement_engine,
            llm_processor,
            history,
            recording_start: None,
            #[cfg(feature = "gui")]
            gui_rx: None,
            #[cfg(feature = "gui")]
            daemon_txs: Vec::new(),
        })
    }

    #[cfg(feature = "gui")]
    pub fn with_channels(
        mut self,
        gui_rx: async_channel::Receiver<GuiMsg>,
        daemon_txs: Vec<async_channel::Sender<DaemonMsg>>,
        amplitude_tx: Option<std::sync::mpsc::Sender<f32>>,
    ) -> Self {
        self.gui_rx = Some(gui_rx);
        self.daemon_txs = daemon_txs;
        self.amplitude_tx = amplitude_tx;
        self
    }

    async fn prewarm_if_hot(&mut self) {
        if self.whisper_mode == WhisperMode::Hot {
            info!("Hot mode: pre-warming whisper model on startup");
            if let Err(e) = self.transcriber.prewarm().await {
                warn!("Failed to pre-warm model: {}", e);
            }
        }
    }

    #[cfg(feature = "gui")]
    fn broadcast(&self, msg: DaemonMsg) {
        for tx in &self.daemon_txs {
            let _ = tx.try_send(msg.clone());
        }
    }

    #[cfg(feature = "gui")]
    fn notify_state(&self) {
        let state = match self.state {
            DaemonState::Idle => RecordingState::Idle,
            DaemonState::Recording => RecordingState::Recording,
            DaemonState::Transcribing => RecordingState::Transcribing,
            DaemonState::Typing => RecordingState::Typing,
        };
        self.broadcast(DaemonMsg::StateChanged(state));
    }

    #[cfg(not(feature = "gui"))]
    fn notify_state(&self) {}

    #[cfg(feature = "gui")]
    fn notify_whisper_mode(&self) {
        self.broadcast(DaemonMsg::WhisperModeChanged(self.whisper_mode));
    }

    #[cfg(feature = "gui")]
    fn notify_model_changed(&self) {
        self.broadcast(DaemonMsg::ModelChanged(self.config.transcription.model.clone()));
    }

    pub async fn run(mut self) -> Result<()> {
        info!(
            "Daemon running (PID {}), send SIGUSR1 to toggle recording",
            std::process::id()
        );
        self.ui
            .show_notification("TJvox", "Daemon started. Send SIGUSR1 to toggle.")
            .await?;

        // Pre-warm model if hot mode
        self.prewarm_if_hot().await;

        // Notify GUI of initial state
        self.notify_state();
        #[cfg(feature = "gui")]
        {
            self.notify_whisper_mode();
            self.notify_model_changed();
        }

        // Start socket server for IPC
        let socket_server = match SocketServer::bind().await {
            Ok(server) => Some(server),
            Err(e) => {
                warn!("Failed to start socket server: {}", e);
                None
            }
        };

        // Start PTT monitor if configured
        #[cfg(feature = "ptt")]
        let mut ptt_rx = {
            if let Some(ref key_name) = self.config.input.ptt_key {
                let (tx, rx) = tokio::sync::mpsc::channel(32);
                match crate::ptt::monitor::PttMonitor::new(key_name) {
                    Ok(monitor) => {
                        tokio::spawn(async move {
                            if let Err(e) = monitor.run(tx).await {
                                tracing::error!("PTT monitor error: {}", e);
                            }
                        });
                        Some(rx)
                    }
                    Err(e) => {
                        warn!("Failed to start PTT monitor: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        };

        let mut sigusr1 = signal(SignalKind::user_defined1())?;
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        loop {
            // Helper future for socket accept
            let socket_accept = async {
                if let Some(ref server) = socket_server {
                    server.accept().await
                } else {
                    // Never resolves if no socket server
                    std::future::pending().await
                }
            };

            // Helper future for PTT events
            #[cfg(feature = "ptt")]
            let ptt_recv = async {
                if let Some(ref mut rx) = ptt_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            };

            #[cfg(feature = "gui")]
            {
                if let Some(ref gui_rx) = self.gui_rx {
                    #[cfg(feature = "ptt")]
                    tokio::select! {
                        _ = sigusr1.recv() => {
                            self.handle_toggle().await;
                        }
                        _ = sigterm.recv() => {
                            info!("Received SIGTERM, shutting down...");
                            break;
                        }
                        _ = sigint.recv() => {
                            info!("Received SIGINT, shutting down...");
                            break;
                        }
                        result = socket_accept => {
                            if let Ok((cmd, stream)) = result {
                                if self.handle_socket_command(cmd, stream).await {
                                    info!("Quit requested via socket");
                                    break;
                                }
                            }
                        }
                        evt = ptt_recv => {
                            if let Some(evt) = evt {
                                self.handle_ptt_event(evt).await;
                            }
                        }
                        msg = gui_rx.recv() => {
                            match msg {
                                Ok(GuiMsg::ToggleRecording) => {
                                    self.handle_toggle().await;
                                }
                                Ok(GuiMsg::SetWhisperMode(mode)) => {
                                    self.set_whisper_mode(mode).await;
                                }
                                Ok(GuiMsg::SetModel(model)) => {
                                    self.set_model(model).await;
                                }
                                Ok(GuiMsg::Quit) => {
                                    info!("Quit requested from GUI");
                                    break;
                                }
                                Err(_) => {
                                    info!("GUI channel closed, shutting down");
                                    break;
                                }
                            }
                        }
                    }

                    #[cfg(not(feature = "ptt"))]
                    tokio::select! {
                        _ = sigusr1.recv() => {
                            self.handle_toggle().await;
                        }
                        _ = sigterm.recv() => {
                            info!("Received SIGTERM, shutting down...");
                            break;
                        }
                        _ = sigint.recv() => {
                            info!("Received SIGINT, shutting down...");
                            break;
                        }
                        result = socket_accept => {
                            if let Ok((cmd, stream)) = result {
                                if self.handle_socket_command(cmd, stream).await {
                                    info!("Quit requested via socket");
                                    break;
                                }
                            }
                        }
                        msg = gui_rx.recv() => {
                            match msg {
                                Ok(GuiMsg::ToggleRecording) => {
                                    self.handle_toggle().await;
                                }
                                Ok(GuiMsg::SetWhisperMode(mode)) => {
                                    self.set_whisper_mode(mode).await;
                                }
                                Ok(GuiMsg::SetModel(model)) => {
                                    self.set_model(model).await;
                                }
                                Ok(GuiMsg::Quit) => {
                                    info!("Quit requested from GUI");
                                    break;
                                }
                                Err(_) => {
                                    info!("GUI channel closed, shutting down");
                                    break;
                                }
                            }
                        }
                    }

                    continue;
                }
            }

            // Headless mode (no GUI channels)
            #[cfg(feature = "ptt")]
            tokio::select! {
                _ = sigusr1.recv() => {
                    self.handle_toggle().await;
                }
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, shutting down...");
                    break;
                }
                _ = sigint.recv() => {
                    info!("Received SIGINT, shutting down...");
                    break;
                }
                result = socket_accept => {
                    if let Ok((cmd, stream)) = result {
                        self.handle_socket_command(cmd, stream).await;
                    }
                }
                evt = ptt_recv => {
                    if let Some(evt) = evt {
                        self.handle_ptt_event(evt).await;
                    }
                }
            }

            #[cfg(not(feature = "ptt"))]
            tokio::select! {
                _ = sigusr1.recv() => {
                    self.handle_toggle().await;
                }
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, shutting down...");
                    break;
                }
                _ = sigint.recv() => {
                    info!("Received SIGINT, shutting down...");
                    break;
                }
                result = socket_accept => {
                    if let Ok((cmd, stream)) = result {
                        self.handle_socket_command(cmd, stream).await;
                    }
                }
            }
        }

        self.shutdown().await;
        Ok(())
    }

    async fn handle_toggle(&mut self) {
        match self.state {
            DaemonState::Idle => {
                if let Err(e) = self.start_recording().await {
                    error!("Failed to start recording: {}", e);
                    let _ = self.ui.show_error("TJvox", &e.to_string()).await;
                    self.state = DaemonState::Idle;
                    self.notify_state();
                }
            }
            DaemonState::Recording => {
                if let Err(e) = self.stop_and_transcribe().await {
                    error!("Failed to transcribe: {}", e);
                    let _ = self.ui.show_error("TJvox", &e.to_string()).await;
                    self.state = DaemonState::Idle;
                    self.notify_state();
                }
            }
            _ => {
                warn!("Toggle received during {} state, ignoring", self.state);
            }
        }
    }

    async fn start_recording(&mut self) -> Result<()> {
        info!("Starting recording");
        self.state = DaemonState::Recording;
        self.recording_start = Some(std::time::Instant::now());
        self.notify_state();

        let mut recorder = AudioRecorder::new(&self.config.audio, self.amplitude_tx.clone())?;
        recorder.start().await?;
        self.recorder = Some(recorder);

        // Note: Parallel model loading removed - model will be loaded at transcription time
        // This simplifies the async code and avoids lifetime issues with the transcriber

        self.ui
            .show_notification("TJvox", "Recording...")
            .await?;
        Ok(())
    }

    async fn stop_and_transcribe(&mut self) -> Result<()> {
        info!("Stopping recording and transcribing");
        self.state = DaemonState::Transcribing;
        self.notify_state();
        self.ui
            .show_notification("TJvox", "Transcribing...")
            .await?;

        let audio_path = match self.recorder.as_mut() {
            Some(recorder) => recorder.stop().await?,
            None => {
                self.state = DaemonState::Idle;
                self.notify_state();
                return Err(anyhow::anyhow!("No active recorder"));
            }
        };

        // Transcribe using whisper-rs (model loads if not already loaded)
        let text = self.transcriber.transcribe(&audio_path).await?;

        // LLM post-processing (grammar/punctuation correction)
        let text = if let Some(ref llm) = self.llm_processor {
            match llm.process(&text).await {
                Ok(corrected) => corrected,
                Err(e) => {
                    warn!("LLM processing failed, using original text: {}", e);
                    text
                }
            }
        } else {
            text
        };

        // Apply post-processing
        let text = self.post_process(&text);

        let duration_ms = self
            .recording_start
            .map(|start| start.elapsed().as_millis() as u64)
            .unwrap_or(0);

        if text.trim().is_empty() {
            self.ui
                .show_notification("TJvox", "No speech detected")
                .await?;
        } else {
            // Type/paste the result
            self.state = DaemonState::Typing;
            self.notify_state();
            let output = OutputManager::new(&self.config.output)?;
            output.type_text(&text).await?;
            self.ui
                .show_notification(
                    "TJvox",
                    &format!("Typed: {}", &text[..text.len().min(50)]),
                )
                .await?;

            // Save to history
            if let Some(ref history) = self.history {
                let entry = crate::history::HistoryEntry {
                    id: 0,
                    timestamp: String::new(),
                    duration_ms,
                    text: text.clone(),
                    model: self.config.transcription.model.clone(),
                    language: self
                        .config
                        .transcription
                        .language
                        .clone()
                        .unwrap_or_default(),
                };
                if let Err(e) = history.save(&entry) {
                    warn!("Failed to save history entry: {}", e);
                }
            }
        }

        // Cleanup
        if let Some(recorder) = self.recorder.take() {
            recorder.cleanup().await.ok();
        }

        // Unload model in cold mode
        if self.whisper_mode == WhisperMode::Cold {
            self.transcriber.unload_model();
        }

        self.state = DaemonState::Idle;
        self.notify_state();
        self.ui.show_notification("TJvox", "Ready").await?;
        Ok(())
    }

    fn post_process(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Apply word replacements before filler word removal
        if let Some(ref engine) = self.replacement_engine {
            result = engine.apply(&result);
        }

        // Remove filler words if configured (case-insensitive)
        if self.config.transcription.remove_filler_words {
            let filler_patterns = [
                r"(?i)\buh\b[,]?",
                r"(?i)\bum\b[,]?",
                r"(?i)\bhmm\b[,]?",
                r"(?i)\blike\b[,]?",
                r"(?i)\byou know\b[,]?",
            ];
            for pattern in &filler_patterns {
                if let Ok(re) = regex::Regex::new(pattern) {
                    result = re.replace_all(&result, " ").to_string();
                }
            }
            // Clean up double spaces
            while result.contains("  ") {
                result = result.replace("  ", " ");
            }
            result = result.trim().to_string();
        }

        // Append trailing space if configured
        if self.config.output.append_trailing_space && !result.is_empty() {
            result.push(' ');
        }

        result
    }

    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    async fn set_whisper_mode(&mut self, mode: WhisperMode) {
        info!("Switching whisper mode to: {}", mode);
        self.whisper_mode = mode;
        match mode {
            WhisperMode::Hot => {
                if !self.transcriber.is_loaded() {
                    #[cfg(feature = "gui")]
                    self.broadcast(DaemonMsg::ModelLoading);
                    if let Err(e) = self.transcriber.load_model().await {
                        error!("Failed to load model for hot mode: {}", e);
                    }
                }
            }
            WhisperMode::Cold => {
                if self.state == DaemonState::Idle {
                    self.transcriber.unload_model();
                }
            }
        }
        #[cfg(feature = "gui")]
        self.notify_whisper_mode();
    }

    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    async fn set_model(&mut self, model: String) {
        if model == self.config.transcription.model {
            return;
        }
        info!("Switching model to: {}", model);
        #[cfg(feature = "gui")]
        self.broadcast(DaemonMsg::ModelLoading);
        // Unload current model so next transcription loads the new one
        self.transcriber.unload_model();
        self.config.transcription.model = model;
        // Recreate transcriber with new config
        match TranscriptionService::new(&self.config.transcription) {
            Ok(t) => {
                self.transcriber = t;
                // If hot mode, load new model immediately
                if self.whisper_mode == WhisperMode::Hot {
                    if let Err(e) = self.transcriber.load_model().await {
                        error!("Failed to load new model: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to create transcriber for new model: {}", e);
            }
        }
        #[cfg(feature = "gui")]
        self.notify_model_changed();
    }

    async fn handle_socket_command(
        &mut self,
        cmd: SocketCommand,
        mut stream: tokio::net::UnixStream,
    ) -> bool {
        use tokio::io::AsyncWriteExt;

        let mut should_quit = false;
        let response = match cmd {
            SocketCommand::Toggle => {
                self.handle_toggle().await;
                format!("ok: {}", self.state)
            }
            SocketCommand::PushStart => {
                self.handle_push_start().await;
                format!("ok: {}", self.state)
            }
            SocketCommand::PushStop => {
                self.handle_push_stop().await;
                format!("ok: {}", self.state)
            }
            SocketCommand::Status => {
                format!("ok: {}", self.state)
            }
            SocketCommand::Quit => {
                should_quit = true;
                "ok: quitting".to_string()
            }
        };

        let _ = stream.write_all(format!("{}\n", response).as_bytes()).await;
        should_quit
    }

    async fn handle_push_start(&mut self) {
        if self.state != DaemonState::Idle {
            info!("Push-start ignored: currently in {} state", self.state);
            return;
        }
        if let Err(e) = self.start_recording().await {
            error!("Failed to start recording (push): {}", e);
            let _ = self.ui.show_error("TJvox", &e.to_string()).await;
            self.state = DaemonState::Idle;
            self.notify_state();
        }
    }

    async fn handle_push_stop(&mut self) {
        if self.state != DaemonState::Recording {
            info!("Push-stop ignored: currently in {} state", self.state);
            return;
        }
        if let Err(e) = self.stop_and_transcribe().await {
            error!("Failed to transcribe (push): {}", e);
            let _ = self.ui.show_error("TJvox", &e.to_string()).await;
            self.state = DaemonState::Idle;
            self.notify_state();
        }
    }

    #[cfg(feature = "ptt")]
    async fn handle_ptt_event(&mut self, evt: crate::ptt::monitor::PttEvent) {
        match evt {
            crate::ptt::monitor::PttEvent::KeyDown => self.handle_push_start().await,
            crate::ptt::monitor::PttEvent::KeyUp => self.handle_push_stop().await,
        }
    }

    async fn shutdown(mut self) {
        // Stop any active recording
        if let Some(mut recorder) = self.recorder.take() {
            let _ = recorder.stop().await;
            let _ = recorder.cleanup().await;
        }

        // Unload model
        self.transcriber.unload_model();

        // Remove PID file
        let uid = unsafe { libc::getuid() };
        let pid_file = std::path::PathBuf::from(format!("/run/user/{}/tjvox.pid", uid));
        let _ = fs::remove_file(&pid_file).await;
        info!("Daemon shut down cleanly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_state_display() {
        assert_eq!(format!("{}", DaemonState::Idle), "idle");
        assert_eq!(format!("{}", DaemonState::Recording), "recording");
        assert_eq!(format!("{}", DaemonState::Transcribing), "transcribing");
        assert_eq!(format!("{}", DaemonState::Typing), "typing");
    }

    #[test]
    fn test_daemon_state_clone() {
        let state = DaemonState::Recording;
        let cloned = state.clone();
        assert_eq!(state, cloned);
    }

    #[test]
    fn test_daemon_state_equality() {
        assert_eq!(DaemonState::Idle, DaemonState::Idle);
        assert_ne!(DaemonState::Idle, DaemonState::Recording);
        assert_ne!(DaemonState::Recording, DaemonState::Transcribing);
        assert_ne!(DaemonState::Transcribing, DaemonState::Typing);
    }
}
