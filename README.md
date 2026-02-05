# Dictation - macOS-Style Voice Dictation for Linux

Native Rust voice dictation with in-process Whisper transcription, a VoiceInk-style waveform overlay, and KDE system tray integration. No containers, no external APIs.

## Features

- **Native Whisper** - In-process transcription via whisper.cpp (no distrobox/containers)
- **Auto Model Download** - Downloads GGML models from HuggingFace on first run
- **Hot/Cold Mode** - Keep model in memory (hot) or load per transcription (cold)
- **Waveform Overlay** - GTK4 floating capsule with live mic waveform during recording
- **System Tray** - KDE-native tray icon with state display, mode toggle, and model selection
- **Clipboard Paste** - Native Wayland clipboard with wl-copy fallback + ydotool/wtype keystroke
- **Toggle Mode** - Press once to start, again to stop
- **Push-to-Talk** - Optional evdev key monitoring for push-to-talk mode (ptt feature)
- **Smart Terminal Detection** - Auto-detects terminal windows and uses Ctrl+Shift+V
- **Word Replacements** - Configurable text substitutions (e.g., "period" → ".")
- **Transcription History** - SQLite-backed history with configurable retention
- **Configurable** - TOML config with model, language, threads, overlay settings

## Quick Start

### 1. Install Build Dependencies

In your dev distrobox or Fedora system:

```bash
sudo dnf install cmake clang-devel gcc-c++ pipewire-devel gtk4-devel
```

### 2. Build

```bash
cd ~/dev/dictation

# If ~/.local/bin/cmake conflicts (Python shim), set:
export CMAKE=/usr/bin/cmake

# Build with GUI (default)
cargo build --release

# Or build headless (no GTK/tray)
cargo build --release --no-default-features
```

### 3. Install

```bash
mkdir -p ~/.local/bin
cp target/release/dictation ~/.local/bin/

# Or use the install script:
./install.sh
```

### 4. Install Runtime Dependencies

For the default paste output method:

```bash
# Fedora
sudo dnf install wl-clipboard ydotool

# Start the ydotool daemon
sudo systemctl enable --now ydotool
```

### 5. Run

```bash
# Start with GUI overlay + system tray (default)
dictation

# Or explicitly:
dictation gui

# Single session (press Enter to stop)
dictation run

# Headless daemon
dictation daemon
```

The first run will auto-download the Whisper model (~150MB for `base`) to `~/.local/share/dictation/models/`.

## Usage

```
dictation [COMMAND]

Commands:
  gui           Start GUI with overlay and system tray (default)
  run           Run a single dictation session
  daemon        Start headless background daemon
  toggle        Toggle recording (via socket or SIGUSR1)
  push-start    Start push-to-talk recording (via socket)
  push-stop     Stop push-to-talk recording (via socket)
  status        Check daemon status
  stop          Stop background daemon
  history       Show transcription history
  history-clear Clear all transcription history
```

### Typical Workflow

1. Run `dictation gui` (or just `dictation`)
2. A system tray icon appears
3. Use tray menu or `dictation toggle` to start recording
4. The waveform overlay appears showing live mic input
5. Toggle again to stop - overlay shows processing dots
6. Transcribed text is pasted into the active window
7. Overlay hides, ready for next dictation

### Keyboard Shortcut

Bind `dictation toggle` to a global shortcut in KDE Settings > Shortcuts.

## Configuration

All settings in `~/.config/dictation/config.toml`:

```toml
[audio]
sample_rate = 16000       # 16kHz optimal for Whisper
channels = 1              # Mono
format = "wav"
temp_dir = "/tmp/dictation"

[transcription]
model = "base"            # tiny, base, small, medium, large-v3-turbo
language = "en"           # Language code or omit for auto-detect
# threads = 4             # Auto-detect if omitted (max(1, min(8, cpus-2)))
# remove_filler_words = false  # Remove "uh", "um", "like", etc.

[whisper]
mode = "cold"             # "hot" = model stays in memory, "cold" = load per use

[output]
delay_ms = 100            # Delay before output
paste_delay_ms = 50       # Delay between clipboard set and Ctrl+V
append_trailing_space = true
method = "auto"           # "auto" (smart detect), "paste", "type", "clipboard"

[ui]
show_notifications = true
notification_timeout_ms = 3000

[overlay]
enabled = true
width = 280
height = 50
position = "bottom-center"
opacity = 0.85
```

