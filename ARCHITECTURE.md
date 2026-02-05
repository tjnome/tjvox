# Dictation - Architecture

## Overview

Native Rust voice dictation for Linux with in-process Whisper transcription, GTK4 overlay, and KDE system tray. Inspired by VoiceInk's macOS architecture, adapted for Linux/Wayland.

## Component Diagram

```
┌──────────────────────────────────────────────────────────────────┐
│ MAIN THREAD (GTK4)              BACKGROUND (tokio runtime)       │
│ ─────────────────               ─────────────────────────        │
│ gtk4::Application                Daemon::run() select! loop      │
│   ├─ OverlayWindow                ├─ SIGUSR1 signal              │
│   │   ├─ DrawingArea (Cairo)       ├─ Unix socket IPC            │
│   │   ├─ Waveform bars            │   └─ toggle/push-start/stop │
│   │   └─ Processing dots          ├─ PTT evdev monitor (opt)    │
│   └─ glib::spawn_future_local     ├─ GuiMsg channel             │
│       ├─ recv DaemonMsg            ├─ AudioRecorder              │
│       └─ update overlay            │   └─ PipeWire native        │
│                                    ├─ TranscriptionService       │
│ PIPEWIRE AUDIO THREAD             │   ├─ WhisperContext (CUDA)   │
│ ─────────────────                  │   └─ Hot/Cold lifecycle     │
│ F32LE stream callback              ├─ ReplacementEngine          │
│   └─ RMS per 50ms                  ├─ HistoryStore (SQLite)      │
│       └─ mpsc → GTK                ├─ OutputManager              │
│                                    │   └─ wl-copy + ydotool     │
│ CLI CLIENTS                        └─ ksni Tray (async)          │
│ ─────────────                                                     │
│ dictation toggle ──→ socket (pref) or SIGUSR1 (fallback)         │
│ dictation push-start/push-stop ──→ socket                        │
│ dictation history ──→ direct SQLite read                         │
│                                                                   │
│ CHANNELS: DaemonMsg (daemon→GTK), GuiMsg (GTK→daemon)            │
│           f32 amplitude (PipeWire→GTK), PttEvent (evdev→daemon)  │
└──────────────────────────────────────────────────────────────────┘
```

## Source Files

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI entry point. Sync `fn main()` creates tokio runtimes manually. GUI subcommand is default when `gui` feature enabled. Includes history and push-to-talk subcommands. |
| `src/lib.rs` | Module declarations. GUI modules gated behind `#[cfg(feature = "gui")]`. |
| `src/config.rs` | TOML configuration: AudioConfig, TranscriptionConfig, WhisperConfig (hot/cold), OutputConfig (paste/type), OverlayConfig, UiConfig, ReplacementsConfig, HistoryConfig, InputConfig. |
| `src/daemon.rs` | State machine (Idle→Recording→Transcribing→Typing→Idle). Holds persistent TranscriptionService. Accepts optional GUI channels for bidirectional communication. Integrates socket IPC, replacements, history, and optional PTT. |
| `src/audio.rs` | Audio recording via native PipeWire (pipewire-rs). F32LE stream capture with real-time RMS amplitude computation. UUID-named WAV files via hound. |
| `src/transcription.rs` | whisper-rs integration. Auto-downloads GGML models from HuggingFace. Reads WAV with hound, converts to f32 mono 16kHz. Supports prewarm for hot mode. CUDA GPU acceleration available via `cuda` feature. |
| `src/output.rs` | Text output. Auto-detects terminal windows (via KWin D-Bus) and uses Ctrl+Shift+V. Paste method: save clipboard → wl-copy text → ydotool Ctrl+V → restore clipboard after 2s. Fallback: wtype. Type method: VirtualKeyboard via xkbcommon + ydotoold socket for layout-aware typing. |
| `src/ui.rs` | Desktop notifications with fallback chain: notify-send → kdialog → zenity → console. |
| `src/error.rs` | Error variants: Config, Audio, Transcription, Output, Ui, Daemon, ModelDownload, ModelLoad, Timeout. |
| `src/messages.rs` | Channel message types: DaemonMsg (StateChanged, Amplitude, WhisperModeChanged, ModelChanged, ModelLoading, Error), GuiMsg (ToggleRecording, SetWhisperMode, SetModel, Quit), RecordingState enum. |
| `src/replacements.rs` | Word/phrase replacement engine. Loads rules from TOML, compiles case-insensitive whole-word regex patterns, applies sequentially. |
| `src/history.rs` | SQLite-backed transcription history. WAL mode for concurrent CLI access. Configurable retention (max_entries). |
| `src/socket.rs` | Unix socket IPC server/client. Newline-delimited text protocol (toggle, push-start, push-stop, status, quit). Socket at `/run/user/{uid}/dictation.sock`. |
| `src/ptt.rs` | Push-to-talk evdev key monitoring (behind `ptt` feature). Scans `/dev/input/event*` for keyboards with target key, monitors press/release. |
| `src/input.rs` | VirtualKeyboard implementation for layout-aware typing. Uses xkbcommon to build character-to-keycode maps from system keyboard layout. Communicates with ydotoold via Unix socket. |
| `src/gui/mod.rs` | GTK4 Application setup. Creates tokio runtime on background thread. Wires async_channels between GTK main thread and tokio daemon/tray. |
| `src/gui/overlay.rs` | 280x50 rounded capsule overlay. Cairo drawing: 21 waveform bars (VoiceInk amplitude boosting `pow(amp, 0.5)`) during recording, 5 pulsing dots during transcription. 20 FPS redraw. |
| `src/gui/tray.rs` | ksni StatusNotifierItem tray. KDE-native icons per state. Menu: toggle recording, hot/cold mode radio, quit. |

