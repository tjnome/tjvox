use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use tracing::{debug, info, warn};

use crate::error::TjvoxError;

/// Evdev key codes for modifier keys.
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTALT: u16 = 100; // AltGr

/// Linux input event types.
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;

/// Key event values.
const KEY_PRESS: i32 = 1;
const KEY_RELEASE: i32 = 0;

/// XKB keycodes are evdev keycodes + 8.
const XKB_KEYCODE_OFFSET: u32 = 8;

/// A character mapped to its evdev keycode and required modifiers.
#[derive(Debug, Clone, Copy)]
struct KeyCombo {
    keycode: u16,
    shift: bool,
    altgr: bool,
}

/// Raw Linux input_event struct (64-bit).
/// Matches the kernel's struct input_event layout for ydotoold socket protocol.
#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

/// Keyboard layout-aware virtual keyboard that communicates directly with ydotoold
/// via its Unix socket, using xkbcommon for correct character-to-keycode mapping.
///
/// Unlike `ydotool type` which assumes a US keyboard layout, this sends the correct
/// keycodes for the user's actual layout (e.g. Norwegian).
pub struct VirtualKeyboard {
    char_map: HashMap<char, KeyCombo>,
    socket_path: String,
}

impl VirtualKeyboard {
    /// Create a new VirtualKeyboard. Detects the keyboard layout, builds a
    /// character map using xkbcommon, and verifies the ydotoold socket exists.
    pub fn new() -> Result<Self> {
        let uid = unsafe { libc::getuid() };
        let socket_path = format!("/run/user/{}/.ydotool_socket", uid);

        if !std::path::Path::new(&socket_path).exists() {
            return Err(TjvoxError::Output(format!(
                "ydotoold socket not found at {}. Is ydotoold running?",
                socket_path
            ))
            .into());
        }

        let char_map = build_char_map()?;
        info!(
            "VirtualKeyboard: {} character mappings loaded",
            char_map.len()
        );

        Ok(Self {
            char_map,
            socket_path,
        })
    }

    /// Type a string by sending keycode events for each character.
    /// Opens a fresh socket connection for each call.
    pub fn type_text(&self, text: &str) -> Result<()> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| {
            TjvoxError::Output(format!("Failed to connect to ydotoold: {}", e))
        })?;

        for ch in text.chars() {
            if let Some(combo) = self.char_map.get(&ch) {
                self.send_key_combo(&mut stream, combo)?;
            } else {
                debug!("No mapping for '{}' (U+{:04X}), skipping", ch, ch as u32);
            }
        }

        Ok(())
    }

    /// Send a full key press/release sequence including any required modifiers.
    fn send_key_combo(&self, stream: &mut UnixStream, combo: &KeyCombo) -> Result<()> {
        // Press modifiers
        if combo.shift {
            write_event(stream, EV_KEY, KEY_LEFTSHIFT, KEY_PRESS)?;
        }
        if combo.altgr {
            write_event(stream, EV_KEY, KEY_RIGHTALT, KEY_PRESS)?;
        }

        // Press key
        write_event(stream, EV_KEY, combo.keycode, KEY_PRESS)?;
        write_event(stream, EV_SYN, 0, 0)?;

        // Release key
        write_event(stream, EV_KEY, combo.keycode, KEY_RELEASE)?;

        // Release modifiers (reverse order)
        if combo.altgr {
            write_event(stream, EV_KEY, KEY_RIGHTALT, KEY_RELEASE)?;
        }
        if combo.shift {
            write_event(stream, EV_KEY, KEY_LEFTSHIFT, KEY_RELEASE)?;
        }

        write_event(stream, EV_SYN, 0, 0)?;
        Ok(())
    }
}

/// Write a single input_event to the ydotoold socket.
fn write_event(stream: &mut UnixStream, type_: u16, code: u16, value: i32) -> Result<()> {
    // Zero-initialize to avoid leaking stack data through padding bytes
    let mut event: InputEvent = unsafe { std::mem::zeroed() };
    event.tv_sec = 0;
    event.tv_usec = 0;
    event.type_ = type_;
    event.code = code;
    event.value = value;

    let bytes = unsafe {
        std::slice::from_raw_parts(
            &event as *const InputEvent as *const u8,
            std::mem::size_of::<InputEvent>(),
        )
    };
    stream.write_all(bytes).map_err(|e| {
        TjvoxError::Output(format!("Failed to write to ydotoold socket: {}", e))
    })?;
    Ok(())
}

