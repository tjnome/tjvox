# TJvox Architecture

## Overview

TJvox is a local Linux dictation app written in Rust.

- Audio capture: PipeWire
- Transcription: `whisper-rs` / `whisper.cpp`
- UI: GTK4 overlay + tray (when `gui` feature is enabled)
- IPC/control: Unix socket + CLI commands
- Persistence: SQLite history

## Runtime Model

Two execution sides work together:

1. **GUI/main thread**
   - Runs GTK application loop
   - Draws recording overlay and tray state
   - Sends user actions to daemon (toggle, mode/model changes)

2. **Daemon/background runtime**
   - Owns recording/transcription state machine
   - Starts/stops audio capture
   - Runs transcription and text output
   - Handles socket commands and optional push-to-talk events

## Core Flow

1. User triggers `toggle` (tray or CLI).
2. Daemon enters recording state and captures audio.
3. Recording stops on next toggle (or push-stop).
4. Audio is transcribed by Whisper.
5. Replacements/history/output pipeline runs.
6. Text is pasted/typed into the active app.

## Main Modules

| Path | Responsibility |
|---|---|
| `src/main.rs` | CLI entry and command dispatch |
| `src/daemon.rs` | Main state machine and orchestration |
| `src/audio.rs` | PipeWire recording and WAV creation |
| `src/transcription.rs` | Model handling + Whisper transcription |
| `src/output.rs` | Clipboard/type output strategy |
| `src/socket.rs` | Local Unix socket IPC |
| `src/config.rs` | TOML configuration loading/defaults |
| `src/history.rs` | SQLite transcription history |
| `src/replacements.rs` | Post-transcription text substitutions |
| `src/messages.rs` | Message types between GUI and daemon |
| `src/gui/overlay.rs` | Recording overlay rendering |
| `src/gui/tray.rs` | Tray menu/state integration |

## IPC and Control

- CLI commands (for example `tjvox toggle`, `tjvox status`) communicate with the daemon over a local Unix socket.
- GUI and daemon communicate with channels for state updates and user actions.

## Configuration and Data

- Config: `~/.config/tjvox/config.toml`
- Models: `~/.local/share/tjvox/models/`
- History DB: `~/.local/share/tjvox/history.db`

## Build Features

| Feature | Purpose |
|---|---|
| `gui` (default) | GTK overlay + tray UI |
| `cuda` | GPU acceleration through Whisper CUDA backend |
| `ptt` | Push-to-talk input monitoring |
