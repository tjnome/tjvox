use anyhow::Result;
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::config::UiConfig;
use crate::error::TjvoxError;

#[derive(Clone)]
pub struct UiManager {
    config: UiConfig,
    has_notify_send: bool,
    has_kdialog: bool,
    has_zenity: bool,
}

impl UiManager {
    pub fn new() -> Result<Self> {
        let has_notify_send = which::which("notify-send").is_ok();
        let has_kdialog = which::which("kdialog").is_ok();
        let has_zenity = which::which("zenity").is_ok();

        debug!(
            "UI capabilities: notify-send={}, kdialog={}, zenity={}",
            has_notify_send, has_kdialog, has_zenity
        );

        Ok(Self {
            config: UiConfig::default(),
            has_notify_send,
            has_kdialog,
            has_zenity,
        })
    }

    pub fn with_config(config: &UiConfig) -> Self {
        let has_notify_send = which::which("notify-send").is_ok();
        let has_kdialog = which::which("kdialog").is_ok();
        let has_zenity = which::which("zenity").is_ok();

        Self {
            config: config.clone(),
            has_notify_send,
            has_kdialog,
            has_zenity,
        }
    }

    pub async fn show_notification(&self, title: &str, message: &str) -> Result<()> {
        if !self.config.show_notifications {
            info!("[Notification] {}: {}", title, message);
            return Ok(());
        }

        if self.has_notify_send {
            self.show_libnotify(title, message).await
        } else if self.has_kdialog {
            self.show_kdialog(title, message).await
        } else if self.has_zenity {
            self.show_zenity(title, message).await
        } else {
            // Fallback to console
            info!("[Notification] {}: {}", title, message);
            Ok(())
        }
    }

    async fn show_libnotify(&self, title: &str, message: &str) -> Result<()> {
        let timeout_ms = self.config.notification_timeout_ms;

        let output = Command::new("notify-send")
            .args(&[
                "--expire-time", &timeout_ms.to_string(),
                title,
                message,
            ])
            .output()
            .await
            .map_err(|e| TjvoxError::Ui(format!("Failed to show notification: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("notify-send warning: {}", stderr);
        }

        Ok(())
    }

    async fn show_kdialog(&self, title: &str, message: &str) -> Result<()> {
        let timeout_secs = (self.config.notification_timeout_ms / 1000).max(1);
        let _ = Command::new("kdialog")
            .args(&[
                "--passivepopup",
                message,
                &timeout_secs.to_string(),
                "--title",
                title,
            ])
            .spawn()?;

        Ok(())
    }

    async fn show_zenity(&self, title: &str, message: &str) -> Result<()> {
        let _ = Command::new("zenity")
            .args(&[
                "--info",
                "--title", title,
                "--text", message,
                "--timeout", &((self.config.notification_timeout_ms / 1000).to_string()),
            ])
            .spawn()?;

        Ok(())
    }

    pub async fn show_error(&self, title: &str, error: &str) -> Result<()> {
        tracing::error!("UI Error - {}: {}", title, error);

        if self.has_notify_send {
            let _ = Command::new("notify-send")
                .args(&[
                    "--urgency", "critical",
                    title,
                    error,
                ])
                .output()
                .await;
        }

        Ok(())
    }
}

impl Default for UiManager {
    fn default() -> Self {
        Self::with_config(&UiConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_manager_new() {
        let manager = UiManager::new();
        assert!(manager.is_ok());
    }

    #[test]
    fn test_ui_manager_default() {
        let manager = UiManager::default();
        assert!(!manager.config.show_notifications);
    }

    #[test]
    fn test_ui_manager_with_config() {
        let config = UiConfig {
            show_notifications: true,
            notification_timeout_ms: 5000,
        };
        let manager = UiManager::with_config(&config);
        assert!(manager.config.show_notifications);
        assert_eq!(manager.config.notification_timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_notification_disabled() {
        let config = UiConfig {
            show_notifications: false,
            notification_timeout_ms: 3000,
        };
        let manager = UiManager::with_config(&config);
        // Should return Ok without spawning any process
        let result = manager.show_notification("Test", "Message").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_show_error() {
        let config = UiConfig {
            show_notifications: false,
            notification_timeout_ms: 3000,
        };
        let manager = UiManager::with_config(&config);
        let result = manager.show_error("Error", "Something went wrong").await;
        assert!(result.is_ok());
    }
}
