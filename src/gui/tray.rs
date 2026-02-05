use ksni::TrayMethods;
use tracing::{error, info};

use crate::config::WhisperMode;
use crate::messages::{DaemonMsg, GuiMsg, RecordingState};

const MODELS: &[&str] = &["tiny", "base", "small", "medium", "large-v3-turbo"];

struct TjvoxTray {
    state: RecordingState,
    whisper_mode: WhisperMode,
    current_model: String,
    model_loading: bool,
    gui_tx: async_channel::Sender<GuiMsg>,
}

impl ksni::Tray for TjvoxTray {
    fn id(&self) -> String {
        "tjvox".to_string()
    }

    fn title(&self) -> String {
        "TJvox".to_string()
    }

    fn icon_name(&self) -> String {
        match self.state {
            RecordingState::Idle => "microphone-sensitivity-muted-symbolic".to_string(),
            RecordingState::Recording => "microphone-sensitivity-high-symbolic".to_string(),
            RecordingState::Transcribing => "system-run-symbolic".to_string(),
            RecordingState::Typing => "system-run-symbolic".to_string(),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = match self.state {
            RecordingState::Idle => format!("Idle ({}, {})", self.whisper_mode, self.current_model),
            RecordingState::Recording => "Recording...".to_string(),
            RecordingState::Transcribing => "Transcribing...".to_string(),
            RecordingState::Typing => "Typing...".to_string(),
        };
        ksni::ToolTip {
            title: "TJvox".to_string(),
            description,
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let status_label = if self.model_loading {
            "Loading model..."
        } else {
            match self.state {
                RecordingState::Idle => "Idle",
                RecordingState::Recording => "Recording...",
                RecordingState::Transcribing => "Transcribing...",
                RecordingState::Typing => "Typing...",
            }
        };

        // Model submenu items
        let model_items: Vec<ksni::MenuItem<Self>> = MODELS
            .iter()
            .map(|&model| {
                let model_str = model.to_string();
                let is_current = self.current_model == model;
                ksni::MenuItem::Standard(ksni::menu::StandardItem {
                    label: format!("{}{}", model, if is_current { " ●" } else { "" }),
                    activate: Box::new(move |tray: &mut Self| {
                        let _ = tray.gui_tx.try_send(GuiMsg::SetModel(model_str.clone()));
                    }),
                    ..Default::default()
                })
            })
            .collect();

        vec![
            // Status display (not clickable)
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: status_label.to_string(),
                enabled: false,
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
            // Setup info
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: "Setup: Bind 'tjvox toggle' to a key".to_string(),
                enabled: false,
                ..Default::default()
            }),
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: "KDE Settings → Shortcuts → Custom".to_string(),
                enabled: false,
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
            // Model selection submenu
            ksni::MenuItem::SubMenu(ksni::menu::SubMenu {
                label: format!("Model: {}", self.current_model),
                submenu: model_items,
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
            // Whisper mode
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: format!(
                    "Cold Mode{}",
                    if self.whisper_mode == WhisperMode::Cold {
                        " ●"
                    } else {
                        ""
                    }
                ),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray
                        .gui_tx
                        .try_send(GuiMsg::SetWhisperMode(WhisperMode::Cold));
                }),
                ..Default::default()
            }),
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: format!(
                    "Hot Mode{}",
                    if self.whisper_mode == WhisperMode::Hot {
                        " ●"
                    } else {
                        ""
                    }
                ),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray
                        .gui_tx
                        .try_send(GuiMsg::SetWhisperMode(WhisperMode::Hot));
                }),
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.gui_tx.try_send(GuiMsg::Quit);
                }),
                ..Default::default()
            }),
        ]
    }
}

pub async fn run_tray(
    gui_tx: async_channel::Sender<GuiMsg>,
    daemon_rx: async_channel::Receiver<DaemonMsg>,
) {
    info!("Starting system tray");

    let tray = TjvoxTray {
        state: RecordingState::Idle,
        whisper_mode: WhisperMode::Cold,
        current_model: "base".to_string(),
        model_loading: false,
        gui_tx,
    };

    let handle: ksni::Handle<TjvoxTray> = match tray.spawn().await {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to create tray: {}", e);
            return;
        }
    };

    // Listen for daemon state updates and update tray
    while let Ok(msg) = daemon_rx.recv().await {
        match msg {
            DaemonMsg::StateChanged(state) => {
                handle
                    .update(|tray| {
                        tray.state = state;
                    })
                    .await;
            }
            DaemonMsg::WhisperModeChanged(mode) => {
                handle
                    .update(|tray| {
                        tray.whisper_mode = mode;
                    })
                    .await;
            }
            DaemonMsg::ModelChanged(model) => {
                handle
                    .update(|tray| {
                        tray.current_model = model;
                        tray.model_loading = false;
                    })
                    .await;
            }
            DaemonMsg::ModelLoading => {
                handle
                    .update(|tray| {
                        tray.model_loading = true;
                    })
                    .await;
            }
            _ => {}
        }
    }

    info!("Tray shutting down");
}