## Available Models

Models are auto-downloaded to `~/.local/share/dictation/models/` on first use.

| Model | Config Value | Size | Speed | Quality |
|-------|-------------|------|-------|---------|
| Tiny | `tiny` | ~75MB | Fastest | Basic |
| Base | `base` | ~150MB | Fast | Good |
| Small | `small` | ~500MB | Medium | Better |
| Medium | `medium` | ~1.5GB | Slow | Great |
| Large v3 Turbo | `large-v3-turbo` | ~3GB | Slowest | Best |

## Hot vs Cold Mode

- **Cold** (default): Model loads when you start recording (parallel with recording) and unloads after transcription. Lower memory usage.
- **Hot**: Model stays loaded in memory after first use. Pre-warmed on startup. Faster transcriptions but uses more RAM.

Toggle via system tray menu or config file.

## Architecture

```
dictation/
├── Cargo.toml
├── install.sh
├── src/
│   ├── main.rs            # CLI entry, sync main with tokio runtimes
│   ├── lib.rs             # Module exports
│   ├── config.rs          # TOML config (audio, transcription, whisper, output, overlay)
│   ├── daemon.rs          # Background daemon with state machine
│   ├── audio.rs           # PipeWire native audio capture (pipewire-rs)
│   ├── transcription.rs   # whisper-rs transcription, model download, hot/cold
│   ├── output.rs          # Clipboard (wl-clipboard-rs → wl-copy fallback) + ydotool/wtype
│   ├── input.rs           # Virtual keyboard for layout-aware typing (xkbcommon + ydotoold)
│   ├── ui.rs              # Desktop notifications (notify-send/kdialog/zenity)
│   ├── error.rs           # Error types
│   ├── messages.rs        # DaemonMsg/GuiMsg enums for GTK<->daemon bridge
│   ├── socket.rs          # Unix socket IPC server/client
│   ├── history.rs         # SQLite-backed transcription history
│   ├── replacements.rs    # Word/phrase replacement engine
│   ├── ptt.rs             # Push-to-talk evdev key monitoring (ptt feature)
│   └── gui/
│       ├── mod.rs          # GTK4 app setup, tokio runtime, channel wiring
│       ├── overlay.rs      # Cairo waveform capsule + processing dots
│       └── tray.rs         # ksni system tray with state/mode/model controls
└── config/
```

### Data Flow

```
User triggers toggle
  ├── Recording starts (PipeWire native capture)
  ├── Model loads in parallel (if cold)
  ├── Audio amplitude feeds waveform overlay directly from capture stream
  └── Overlay shows with live waveform bars

User triggers toggle again
  ├── Recording stops
  ├── Overlay switches to processing dots
  ├── Whisper transcribes audio in-process
  ├── Post-processing (filler removal, trailing space)
  ├── Text pasted via wl-copy → Ctrl+V
  ├── Original clipboard restored after 2s
  └── Overlay hides
```

## Build Features

| Feature | Default | Description |
|---------|---------|-------------|
| `gui` | Yes | GTK4 overlay, Cairo waveform, ksni tray |
| `cuda` | No | GPU acceleration via CUDA (requires NVIDIA toolkit) |
| `ptt` | No | Push-to-talk evdev key monitoring |

```bash
# Build with GUI (default)
cargo build --release

# Build headless only
cargo build --release --no-default-features

# Build with CUDA support (requires cuda-toolkit)
cargo build --release --features cuda

# Build with push-to-talk support
cargo build --release --features ptt

# Build with all optional features
cargo build --release --features "cuda,ptt"
```

## System Dependencies

**Build time:**
- cmake, clang-devel, gcc-c++ (whisper.cpp compilation)
- pipewire-devel (PipeWire audio capture)
- gtk4-devel (GUI overlay)

**Runtime:**
- PipeWire (audio capture, standard on modern Linux)
- wl-clipboard (clipboard operations)
- ydotool (paste keystroke simulation)
- wtype (alternative paste keystroke - some compositors)
- notify-send, kdialog, or zenity (notifications, optional)
- D-Bus (system tray via StatusNotifierItem)

**Optional for ptt feature:**
- Read access to `/dev/input/event*` (for evdev key monitoring)

**Optional for cuda feature:**
- NVIDIA driver and CUDA toolkit

## License

MIT
