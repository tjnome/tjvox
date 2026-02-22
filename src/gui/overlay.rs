use gtk4::prelude::*;
use gtk4::{self, glib};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config::OverlayConfig;
use crate::layer_shell::LayerShellFns;
use crate::messages::RecordingState;

const NUM_BARS: usize = 21;
const AMPLITUDE_HISTORY_SIZE: usize = 64;

#[derive(Clone)]
pub struct OverlayWindow {
    window: gtk4::Window,
    drawing_area: gtk4::DrawingArea,
    state: Arc<Mutex<RecordingState>>,
    amplitude_history: Arc<Mutex<VecDeque<f32>>>,
    bar_levels: Arc<Mutex<[f32; NUM_BARS]>>,
    start_time: Arc<Mutex<Instant>>,
}

impl OverlayWindow {
    pub fn new(
        app: &gtk4::Application,
        config: &OverlayConfig,
        layer_shell: Option<&LayerShellFns>,
    ) -> Self {
        // Transparent window background via CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_data(
            "window.tjvox-overlay { background-color: transparent; } \
             window.tjvox-overlay > * { background-color: transparent; }",
        );
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let window = gtk4::Window::builder()
            .application(app)
            .title("TJvox")
            .default_width(config.width)
            .default_height(config.height)
            .decorated(false)
            .resizable(false)
            .build();

        window.add_css_class("tjvox-overlay");
        window.set_focusable(false);

        if let Some(ls) = layer_shell {
            ls.apply_to_window(&window, &config.position);
        }

        let drawing_area = gtk4::DrawingArea::new();
        drawing_area.set_content_width(config.width);
        drawing_area.set_content_height(config.height);

        window.set_child(Some(&drawing_area));

        let state = Arc::new(Mutex::new(RecordingState::Idle));
        let amplitude_history =
            Arc::new(Mutex::new(VecDeque::with_capacity(AMPLITUDE_HISTORY_SIZE)));
        let bar_levels = Arc::new(Mutex::new([0.0f32; NUM_BARS]));
        let start_time = Arc::new(Mutex::new(Instant::now()));
        let opacity = config.opacity;

        // Set up Cairo drawing
        let state_draw = state.clone();
        let bar_levels_draw = bar_levels.clone();
        let start_time_draw = start_time.clone();

        drawing_area.set_draw_func(move |_area, cr, w, h| {
            // Safely get values from shared state, using defaults if mutex is poisoned
            let current_state = state_draw
                .lock()
                .map(|g| *g)
                .unwrap_or(RecordingState::Idle);
            let bars = bar_levels_draw
                .lock()
                .map(|g| *g)
                .unwrap_or([0.0; NUM_BARS]);
            let elapsed = start_time_draw
                .lock()
                .map(|g| g.elapsed().as_secs_f64())
                .unwrap_or(0.0);

            draw_overlay(
                cr,
                w as f64,
                h as f64,
                opacity,
                current_state,
                &bars,
                elapsed,
            );
        });

        // 20 FPS update timer — also smooths bar levels from amplitude history
        let da_clone = drawing_area.clone();
        let hist_update = amplitude_history.clone();
        let bars_update = bar_levels.clone();
        let state_update = state.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Ok(state) = state_update.lock() {
                if *state == RecordingState::Recording {
                    update_bar_levels(&hist_update, &bars_update);
                }
            }
            da_clone.queue_draw();
            glib::ControlFlow::Continue
        });

        // Start hidden
        window.set_visible(false);

        Self {
            window,
            drawing_area,
            state,
            amplitude_history,
            bar_levels,
            start_time,
        }
    }

    pub fn show(&self) {
        if let Ok(mut time) = self.start_time.lock() {
            *time = Instant::now();
        }
        self.window.set_visible(true);
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn set_state(&self, state: RecordingState) {
        if let Ok(mut s) = self.state.lock() {
            *s = state;
        }
        if state == RecordingState::Recording {
            if let Ok(mut history) = self.amplitude_history.lock() {
                history.clear();
            }
            if let Ok(mut bars) = self.bar_levels.lock() {
                *bars = [0.0; NUM_BARS];
            }
        }
        if state == RecordingState::Recording || state == RecordingState::Transcribing {
            if let Ok(mut time) = self.start_time.lock() {
                *time = Instant::now();
            }
        }
        self.drawing_area.queue_draw();
    }

    pub fn set_amplitude(&self, amp: f32) {
        if let Ok(mut history) = self.amplitude_history.lock() {
            history.push_back(amp);
            while history.len() > AMPLITUDE_HISTORY_SIZE {
                history.pop_front();
            }
        }
    }
}

