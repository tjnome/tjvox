mod overlay;
mod tray;

use anyhow::Result;
use gtk4::prelude::*;
use gtk4::{self, glib};
use tracing::{error, info};

use crate::config::Config;
use crate::daemon::Daemon;
use crate::layer_shell::LayerShellFns;
use crate::messages::{DaemonMsg, GuiMsg, RecordingState};

pub fn run_gui(config: Config) -> Result<()> {
    let application = gtk4::Application::builder()
        .application_id("com.github.tjnome.tjvox")
        .build();

    let config_clone = config.clone();

    application.connect_activate(move |app| {
        if let Err(e) = setup_gui(app, config_clone.clone()) {
            error!("Failed to setup GUI: {}", e);
        }
    });

    application.run_with_args::<String>(&[]);
    Ok(())
}

fn setup_gui(app: &gtk4::Application, config: Config) -> Result<()> {
    // Create separate daemon→listener channels (broadcast pattern)
    // Each listener gets its own receiver so every message reaches all listeners
    let (overlay_tx, overlay_rx) = async_channel::bounded::<DaemonMsg>(32);
    let (tray_tx, tray_rx) = async_channel::bounded::<DaemonMsg>(32);

    // Single GUI→daemon channel (multiple senders, one receiver)
    let (gui_tx, gui_rx) = async_channel::bounded::<GuiMsg>(32);

    // Amplitude channel: std::sync::mpsc from PipeWire audio thread → GTK timer poll
    let (amp_tx, amp_rx) = std::sync::mpsc::channel::<f32>();

    // Try to load gtk4-layer-shell for wlroots compositors
    let layer_shell = LayerShellFns::load();

    // Create the overlay window
    let overlay = overlay::OverlayWindow::new(app, &config.overlay, layer_shell.as_ref());

    // Spawn the tray in the tokio runtime (background thread)
    let tray_gui_tx = gui_tx.clone();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create tokio runtime: {}", e))?;

    // Spawn daemon in tokio runtime with broadcast senders
    let daemon_config = config.clone();
    let daemon_txs = vec![overlay_tx, tray_tx];
    rt.spawn(async move {
        match Daemon::new(daemon_config).await {
            Ok(daemon) => {
                let daemon = daemon.with_channels(gui_rx, daemon_txs, Some(amp_tx));
                if let Err(e) = daemon.run().await {
                    error!("Daemon error: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to create daemon: {}", e);
            }
        }
    });

    // Spawn tray in tokio runtime with its own dedicated receiver
    rt.spawn(async move {
        tray::run_tray(tray_gui_tx, tray_rx).await;
    });

    // Listen for daemon messages on the GTK main thread (overlay's dedicated receiver)
    let overlay_for_daemon = overlay.clone();
    let app_for_quit = app.clone();
    glib::spawn_future_local(async move {
        while let Ok(msg) = overlay_rx.recv().await {
            match msg {
                DaemonMsg::StateChanged(state) => {
                    info!("State changed: {:?}", state);
                    overlay_for_daemon.set_state(state);

                    match state {
                        RecordingState::Recording => {
                            overlay_for_daemon.show();
                        }
                        RecordingState::Transcribing => {
                            overlay_for_daemon.show();
                        }
                        RecordingState::Idle => {
                            overlay_for_daemon.hide();
                        }
                        RecordingState::Typing => {
                            overlay_for_daemon.hide();
                        }
                    }
                }
                DaemonMsg::Error(e) => {
                    error!("Daemon error: {}", e);
                }
                _ => {}
            }
        }
        info!("Daemon channel closed, quitting application");
        app_for_quit.quit();
    });

    // Poll amplitude from PipeWire audio thread via mpsc (non-blocking)
    let overlay_for_amp = overlay.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(25), move || {
        // Drain all pending amplitude values into overlay history
        while let Ok(amp) = amp_rx.try_recv() {
            overlay_for_amp.set_amplitude(amp);
        }
        glib::ControlFlow::Continue
    });

    // Handle application shutdown
    let gui_tx_quit = gui_tx;
    app.connect_shutdown(move |_| {
        let _ = gui_tx_quit.try_send(GuiMsg::Quit);
    });

    // Leak the runtime so it lives for the process lifetime
    // (GTK Application::run takes control of the main loop)
    std::mem::forget(rt);

    Ok(())
}
