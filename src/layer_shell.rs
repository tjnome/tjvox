use libloading::Library;
use std::ffi::c_void;
use tracing::{debug, info, warn};

// gtk4-layer-shell layer constants
const LAYER_OVERLAY: u32 = 3;

// gtk4-layer-shell edge constants
const EDGE_LEFT: u32 = 0;
const EDGE_RIGHT: u32 = 1;
const EDGE_TOP: u32 = 2;
const EDGE_BOTTOM: u32 = 3;

type InitForWindowFn = unsafe extern "C" fn(*mut c_void);
type SetLayerFn = unsafe extern "C" fn(*mut c_void, u32);
type SetAnchorFn = unsafe extern "C" fn(*mut c_void, u32, i32);
type SetExclusiveZoneFn = unsafe extern "C" fn(*mut c_void, i32);
type SetNamespaceFn = unsafe extern "C" fn(*mut c_void, *const std::ffi::c_char);

pub struct LayerShellFns {
    _lib: Library,
    init_for_window: InitForWindowFn,
    set_layer: SetLayerFn,
    set_anchor: SetAnchorFn,
    set_exclusive_zone: SetExclusiveZoneFn,
    set_namespace: SetNamespaceFn,
}

impl LayerShellFns {
    pub fn load() -> Option<Self> {
        let lib = match unsafe { Library::new("libgtk4-layer-shell.so.0") } {
            Ok(lib) => lib,
            Err(e) => {
                debug!("gtk4-layer-shell not available: {}", e);
                return None;
            }
        };

        unsafe {
            let init_for_window = *lib.get::<InitForWindowFn>(b"gtk_layer_init_for_window\0").ok()?;
            let set_layer = *lib.get::<SetLayerFn>(b"gtk_layer_set_layer\0").ok()?;
            let set_anchor = *lib.get::<SetAnchorFn>(b"gtk_layer_set_anchor\0").ok()?;
            let set_exclusive_zone = *lib.get::<SetExclusiveZoneFn>(b"gtk_layer_set_exclusive_zone\0").ok()?;
            let set_namespace = *lib.get::<SetNamespaceFn>(b"gtk_layer_set_namespace\0").ok()?;

            info!("gtk4-layer-shell loaded successfully");
            Some(Self {
                _lib: lib,
                init_for_window,
                set_layer,
                set_anchor,
                set_exclusive_zone,
                set_namespace,
            })
        }
    }

    pub fn apply_to_window(&self, window: &gtk4::Window, position: &str) {
        use gtk4::prelude::*;

        let ptr = window.as_ptr() as *mut c_void;

        unsafe {
            (self.init_for_window)(ptr);
            (self.set_layer)(ptr, LAYER_OVERLAY);
            (self.set_exclusive_zone)(ptr, -1);

            let namespace = b"tjvox-overlay\0";
            (self.set_namespace)(ptr, namespace.as_ptr() as *const std::ffi::c_char);

            // Reset all anchors first
            (self.set_anchor)(ptr, EDGE_LEFT, 0);
            (self.set_anchor)(ptr, EDGE_RIGHT, 0);
            (self.set_anchor)(ptr, EDGE_TOP, 0);
            (self.set_anchor)(ptr, EDGE_BOTTOM, 0);

            match position {
                "bottom-center" => {
                    (self.set_anchor)(ptr, EDGE_BOTTOM, 1);
                    (self.set_anchor)(ptr, EDGE_LEFT, 1);
                    (self.set_anchor)(ptr, EDGE_RIGHT, 1);
                }
                "top-center" => {
                    (self.set_anchor)(ptr, EDGE_TOP, 1);
                    (self.set_anchor)(ptr, EDGE_LEFT, 1);
                    (self.set_anchor)(ptr, EDGE_RIGHT, 1);
                }
                "bottom-left" => {
                    (self.set_anchor)(ptr, EDGE_BOTTOM, 1);
                    (self.set_anchor)(ptr, EDGE_LEFT, 1);
                }
                "bottom-right" => {
                    (self.set_anchor)(ptr, EDGE_BOTTOM, 1);
                    (self.set_anchor)(ptr, EDGE_RIGHT, 1);
                }
                "top-left" => {
                    (self.set_anchor)(ptr, EDGE_TOP, 1);
                    (self.set_anchor)(ptr, EDGE_LEFT, 1);
                }
                "top-right" => {
                    (self.set_anchor)(ptr, EDGE_TOP, 1);
                    (self.set_anchor)(ptr, EDGE_RIGHT, 1);
                }
                "center" => {
                    // No anchors — centered
                }
                other => {
                    warn!("Unknown overlay position '{}', using no anchors", other);
                }
            }
        }

        info!("Layer shell applied with position '{}'", position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_shell_load_returns_none_without_lib() {
        // On most CI/dev environments, libgtk4-layer-shell is not installed
        // This test verifies graceful fallback
        let result = LayerShellFns::load();
        // We can't assert None because it might be installed — just verify no panic
        let _ = result;
    }

    #[test]
    fn test_layer_constants() {
        assert_eq!(LAYER_OVERLAY, 3);
        assert_eq!(EDGE_LEFT, 0);
        assert_eq!(EDGE_RIGHT, 1);
        assert_eq!(EDGE_TOP, 2);
        assert_eq!(EDGE_BOTTOM, 3);
    }
}