fn update_bar_levels(
    history: &Arc<Mutex<VecDeque<f32>>>,
    bar_levels: &Arc<Mutex<[f32; NUM_BARS]>>,
) {
    let (history, mut bars) = match (history.lock(), bar_levels.lock()) {
        (Ok(h), Ok(b)) => (h, b),
        _ => return, // Mutex poisoned, skip update
    };

    if history.is_empty() {
        for bar in bars.iter_mut() {
            *bar *= 0.85;
        }
        return;
    }

    let center = NUM_BARS / 2;

    for i in 0..NUM_BARS {
        // Mirror from center: center bar uses most recent sample,
        // edge bars use progressively older samples — creates a natural
        // outward-spreading waveform like VoiceInk
        let distance = (i as i32 - center as i32).unsigned_abs() as usize;
        let hist_idx = history.len().saturating_sub(1 + distance * 2);
        let raw_amp = history.get(hist_idx).copied().unwrap_or(0.0);

        // Apply gain (typical mic RMS is 0.001–0.1)
        let gained = (raw_amp * 8.0).min(1.0);

        // VoiceInk-style amplitude boosting: compress dynamic range
        let target = gained.powf(0.5);

        // Smooth interpolation: fast attack, slow decay
        let current = bars[i];
        if target > current {
            bars[i] = current + (target - current) * 0.6;
        } else {
            bars[i] = current + (target - current) * 0.15;
        }
    }
}

fn draw_overlay(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    opacity: f64,
    state: RecordingState,
    bar_levels: &[f32; NUM_BARS],
    elapsed: f64,
) {
    // Clear entire surface to transparent
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    let radius = height / 2.0;
    let padding = 1.0;

    // Build capsule path
    cr.new_path();
    cr.arc(
        radius,
        height / 2.0,
        radius - padding,
        std::f64::consts::PI * 0.5,
        std::f64::consts::PI * 1.5,
    );
    cr.arc(
        width - radius,
        height / 2.0,
        radius - padding,
        std::f64::consts::PI * 1.5,
        std::f64::consts::PI * 0.5,
    );
    cr.close_path();

    // Dark semi-transparent fill
    cr.set_source_rgba(0.06, 0.06, 0.10, opacity);
    let _ = cr.fill_preserve();

    // Subtle border
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
    cr.set_line_width(0.5);
    let _ = cr.stroke();

    match state {
        RecordingState::Recording => {
            draw_waveform(cr, width, height, bar_levels, elapsed);
        }
        RecordingState::Transcribing => {
            draw_processing_dots(cr, width, height, elapsed);
        }
        _ => {}
    }
}

fn draw_waveform(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    bar_levels: &[f32; NUM_BARS],
    elapsed: f64,
) {
    let bar_width = 3.0;
    let bar_gap = 2.0;
    let total_width = NUM_BARS as f64 * (bar_width + bar_gap) - bar_gap;
    let start_x = (width - total_width) / 2.0;
    let center_y = height / 2.0;
    let max_bar_height = height * 0.65;
    let min_bar_height = 2.5;

    let center = NUM_BARS / 2;

    for (i, level) in bar_levels.iter().enumerate().take(NUM_BARS) {
        let x = start_x + i as f64 * (bar_width + bar_gap);
        let amp = *level as f64;

        // Center weighting: center bars slightly taller
        let distance_from_center = ((i as f64 - center as f64) / center as f64).abs();
        let center_weight = 1.0 - distance_from_center * 0.25;

        // Subtle idle breathing so user knows it's active
        let idle_breath = 1.0 + (elapsed * 1.5).sin() * 0.08;
        let bar_height = (min_bar_height * idle_breath)
            + (max_bar_height - min_bar_height) * amp * center_weight;

        let y = center_y - bar_height / 2.0;

        // Blue-white gradient: brighter with amplitude
        let intensity = 0.3 + amp * 0.7;
        cr.set_source_rgba(
            0.35 * intensity + 0.15,
            0.60 * intensity + 0.15,
            1.0 * intensity,
            0.7 + amp * 0.3,
        );

        // Draw rounded bar
        let bar_radius = bar_width / 2.0;
        if bar_height > bar_width {
            cr.new_path();
            cr.arc(
                x + bar_radius,
                y + bar_radius,
                bar_radius,
                std::f64::consts::PI,
                0.0,
            );
            cr.arc(
                x + bar_radius,
                y + bar_height - bar_radius,
                bar_radius,
                0.0,
                std::f64::consts::PI,
            );
            cr.close_path();
        } else {
            // Very short bar: just a circle
            cr.new_path();
            cr.arc(
                x + bar_radius,
                center_y,
                bar_radius,
                0.0,
                std::f64::consts::PI * 2.0,
            );
        }
        let _ = cr.fill();
    }
}

fn draw_processing_dots(cr: &cairo::Context, width: f64, height: f64, elapsed: f64) {
    let num_dots = 5;
    let dot_radius = 3.5;
    let dot_gap = 14.0;
    let total_width = num_dots as f64 * (dot_radius * 2.0 + dot_gap) - dot_gap;
    let start_x = (width - total_width) / 2.0;
    let center_y = height / 2.0;

    for i in 0..num_dots {
        let x = start_x + i as f64 * (dot_radius * 2.0 + dot_gap) + dot_radius;
        let phase = i as f64 * 0.5;

        // Sequential pulsing
        let pulse = ((elapsed * 2.5 - phase).sin() * 0.5 + 0.5).max(0.15);
        let r = dot_radius * (0.7 + pulse * 0.3);

        cr.set_source_rgba(0.35, 0.65, 1.0, 0.3 + pulse * 0.7);
        cr.new_path();
        cr.arc(x, center_y, r, 0.0, std::f64::consts::PI * 2.0);
        let _ = cr.fill();
    }
}