## Key Design Patterns

### Parallel Model Loading

When the user starts recording, the model begins loading in the background simultaneously. By the time recording finishes, the model is already loaded:

```
Toggle ON
  ├─ pw-record starts immediately
  └─ TranscriptionService::load_model() fires in parallel

Toggle OFF
  ├─ pw-record stops
  └─ transcribe() finds model already loaded → fast transcription
```

### Hot/Cold Mode

- **Hot**: `WhisperContext` persists in `TranscriptionService` across recordings. Pre-warmed on daemon startup with a 1-second silence transcription.
- **Cold**: `WhisperContext` created per transcription (parallel with recording), dropped after. Lower memory footprint.

### GTK4 + Tokio Bridge

GTK4 requires owning the main thread. Tokio runs on a background thread. `async_channel` provides bidirectional communication:

- `DaemonMsg`: daemon → GTK (state changes, amplitude, errors)
- `GuiMsg`: GTK/tray → daemon (toggle, mode switch, quit)
- `f32`: PipeWire → GTK (amplitude for waveform)

### Clipboard Save/Restore

Output uses VoiceInk's clipboard pattern:
1. Save current clipboard via native wl-clipboard-rs (wlroots protocol) or `wl-paste` fallback
2. Set clipboard to transcribed text via native wl-clipboard-rs or `wl-copy` fallback
3. For terminals: Simulate Ctrl+Shift+V via `ydotool` (29:1,42:1,47:1 sequence)
4. For GUI apps: Simulate Ctrl+V via `ydotool` (preferred) or `wtype` (fallback)
5. Restore original clipboard after configurable delay (default 2s, background task)

## Configuration

Stored at `~/.config/dictation/config.toml`. Auto-created with defaults if missing.

```toml
[audio]
sample_rate = 16000
channels = 1
format = "wav"
temp_dir = "/tmp/dictation"

[transcription]
model = "base"
language = "en"
# threads = 4
# remove_filler_words = false

[whisper]
mode = "cold"

[output]
delay_ms = 100
paste_delay_ms = 50
append_trailing_space = true
method = "paste"

[ui]
show_notifications = true
notification_timeout_ms = 3000

[overlay]
enabled = true
width = 280
height = 50
position = "bottom-center"
opacity = 0.85

[replacements]
enabled = true
file = "~/.config/dictation/replacements.toml"

[history]
enabled = true
max_entries = 1000

[input]
# ptt_key = "KEY_F13"  # requires ptt feature
```

