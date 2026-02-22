# TJvox - macOS-Style Voice Dictation for Linux

Native Rust voice dictation with local Whisper transcription, a GTK overlay, and system tray controls.

## Platform Support

- Primary target: **Linux Wayland** sessions (KDE Plasma tested)
- X11 support: **not officially supported**
- Why: output and window integration depend on Wayland tools/protocols (`wl-clipboard`, `ydotool`/`wtype`, KWin D-Bus detection)

If you run under X11, basic pieces may still work, but paste behavior, focus detection, and typing reliability are not guaranteed.

## Features

- In-process transcription via `whisper.cpp` (no external API)
- Auto model download on first run
- Hot/Cold whisper lifecycle modes
- GTK4 waveform overlay during recording
- System tray controls (start/stop, mode, model)
- Wayland-first output (`wl-clipboard`, `ydotool`, `wtype` fallback)
- Toggle and push-to-talk recording modes
- Smart terminal detection for Ctrl+Shift+V paste
- Configurable models, language, output mode, and replacements
- SQLite transcription history with retention
- Optional features: `ptt` and `cuda`

## Quick Start

### 1. Install build dependencies

Fedora:

```bash
sudo dnf install cmake clang-devel gcc-c++ pipewire-devel gtk4-devel openssl-devel git pkgconf-pkg-config
```

Ubuntu:

```bash
sudo apt install cmake libclang-dev g++ libpipewire-0.3-dev libgtk-4-dev libssl-dev libseccomp-dev liblcms2-dev git pkg-config
```

Debian (Trixie/Forky):

```bash
sudo apt install cmake libclang-dev g++ libpipewire-0.3-dev libgtk-4-dev libssl-dev libseccomp-dev liblcms2-dev git pkg-config
```

Arch:

```bash
sudo pacman -Syu --needed base-devel cmake clang pipewire gtk4 openssl git pkgconf
```

### 2. Build and install

```bash
cd /path/to/tjvox

# If a Python cmake shim causes issues
export CMAKE=/usr/bin/cmake

cargo build --release
mkdir -p ~/.local/bin
cp target/release/tjvox ~/.local/bin/
```

Or use:

```bash
./install.sh
```

`install.sh` build modes:

```bash
# Auto-detect CUDA toolkit (default)
./install.sh

# Force CPU-only build
./install.sh --cpu

# Force CUDA build (NVIDIA toolkit required)
./install.sh --cuda
```

## CPU vs CUDA Builds

- Default build is CPU and works on Intel/AMD/NVIDIA systems.
- CUDA is optional and only for NVIDIA systems with CUDA toolkit (`nvcc`) installed.
- If CUDA is unavailable, use CPU build (`cargo build --release` or `./install.sh --cpu`).

### 3. Install runtime dependencies

Fedora:

```bash
sudo dnf install wl-clipboard ydotool wtype
sudo systemctl enable --now ydotool
```

Ubuntu/Debian:

```bash
sudo apt install wl-clipboard ydotool wtype
```

These are required for text output on Wayland. If missing, `install.sh` will warn and dictation output may not be inserted into focused apps.

Arch:

```bash
sudo pacman -S --needed wl-clipboard ydotool wtype
```

Note: on some Debian-based systems, `ydotoold` may need to be started manually if no unit file is shipped.

### 4. Run

```bash
# GUI + tray (default)
tjvox

# Single session
tjvox run

# Background daemon
tjvox daemon
```

First run downloads the selected Whisper model to `~/.local/share/tjvox/models/`.

## Usage

```bash
macOS-style voice dictation for Linux with GPU acceleration

Usage: tjvox [OPTIONS] [COMMAND]

Commands:
  run            Run a single dictation session
  daemon         Start background daemon (headless)
  gui            Start GUI with overlay and system tray
  toggle         Toggle recording (send SIGUSR1 to daemon)
  stop           Stop background daemon
  status         Check daemon status
  history        Show transcription history
  history-clear  Clear all transcription history
  push-start     Start push-to-talk recording (via socket)
  push-stop      Stop push-to-talk recording (via socket)
  help           Print this message or the help of the given subcommand(s)

Options:
  -c, --config <FILE>
  -h, --help           Print help
  -V, --version        Print version
```

Set a global shortcut to `tjvox toggle` in your desktop settings.

## Configuration

Config file: `~/.config/tjvox/config.toml`

Example config is available at `config/config.example.toml`.

```bash
mkdir -p ~/.config/tjvox
cp config/config.example.toml ~/.config/tjvox/config.toml
```

Common settings:

- `transcription.model` (`tiny`, `base`, `small`, `medium`, `large-v3-turbo`)
- `transcription.language` (for example `en`; unset for auto)
- `whisper.mode` (`cold` or `hot`)
- `output.method` (`auto`, `paste`, `type`, `clipboard`)
- `overlay.enabled` (`true`/`false`)

## Whisper Models

Models are downloaded on first use to `~/.local/share/tjvox/models/`.

| Model | Value | Size | Speed | Quality |
|---|---|---|---|---|
| Tiny | `tiny` | ~75MB | Fastest | Basic |
| Base | `base` | ~150MB | Fast | Good |
| Small | `small` | ~500MB | Medium | Better |
| Medium | `medium` | ~1.5GB | Slow | Great |
| Large v3 Turbo | `large-v3-turbo` | ~3GB | Slowest | Best |

## Build Features

| Feature | Default | Description |
|---|---|---|
| `gui` | Yes | GTK overlay and tray |
| `cuda` | No | CUDA acceleration |
| `ptt` | No | Push-to-talk via evdev |

Examples:

```bash
cargo build --release
cargo build --release --no-default-features
cargo build --release --features cuda
cargo build --release --features ptt
```

## License

MIT
