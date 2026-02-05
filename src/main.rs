use std::path::PathBuf;
use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

use tjvox::config::Config;
use tjvox::daemon::Daemon;
use tjvox::history::HistoryStore;
use tjvox::socket;
use tjvox::ui::UiManager;
use tjvox::audio::AudioRecorder;
use tjvox::transcription::TranscriptionService;
use tjvox::output::OutputManager;

#[derive(Parser)]
#[command(name = "tjvox")]
#[command(about = "macOS-style voice dictation for Linux with GPU acceleration")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a single dictation session
    Run,
    /// Start background daemon (headless)
    Daemon,
    /// Start GUI with overlay and system tray
    #[cfg(feature = "gui")]
    Gui,
    /// Toggle recording (send SIGUSR1 to daemon)
    Toggle,
    /// Stop background daemon
    Stop,
    /// Check daemon status
    Status,
    /// Show transcription history
    History {
        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Clear all transcription history
    HistoryClear,
    /// Start push-to-talk recording (via socket)
    PushStart,
    /// Stop push-to-talk recording (via socket)
    PushStop,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let config_path = cli.config
        .or_else(|| {
            dirs::config_dir()
                .map(|d| d.join("tjvox/config.toml"))
        })
        .ok_or_else(|| anyhow::anyhow!("Could not determine config path"))?;

    match cli.command {
        Some(Commands::Toggle) => {
            toggle_daemon()?;
        }
        Some(Commands::Stop) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                info!("Stopping daemon");
                stop_daemon().await
            })?;
        }
        Some(Commands::Status) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(check_status())?;
        }
        #[cfg(feature = "gui")]
        Some(Commands::Gui) | None => {
            let rt = tokio::runtime::Runtime::new()?;
            let config = rt.block_on(Config::load(&config_path))?;
            drop(rt);
            info!("Starting TJvox GUI");
            tjvox::gui::run_gui(config)?;
        }
        #[cfg(not(feature = "gui"))]
        None => {
            let rt = tokio::runtime::Runtime::new()?;
            let config = rt.block_on(Config::load(&config_path))?;
            info!("Starting TJvox daemon");
            rt.block_on(async {
                let daemon = Daemon::new(config).await?;
                daemon.run().await
            })?;
        }
        Some(Commands::Daemon) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let config = Config::load(&config_path).await?;
                info!("Starting TJvox daemon");
                let daemon = Daemon::new(config).await?;
                daemon.run().await
            })?;
        }
        Some(Commands::Run) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let config = Config::load(&config_path).await?;
                info!("Running single TJvox session");
                run_single_session(config).await
            })?;
        }
        Some(Commands::History { limit }) => {
            show_history(limit)?;
        }
        Some(Commands::HistoryClear) => {
            clear_history()?;
        }
        Some(Commands::PushStart) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let response = socket::send_command("push-start").await?;
                println!("{}", response);
                Ok::<(), anyhow::Error>(())
            })?;
        }
        Some(Commands::PushStop) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let response = socket::send_command("push-stop").await?;
                println!("{}", response);
                Ok::<(), anyhow::Error>(())
            })?;
        }
    }

    Ok(())
}

async fn run_single_session(config: Config) -> Result<()> {
    let ui = UiManager::with_config(&config.ui);

    let mut recorder = AudioRecorder::new(&config.audio, None)?;
    let recording_path = recorder.start().await?;

    println!("Recording to: {}", recording_path.display());
    println!("Press Enter to stop recording...");
    ui.show_notification("TJvox", "Recording... Press Enter to stop").await?;

    // Wait for Enter key to stop
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;

    println!("Stopping recording...");
    let audio_path = recorder.stop().await?;

    println!("Transcribing...");
    ui.show_notification("TJvox", "Transcribing...").await?;

    let mut transcriber = TranscriptionService::new(&config.transcription)?;
    let text = transcriber.transcribe(&audio_path).await?;

    if text.trim().is_empty() {
        println!("No speech detected.");
        ui.show_notification("TJvox", "No speech detected").await?;
    } else {
        println!("Transcribed: {}", text);
        let output = OutputManager::new(&config.output)?;
        output.type_text(&text).await?;
        ui.show_notification("TJvox", &format!("Transcribed: {}", &text[..text.len().min(50)])).await?;
    }

    // Cleanup
    let _ = tokio::fs::remove_file(&audio_path).await;

    Ok(())
}