### Word Replacements

Configurable word/phrase replacements applied after transcription, before filler word removal. Rules loaded from a separate TOML file (`~/.config/dictation/replacements.toml`). Each rule compiles to a case-insensitive whole-word regex (`(?i)\b{key}\b`), applied sequentially.

Default rules convert spoken punctuation ("period" → ".", "comma" → ",", etc.) and formatting ("new line" → "\n", "new paragraph" → "\n\n").

### Transcription History

SQLite-backed persistence of all transcriptions. WAL journal mode enables concurrent read access from CLI while the daemon writes. Each entry stores timestamp, recording duration, transcribed text, model name, and language.

Retention is automatic: after each save, entries beyond `max_entries` are pruned (oldest first). CLI commands `dictation history` and `dictation history-clear` read the database directly without requiring the daemon.

Database location: `~/.local/share/dictation/history.db`

### Push-to-Talk / Socket IPC

Unix socket at `/run/user/{uid}/dictation.sock` provides IPC alongside the existing SIGUSR1 signal mechanism. Protocol is newline-delimited text commands: `toggle`, `push-start`, `push-stop`, `status`, `quit`.

Push-to-talk is idempotent: `push-start` only starts recording if Idle, `push-stop` only stops if Recording. The `toggle` CLI command prefers socket, falling back to SIGUSR1 if the socket is unavailable.

Optional evdev key monitoring (behind the `ptt` feature flag) scans `/dev/input/event*` for keyboards with a configured key and sends KeyDown/KeyUp events via mpsc channel to the daemon select! loop.

### CUDA GPU Acceleration

The `cuda` feature flag passes through to `whisper-rs/cuda`, which sets `-DGGML_CUDA=1` during whisper.cpp cmake build. `WhisperContextParameters::default()` auto-detects and uses the GPU at runtime. No Rust code changes required — only the build environment needs CUDA toolkit (nvcc, libcublas, libcudart).

## Feature Flags

```toml
[features]
default = ["gui"]
gui = ["dep:gtk4", "dep:cairo-rs", "dep:async-channel", "dep:ksni"]
cuda = ["whisper-rs/cuda"]
ptt = ["dep:evdev"]
```

## Build

```bash
# Build dependencies (Fedora)
sudo dnf install cmake clang-devel gcc-c++ pipewire-devel gtk4-devel

# If Python cmake shim conflicts:
export CMAKE=/usr/bin/cmake

# Full build (GUI + headless)
cargo build --release

# Headless only
cargo build --release --no-default-features

# With CUDA GPU acceleration (requires cuda-toolkit)
cargo build --release --features cuda

# With push-to-talk evdev monitoring
cargo build --release --features ptt
```

## Runtime Dependencies

- PipeWire (native audio capture)
- `wl-copy`, `wl-paste` (clipboard, fallback)
- `ydotool` (keystroke simulation on KDE Plasma Wayland)
- `notify-send` or `kdialog` or `zenity` (notifications)
- D-Bus session bus (system tray)
- NVIDIA driver (for CUDA feature, host-level)

## Deferred: GTK4 Layer Shell

GTK4 layer shell (`gtk4-layer-shell` crate) is deferred for the following reasons:

- Requires installing `gtk4-layer-shell` system library which cannot be bundled
- Current overlay with `set_focusable(false)` + `set_decorated(false)` works adequately on KDE Plasma
- The `gtk4-layer-shell = "0.5"` crate is compatible with current `gtk4 = "0.9"`; version 0.7+ requires gtk4 0.10
- Could revisit with dynamic loading (`libloading` + dlopen) to make it optional at runtime
- Primary benefit would be on wlroots-based compositors (Sway, Hyprland) where layer shell provides proper overlay stacking

## Future Enhancements

- AI enhancement via LLM post-processing
- Per-app configuration (different prompts/languages per active window)
- Dynamic `libloading` for optional gtk4-layer-shell support
