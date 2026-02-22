#!/bin/bash
# Build and install tjvox locally

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"

CUDA_MODE="auto"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cuda)
            CUDA_MODE="on"
            shift
            ;;
        --cpu)
            CUDA_MODE="off"
            shift
            ;;
        --auto-cuda)
            CUDA_MODE="auto"
            shift
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./install.sh [--cuda | --cpu | --auto-cuda]

Options:
  --cuda        Force CUDA build (NVIDIA toolkit required)
  --cpu         Force CPU build (disable CUDA)
  --auto-cuda   Auto-detect CUDA toolkit and use it when available (default)
  -h, --help  Show this help
EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run ./install.sh --help for usage."
            exit 1
            ;;
    esac
done

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found in PATH. Install Rust toolchain first."
    exit 1
fi

# Prefer system cmake when a broken Python shim is first in PATH
if command -v cmake >/dev/null 2>&1; then
    if ! cmake --version >/dev/null 2>&1; then
        if [[ -x /usr/bin/cmake ]]; then
            export CMAKE=/usr/bin/cmake
            echo "Detected broken cmake shim in PATH; using /usr/bin/cmake"
        else
            echo "cmake in PATH is broken and /usr/bin/cmake is unavailable"
            echo "Install a working cmake package and retry."
            exit 1
        fi
    fi
else
    echo "cmake not found in PATH. Install cmake first."
    exit 1
fi

if command -v pkg-config >/dev/null 2>&1; then
    missing_pc=()
    for pc in libseccomp lcms2; do
        if ! pkg-config --exists "$pc"; then
            missing_pc+=("$pc")
        fi
    done

    if [[ ${#missing_pc[@]} -gt 0 ]]; then
        echo "Missing required development libraries: ${missing_pc[*]}"
        echo "Install build deps from README before running install.sh."
        echo "Ubuntu/Debian hint: sudo apt install libseccomp-dev liblcms2-dev"
        exit 1
    fi
fi

MAKE_BIN=""
if command -v make >/dev/null 2>&1; then
    MAKE_BIN="$(command -v make)"
elif command -v gmake >/dev/null 2>&1; then
    MAKE_BIN="$(command -v gmake)"
else
    echo "No make implementation found (need make or gmake)."
    echo "Install build essentials/base-devel and retry."
    exit 1
fi
export CMAKE_MAKE_PROGRAM="$MAKE_BIN"

echo "Building tjvox binary..."
echo "Using target dir: $CARGO_TARGET_DIR"
BUILD_ARGS=(--release --manifest-path "$SCRIPT_DIR/Cargo.toml")
ENABLE_CUDA=false
if [[ "$CUDA_MODE" == "on" ]]; then
    ENABLE_CUDA=true
elif [[ "$CUDA_MODE" == "auto" ]]; then
    if command -v nvcc >/dev/null 2>&1; then
        ENABLE_CUDA=true
        echo "Detected CUDA toolkit (nvcc). Enabling CUDA build."
    else
        echo "No CUDA toolkit detected (nvcc not found). Using CPU build."
    fi
fi

if [[ "$ENABLE_CUDA" == true ]]; then
    echo "CUDA build enabled (--features cuda)"
    BUILD_ARGS+=(--features cuda)
fi

cargo build "${BUILD_ARGS[@]}"

mkdir -p "$HOME/.local/bin"
cp "$CARGO_TARGET_DIR/release/tjvox" "$HOME/.local/bin/tjvox"
chmod +x "$HOME/.local/bin/tjvox"

mkdir -p "$HOME/.config/tjvox"
if [[ ! -f "$HOME/.config/tjvox/config.toml" ]]; then
    if [[ -f "$SCRIPT_DIR/config/config.example.toml" ]]; then
        echo "Installing default config from config/config.example.toml"
        cp "$SCRIPT_DIR/config/config.example.toml" "$HOME/.config/tjvox/config.toml"
    else
        echo "No config example found; tjvox will create defaults on first run."
    fi
fi

echo "Checking runtime tools..."
missing=()

if ! command -v wl-copy >/dev/null 2>&1; then
    missing+=("wl-clipboard")
fi

if ! command -v ydotool >/dev/null 2>&1 && ! command -v wtype >/dev/null 2>&1; then
    missing+=("ydotool or wtype")
fi

if [[ ${#missing[@]} -gt 0 ]]; then
    echo "WARNING: Missing runtime dependency(s): ${missing[*]}"
    echo "Install packages with your distro package manager (dnf/apt/pacman)."
fi

echo
echo "Installed: $HOME/.local/bin/tjvox"
echo "Config:    $HOME/.config/tjvox/config.toml"
echo
echo "Try: tjvox --help"