/// Get the PID file path in the user's runtime directory (secure, user-private).
fn pid_file_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/tjvox.pid", uid))
}

/// Check if a process with the given PID is actually a dictation process.
fn is_tjvox_process(pid: i32) -> bool {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
        cmdline.contains("tjvox")
    } else {
        false
    }
}

fn toggle_daemon() -> Result<()> {
    // Try socket first, fall back to SIGUSR1
    let rt = tokio::runtime::Runtime::new()?;
    match rt.block_on(socket::send_command("toggle")) {
        Ok(response) => {
            println!("{}", response);
            return Ok(());
        }
        Err(_) => {
            info!("Socket not available, falling back to SIGUSR1");
        }
    }

    let pid_file = pid_file_path();
    if !pid_file.exists() {
        anyhow::bail!("Daemon is not running (no PID file)");
    }

    let pid_str = std::fs::read_to_string(&pid_file)?;
    let pid: i32 = pid_str.trim().parse()?;

    if !is_tjvox_process(pid) {
        // Stale PID file — clean it up
        std::fs::remove_file(&pid_file).ok();
        anyhow::bail!("Stale PID file (PID {} is not a tjvox process)", pid);
    }

    let ret = unsafe { libc::kill(pid, libc::SIGUSR1) };
    if ret != 0 {
        anyhow::bail!("Failed to send SIGUSR1 to PID {}", pid);
    }

    println!("Sent toggle signal to daemon (PID {})", pid);
    Ok(())
}

async fn stop_daemon() -> Result<()> {
    let pid_file = pid_file_path();
    if !pid_file.exists() {
        println!("Daemon is not running");
        return Ok(());
    }

    let pid = tokio::fs::read_to_string(&pid_file).await?;
    let pid: i32 = pid.trim().parse()?;

    if !is_tjvox_process(pid) {
        tokio::fs::remove_file(&pid_file).await.ok();
        println!("Removed stale PID file (PID {} is not a tjvox process)", pid);
        return Ok(());
    }

    let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        anyhow::bail!("Failed to send SIGTERM to PID {}: {}", pid, err);
    }

    tokio::fs::remove_file(&pid_file).await.ok();
    println!("Daemon stopped");
    Ok(())
}

async fn check_status() -> Result<()> {
    let pid_file = pid_file_path();
    if pid_file.exists() {
        let pid_str = tokio::fs::read_to_string(&pid_file).await?;
        let pid: i32 = pid_str.trim().parse()?;
        // Verify process is actually alive and is dictation
        if is_tjvox_process(pid) {
            println!("Daemon is running (PID: {})", pid);
        } else {
            // Clean up stale PID file
            tokio::fs::remove_file(&pid_file).await.ok();
            println!("Daemon is not running (cleaned up stale PID file)");
        }
    } else {
        println!("Daemon is not running");
    }
    Ok(())
}

fn data_dir_fallback() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| {
        // Fall back to $HOME/.local/share instead of unexpandable tilde
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".local/share"))
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
    })
}

fn show_history(limit: u32) -> Result<()> {
    let db_path = data_dir_fallback().join("tjvox/history.db");

    if !db_path.exists() {
        println!("No history found.");
        return Ok(());
    }

    let store = HistoryStore::open(&db_path, 1000)?;
    let entries = store.list(limit)?;

    if entries.is_empty() {
        println!("No transcription history.");
        return Ok(());
    }

    for entry in &entries {
        let duration = if entry.duration_ms > 0 {
            format!("{:.1}s", entry.duration_ms as f64 / 1000.0)
        } else {
            "?".to_string()
        };
        println!(
            "[{}] ({}, {}, {}) {}",
            entry.timestamp, duration, entry.model, entry.language, entry.text
        );
    }

    println!("\n{} entries shown.", entries.len());
    Ok(())
}

fn clear_history() -> Result<()> {
    let db_path = data_dir_fallback().join("tjvox/history.db");

    if !db_path.exists() {
        println!("No history to clear.");
        return Ok(());
    }

    let store = HistoryStore::open(&db_path, 1000)?;
    store.clear()?;
    println!("History cleared.");
    Ok(())
}
