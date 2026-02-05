#[cfg(feature = "ptt")]
pub mod monitor {
    use anyhow::{Context, Result};
    use evdev::{Device, InputEventKind, Key};
    use tokio::sync::mpsc;
    use tracing::{debug, info, warn};

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum PttEvent {
        KeyDown,
        KeyUp,
    }

    pub struct PttMonitor {
        key: Key,
    }

    impl PttMonitor {
        pub fn new(key_name: &str) -> Result<Self> {
            let key = parse_key_name(key_name)
                .with_context(|| format!("Unknown key name: {}", key_name))?;
            info!("PTT monitor configured for key: {:?}", key);
            Ok(Self { key })
        }

        pub async fn run(self, tx: mpsc::Sender<PttEvent>) -> Result<()> {
            let key = self.key;

            // Scan for input devices with the target key
            let devices = evdev::enumerate()
                .filter_map(|(_, device)| {
                    if device
                        .supported_keys()
                        .map_or(false, |keys| keys.contains(key))
                    {
                        Some(device)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            if devices.is_empty() {
                anyhow::bail!(
                    "No input device found with key {:?}. Check permissions on /dev/input/event*.",
                    key
                );
            }

            info!(
                "Monitoring {} device(s) for key {:?}",
                devices.len(),
                key
            );

            // Monitor all matching devices concurrently
            let mut handles = Vec::new();
            for device in devices {
                let tx = tx.clone();
                let handle = tokio::task::spawn_blocking(move || {
                    monitor_device(device, key, tx)
                });
                handles.push(handle);
            }

            // Wait for any to finish (they shouldn't under normal operation)
            for handle in handles {
                if let Err(e) = handle.await {
                    warn!("PTT monitor task failed: {}", e);
                }
            }

            Ok(())
        }
    }

    fn monitor_device(
        mut device: Device,
        key: Key,
        tx: mpsc::Sender<PttEvent>,
    ) -> Result<()> {
        let name = device
            .name()
            .unwrap_or("unknown")
            .to_string();
        info!("Monitoring device: {}", name);

        loop {
            let events = device
                .fetch_events()
                .context("Failed to fetch events from device")?;

            for event in events {
                if let InputEventKind::Key(k) = event.kind() {
                    if k == key {
                        let ptt_event = match event.value() {
                            1 => Some(PttEvent::KeyDown),
                            0 => Some(PttEvent::KeyUp),
                            _ => None, // repeat events ignored
                        };

                        if let Some(evt) = ptt_event {
                            debug!("PTT event: {:?} from {}", evt, name);
                            if tx.blocking_send(evt).is_err() {
                                return Ok(()); // channel closed
                            }
                        }
                    }
                }
            }
        }
    }

    fn parse_key_name(name: &str) -> Option<Key> {
        // Support common key names like "KEY_F13", "KEY_SCROLLLOCK", etc.
        let name = name.to_uppercase();
        let name = if name.starts_with("KEY_") {
            name
        } else {
            format!("KEY_{}", name)
        };

        // Use evdev's key constants - match common ones explicitly
        match name.as_str() {
            "KEY_F1" => Some(Key::KEY_F1),
            "KEY_F2" => Some(Key::KEY_F2),
            "KEY_F3" => Some(Key::KEY_F3),
            "KEY_F4" => Some(Key::KEY_F4),
            "KEY_F5" => Some(Key::KEY_F5),
            "KEY_F6" => Some(Key::KEY_F6),
            "KEY_F7" => Some(Key::KEY_F7),
            "KEY_F8" => Some(Key::KEY_F8),
            "KEY_F9" => Some(Key::KEY_F9),
            "KEY_F10" => Some(Key::KEY_F10),
            "KEY_F11" => Some(Key::KEY_F11),
            "KEY_F12" => Some(Key::KEY_F12),
            "KEY_F13" => Some(Key::KEY_F13),
            "KEY_F14" => Some(Key::KEY_F14),
            "KEY_F15" => Some(Key::KEY_F15),
            "KEY_F16" => Some(Key::KEY_F16),
            "KEY_F17" => Some(Key::KEY_F17),
            "KEY_F18" => Some(Key::KEY_F18),
            "KEY_F19" => Some(Key::KEY_F19),
            "KEY_F20" => Some(Key::KEY_F20),
            "KEY_F21" => Some(Key::KEY_F21),
            "KEY_F22" => Some(Key::KEY_F22),
            "KEY_F23" => Some(Key::KEY_F23),
            "KEY_F24" => Some(Key::KEY_F24),
            "KEY_SCROLLLOCK" => Some(Key::KEY_SCROLLLOCK),
            "KEY_PAUSE" => Some(Key::KEY_PAUSE),
            "KEY_INSERT" => Some(Key::KEY_INSERT),
            "KEY_PRINT" => Some(Key::KEY_PRINT),
            "KEY_RIGHTALT" => Some(Key::KEY_RIGHTALT),
            "KEY_RIGHTCTRL" => Some(Key::KEY_RIGHTCTRL),
            "KEY_LEFTMETA" => Some(Key::KEY_LEFTMETA),
            "KEY_RIGHTMETA" => Some(Key::KEY_RIGHTMETA),
            "KEY_CAPSLOCK" => Some(Key::KEY_CAPSLOCK),
            _ => None,
        }
    }
}
