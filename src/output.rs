use anyhow::Result;
use std::io::Read;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::config::OutputConfig;
use crate::error::TjvoxError;

/// Known terminal emulator resource class patterns (lowercase).
/// Matched as substrings against the active window's resourceClass.
const TERMINAL_PATTERNS: &[&str] = &[
    "ghostty",
    "konsole",
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "xterm",
    "gnome-terminal",
    "tilix",
    "terminator",
    "sakura",
    "urxvt",
    "rxvt",
    "st-256color",
    "mate-terminal",
    "lxterminal",
    "xfce4-terminal",
    "cool-retro-term",
    "yakuake",
    "guake",
    "terminology",
    "contour",
    "rio",
];

pub struct OutputManager {
    config: OutputConfig,
}

impl OutputManager {
    pub fn new(config: &OutputConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
        })
    }

    pub async fn type_text(&self, text: &str) -> Result<()> {
        info!("Outputting {} characters", text.len());

        // Small delay before output
        sleep(Duration::from_millis(self.config.delay_ms)).await;

        match self.config.method.as_str() {
            "auto" => self.auto_output(text).await?,
            "paste" => self.paste_text(text).await?,
            "type" => self.type_with_ydotool(text).await?,
            "clipboard" => self.clipboard_only(text).await?,
            other => {
                warn!("Unknown output method '{}', falling back to auto", other);
                self.auto_output(text).await?;
            }
        }

        info!("Text output successfully");
        Ok(())
    }

    /// Smart output: detect active window type and choose the best method.
    /// Terminals get clipboard + Ctrl+Shift+V (terminal paste shortcut).
    /// GUI apps get clipboard + Ctrl+V (standard paste).
    async fn auto_output(&self, text: &str) -> Result<()> {
        let is_terminal = detect_terminal_focused().await;

        if is_terminal {
            info!("Terminal detected, using clipboard + Ctrl+Shift+V");
            self.paste_text_terminal(text).await
        } else {
            debug!("GUI window detected, using clipboard + Ctrl+V");
            self.paste_text(text).await
        }
    }

    /// Set clipboard then simulate Ctrl+Shift+V paste (terminal paste shortcut).
    /// Works in Ghostty, Konsole, Kitty, Alacritty, WezTerm, and most terminals.
    async fn paste_text_terminal(&self, text: &str) -> Result<()> {
        // Save current clipboard content
        let saved_clipboard = get_clipboard().await.ok();

        // Set clipboard to transcribed text
        set_clipboard(text).await?;

        // Brief delay to let clipboard settle
        sleep(Duration::from_millis(self.config.paste_delay_ms)).await;

        // Simulate Ctrl+Shift+V via ydotool
        // 29 = KEY_LEFTCTRL, 42 = KEY_LEFTSHIFT, 47 = KEY_V
        self.ensure_ydotoold().await?;
        let output = Command::new("ydotool")
            .args(["key", "29:1", "42:1", "47:1", "47:0", "42:0", "29:0"])
            .output()
            .await
            .map_err(|e| TjvoxError::Output(format!("ydotool key failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(
                TjvoxError::Output(format!("ydotool Ctrl+Shift+V failed: {}", stderr))
                    .into(),
            );
        }

        // Restore original clipboard after a delay
        if let Some(original) = saved_clipboard {
            let delay_ms = self.config.paste_delay_ms.max(2000);
            tokio::spawn(async move {
                sleep(Duration::from_millis(delay_ms)).await;
                if let Err(e) = set_clipboard(&original).await {
                    debug!("Failed to restore clipboard: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Set clipboard then simulate Ctrl+V paste.
    async fn paste_text(&self, text: &str) -> Result<()> {
        // Save current clipboard content
        let saved_clipboard = get_clipboard().await.ok();

        // Set clipboard to transcribed text
        set_clipboard(text).await?;

        // Brief delay to let clipboard settle
        sleep(Duration::from_millis(self.config.paste_delay_ms)).await;

        // Simulate Ctrl+V — try ydotool first (works on KDE Plasma Wayland),
        // fall back to wtype
        if let Err(e) = self.send_paste_keystroke_ydotool().await {
            debug!("ydotool paste failed ({}), trying wtype", e);
            self.send_paste_keystroke_wtype().await?;
        }

        // Restore original clipboard after a delay
        if let Some(original) = saved_clipboard {
            let delay_ms = self.config.paste_delay_ms.max(2000);
            tokio::spawn(async move {
                sleep(Duration::from_millis(delay_ms)).await;
                if let Err(e) = set_clipboard(&original).await {
                    debug!("Failed to restore clipboard: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Just set the clipboard, don't paste. User can Ctrl+V manually.
    async fn clipboard_only(&self, text: &str) -> Result<()> {
        set_clipboard(text).await?;
        info!("Text copied to clipboard (use Ctrl+V to paste)");
        Ok(())
    }

    /// Ctrl+V via ydotool (works on KDE Plasma Wayland)
    async fn send_paste_keystroke_ydotool(&self) -> Result<()> {
        if which::which("ydotool").is_err() {
            return Err(TjvoxError::Output("ydotool not found".to_string()).into());
        }

        self.ensure_ydotoold().await?;

        // ydotool key: 29 = KEY_LEFTCTRL, 47 = KEY_V
        let output = Command::new("ydotool")
            .args(["key", "29:1", "47:1", "47:0", "29:0"])
            .output()
            .await
            .map_err(|e| TjvoxError::Output(format!("ydotool key failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(
                TjvoxError::Output(format!("ydotool key failed: {}", stderr)).into(),
            );
        }

        Ok(())
    }

    /// Ctrl+V via wtype (needs virtual-keyboard-v1 protocol support)
    async fn send_paste_keystroke_wtype(&self) -> Result<()> {
        if which::which("wtype").is_err() {
            return Err(TjvoxError::Output(
                "Neither ydotool nor wtype available for paste keystroke".to_string(),
            )
            .into());
        }

        let output = Command::new("wtype")
            .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
            .output()
            .await
            .map_err(|e| TjvoxError::Output(format!("wtype failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(
                TjvoxError::Output(format!("wtype paste failed: {}", stderr)).into(),
            );
        }

        Ok(())
    }

    async fn ensure_ydotoold(&self) -> Result<()> {
        let check = Command::new("pgrep")
            .arg("ydotoold")
            .output()
            .await?;

        if !check.status.success() {
            warn!("ydotoold not running, attempting to start...");
            let _ = Command::new("ydotoold").spawn()?;
            sleep(Duration::from_millis(500)).await;
        }

        Ok(())
    }

    /// Type text using native VirtualKeyboard (xkbcommon + ydotoold socket).
    /// Respects the user's keyboard layout for correct character mapping.
    async fn type_with_ydotool(&self, text: &str) -> Result<()> {
        self.ensure_ydotoold().await?;

        let text = text.to_string();
        tokio::task::spawn_blocking(move || {
            let vk = crate::input::VirtualKeyboard::new()?;
            vk.type_text(&text)
        })
        .await?
    }
}

/// Detect if the currently focused window is a terminal emulator.
/// Uses KDE's KWin D-Bus API to query the active window's resourceClass.
/// Returns false if detection fails (safe default: use clipboard paste).
async fn detect_terminal_focused() -> bool {
    let output = Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "org.kde.KWin",
            "--object-path", "/KWin",
            "--method", "org.kde.KWin.queryWindowInfo",
        ])
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => {
            debug!("KWin D-Bus query failed, assuming GUI window");
            return false;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse resourceClass from the D-Bus response
    // Format: ..., 'resourceClass': <'com.mitchellh.ghostty'>, ...
    let class = extract_dbus_string(&stdout, "resourceClass")
        .unwrap_or_default()
        .to_lowercase();

    if class.is_empty() {
        return false;
    }

    let is_terminal = TERMINAL_PATTERNS.iter().any(|p| class.contains(p));
    debug!("Active window resourceClass='{}', terminal={}", class, is_terminal);
    is_terminal
}

/// Extract a string value from KWin's D-Bus variant map output.
/// Looks for pattern: 'key': <'value'>
fn extract_dbus_string(output: &str, key: &str) -> Option<String> {
    let pattern = format!("'{}': <'", key);
    let start = output.find(&pattern)? + pattern.len();
    let rest = &output[start..];
    let end = rest.find("'>")?;
    Some(rest[..end].to_string())
}

/// Get clipboard contents. Tries native wl-clipboard-rs first (wlroots protocol),
/// falls back to wl-paste (standard Wayland protocol via wl_data_device_manager).
async fn get_clipboard() -> Result<String> {
    // Try native Rust clipboard (wlroots data-control protocol)
    let native_result = tokio::task::spawn_blocking(|| {
        use wl_clipboard_rs::paste;
        let result = paste::get_contents(
            paste::ClipboardType::Regular,
            paste::Seat::Unspecified,
            paste::MimeType::Text,
        );
        match result {
            Ok((mut pipe, _)) => {
                let mut contents = String::new();
                pipe.read_to_string(&mut contents)?;
                Ok(contents)
            }
            Err(e) => Err(anyhow::anyhow!("{}", e)),
        }
    })
    .await?;

    if let Ok(text) = native_result {
        return Ok(text);
    }

    // Fallback: wl-paste (supports standard wl_data_device_manager on KDE etc.)
    debug!("Native clipboard read unavailable, using wl-paste");
    let output = Command::new("wl-paste")
        .arg("--no-newline")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("wl-paste failed: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(anyhow::anyhow!("wl-paste returned error"))
    }
}

/// Set clipboard contents. Tries native wl-clipboard-rs first (wlroots protocol),
/// falls back to wl-copy (standard Wayland protocol via wl_data_device_manager).
async fn set_clipboard(text: &str) -> Result<()> {
    // Try native Rust clipboard (wlroots data-control protocol)
    let text_for_native = text.to_string();
    let native_result = tokio::task::spawn_blocking(move || {
        use wl_clipboard_rs::copy::{MimeType, Options, Source};
        let opts = Options::new();
        opts.copy(
            Source::Bytes(text_for_native.into_bytes().into()),
            MimeType::Text,
        )
    })
    .await?;

    if native_result.is_ok() {
        return Ok(());
    }

    // Fallback: wl-copy (supports standard wl_data_device_manager on KDE etc.)
    debug!("Native clipboard write unavailable, using wl-copy");
    let mut child = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| TjvoxError::Output(format!("Failed to run wl-copy: {}", e)))?;

    if let Some(ref mut stdin) = child.stdin {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| {
                TjvoxError::Output(format!("Failed to write to wl-copy: {}", e))
            })?;
    }
    drop(child.stdin.take());

    let status = child.wait().await.map_err(|e| {
        TjvoxError::Output(format!("wl-copy failed: {}", e))
    })?;

    if !status.success() {
        return Err(
            TjvoxError::Output("wl-copy exited with error".to_string()).into(),
        );
    }

    Ok(())
}

/// Check if a window class matches any known terminal pattern.
#[cfg(test)]
fn is_terminal_class(class: &str) -> bool {
    let lower = class.to_lowercase();
    TERMINAL_PATTERNS.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_dbus_string_basic() {
        let output = "({'resourceClass': <'com.mitchellh.ghostty'>, 'caption': <'Terminal'>},)";
        let result = extract_dbus_string(output, "resourceClass");
        assert_eq!(result, Some("com.mitchellh.ghostty".to_string()));
    }

    #[test]
    fn test_extract_dbus_string_caption() {
        let output = "({'resourceClass': <'firefox'>, 'caption': <'Mozilla Firefox'>},)";
        let result = extract_dbus_string(output, "caption");
        assert_eq!(result, Some("Mozilla Firefox".to_string()));
    }

    #[test]
    fn test_extract_dbus_string_missing_key() {
        let output = "({'resourceClass': <'firefox'>},)";
        let result = extract_dbus_string(output, "nonexistent");
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_dbus_string_empty_value() {
        let output = "({'resourceClass': <''>},)";
        let result = extract_dbus_string(output, "resourceClass");
        assert_eq!(result, Some(String::new()));
    }

    #[test]
    fn test_extract_dbus_string_empty_input() {
        let result = extract_dbus_string("", "resourceClass");
        assert_eq!(result, None);
    }

    #[test]
    fn test_is_terminal_class_ghostty() {
        assert!(is_terminal_class("com.mitchellh.ghostty"));
    }

    #[test]
    fn test_is_terminal_class_konsole() {
        assert!(is_terminal_class("org.kde.konsole"));
    }

    #[test]
    fn test_is_terminal_class_alacritty() {
        assert!(is_terminal_class("Alacritty"));
    }

    #[test]
    fn test_is_terminal_class_firefox() {
        assert!(!is_terminal_class("firefox"));
    }

    #[test]
    fn test_is_terminal_class_empty() {
        assert!(!is_terminal_class(""));
    }

    #[test]
    fn test_is_terminal_class_kitty() {
        assert!(is_terminal_class("kitty"));
    }

    #[test]
    fn test_is_terminal_class_wezterm() {
        assert!(is_terminal_class("org.wezfurlong.wezterm"));
    }

    #[test]
    fn test_terminal_patterns_count() {
        // Ensure we have a reasonable number of patterns
        assert!(TERMINAL_PATTERNS.len() >= 20);
    }

    #[test]
    fn test_output_manager_new() {
        let config = OutputConfig {
            delay_ms: 100,
            paste_delay_ms: 50,
            append_trailing_space: true,
            method: "auto".to_string(),
        };
        let manager = OutputManager::new(&config);
        assert!(manager.is_ok());
    }
}
