#!/bin/bash
# Build and install the tjvox binary

set -e

echo "Building tjvox binary..."

# Enter dev container and build (needs cmake, clang, gcc-c++ for whisper.cpp)
distrobox enter dev -- bash -c "
    source ~/.cargo/env
    cd /var/home/tjnome/dev/dictation
    cargo build --release 2>&1
"

# Copy binary to host
mkdir -p ~/.local/bin
cp ~/dev/dictation/target/release/tjvox ~/.local/bin/
chmod +x ~/.local/bin/tjvox

# Create config directory
mkdir -p ~/.config/tjvox

# Create default config if it doesn't exist
if [[ ! -f ~/.config/tjvox/config.toml ]]; then
    echo "Creating default config..."
    cat > ~/.config/tjvox/config.toml << 'EOF'
[audio]
sample_rate = 16000
channels = 1
format = "wav"
temp_dir = "/tmp/tjvox"

[transcription]
model = "base"
language = "en"
# threads = 4  # auto-detect if omitted
# remove_filler_words = false

[whisper]
mode = "cold"  # "hot" keeps model in memory, "cold" loads per transcription

[output]
delay_ms = 100
paste_delay_ms = 50
append_trailing_space = true
method = "paste"  # "paste" (wl-copy + wtype) or "type" (ydotool)

[ui]
show_notifications = true
notification_timeout_ms = 3000

[overlay]
enabled = true
width = 280
height = 50
position = "bottom-center"
opacity = 0.85
EOF
fi

# Check for Wayland clipboard tools
echo "Checking dependencies..."
missing=""

if ! command -v wl-copy &> /dev/null; then
    missing="$missing wl-clipboard"
fi

if ! command -v wtype &> /dev/null; then
    missing="$missing wtype"
fi

if [[ -n "$missing" ]]; then
    echo "WARNING: Missing packages for paste mode:$missing"
    echo "Install with: rpm-ostree install$missing --apply-live --idempotent"
fi

echo ""
echo "TJvox binary installed to ~/.local/bin/tjvox"
echo ""
echo "Usage:"
echo "   tjvox gui       # Start with overlay and system tray (default)"
echo "   tjvox run       # Run single session (press Enter to stop)"
echo "   tjvox daemon    # Start headless background daemon"
echo "   tjvox toggle    # Toggle recording (send SIGUSR1 to daemon)"
echo "   tjvox status    # Check daemon status"
echo "   tjvox stop      # Stop daemon"
echo ""
echo "Config: ~/.config/tjvox/config.toml"
echo ""
echo "First run will auto-download the whisper model (~150MB for 'base')."
echo ""
echo "Test it:"
echo "   1. Open a text editor"
echo "   2. Run: tjvox run"
echo "   3. Speak, then press Enter to stop"
echo "   4. Watch your text appear!"