/// Build a HashMap<char, KeyCombo> by loading the system XKB keymap and iterating
/// all keycodes at each shift level.
fn build_char_map() -> Result<HashMap<char, KeyCombo>> {
    use xkbcommon::xkb;

    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

    let (layout, variant) = detect_keyboard_layout();
    info!("XKB layout: '{}', variant: '{}'", layout, variant);

    let rules = String::new();
    let model = String::new();

    let keymap = xkb::Keymap::new_from_names(
        &ctx,
        &rules,    // rules — empty = system default ("evdev")
        &model,    // model — empty = system default
        &layout,
        &variant,
        None,      // options
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| {
        TjvoxError::Output(format!(
            "Failed to compile XKB keymap for layout='{}' variant='{}'",
            layout, variant
        ))
    })?;

    let mut map = HashMap::new();

    let min_kc = keymap.min_keycode();
    let max_kc = keymap.max_keycode();

    // Level mapping: 0=base, 1=Shift, 2=AltGr, 3=Shift+AltGr
    let levels: [(u32, bool, bool); 4] = [
        (0, false, false),
        (1, true, false),
        (2, false, true),
        (3, true, true),
    ];

    // Iterate all keycodes in the keymap
    let min_raw = min_kc.raw();
    let max_raw = max_kc.raw();

    for xkb_kc_raw in min_raw..=max_raw {
        let xkb_kc = xkb::Keycode::new(xkb_kc_raw);

        // evdev keycode = XKB keycode - 8
        if xkb_kc_raw < XKB_KEYCODE_OFFSET {
            continue;
        }
        let evdev_kc = (xkb_kc_raw - XKB_KEYCODE_OFFSET) as u16;

        for &(level, shift, altgr) in &levels {
            let syms = keymap.key_get_syms_by_level(xkb_kc, 0, level);
            for sym in syms {
                let cp = xkb::keysym_to_utf32(*sym);
                if cp == 0 || cp > 0x10FFFF {
                    continue;
                }
                if let Some(ch) = char::from_u32(cp) {
                    // Prefer the simplest combo (lowest level) — don't overwrite
                    map.entry(ch).or_insert(KeyCombo {
                        keycode: evdev_kc,
                        shift,
                        altgr,
                    });
                }
            }
        }
    }

    // Explicit mappings for control characters commonly in transcribed text
    // KEY_ENTER = 28, KEY_TAB = 15, KEY_SPACE = 57
    map.entry('\n').or_insert(KeyCombo {
        keycode: 28,
        shift: false,
        altgr: false,
    });
    map.entry('\t').or_insert(KeyCombo {
        keycode: 15,
        shift: false,
        altgr: false,
    });

    debug!("Built char map with {} entries", map.len());
    Ok(map)
}

/// Detect the keyboard layout from KDE's kxkbrc config, falling back to
/// environment variables, then system defaults.
fn detect_keyboard_layout() -> (String, String) {
    // Try KDE config first
    if let Some(config_dir) = dirs::config_dir() {
        let kxkbrc = config_dir.join("kxkbrc");
        if let Ok(content) = std::fs::read_to_string(&kxkbrc) {
            let mut layout = String::new();
            let mut variant = String::new();

            for line in content.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("LayoutList=") {
                    layout = val.split(',').next().unwrap_or("").to_string();
                }
                if let Some(val) = line.strip_prefix("VariantList=") {
                    variant = val.split(',').next().unwrap_or("").to_string();
                }
            }

            if !layout.is_empty() {
                debug!(
                    "Detected keyboard layout from kxkbrc: layout='{}' variant='{}'",
                    layout, variant
                );
                return (layout, variant);
            }
        }
    }

    // Try XKB environment variables
    let layout = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
    let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
    if !layout.is_empty() {
        debug!(
            "Detected keyboard layout from env: layout='{}' variant='{}'",
            layout, variant
        );
        return (layout, variant);
    }

    // Fall back to system defaults (xkbcommon will use XKB_DEFAULT_* or compiled-in defaults)
    warn!("Could not detect keyboard layout, using system default");
    (String::new(), String::new())
}
