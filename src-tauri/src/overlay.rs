//! The recording overlay: a small pill at the bottom-center of the primary
//! monitor. It is a non-activating NSPanel, so it never takes keyboard focus
//! and ⌘V still goes to the app the user is typing in.

use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use tauri_nspanel::{
    CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt, tauri_panel,
};

const LABEL: &str = "overlay";
/// Gap between the pill and the bottom of the work area, in logical px.
const BOTTOM_MARGIN: f64 = 48.0;
const ERROR_DURATION: Duration = Duration::from_secs(2);

/// Bumped on every show or hide, so a pending error auto-hide can tell
/// whether the overlay has been reused since.
static GENERATION: AtomicU64 = AtomicU64::new(0);

tauri_panel! {
    panel!(OverlayPanel {
        config: {
            can_become_key_window: false,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Recording,
    Transcribing,
    Error,
}

#[derive(Clone, Serialize)]
struct StatePayload<'a> {
    state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

/// Convert the `overlay` window into a non-activating panel. Call once, on
/// the main thread, during app setup.
pub fn setup(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or("The overlay window is missing")?;
    let panel = window
        .to_panel::<OverlayPanel>()
        .map_err(|e| format!("Could not create the overlay panel: {e}"))?;

    panel
        .set_style_mask(
            StyleMask::empty()
                .borderless()
                .nonactivating_panel()
                .value(),
        )
        .map_err(|e| format!("Could not set the overlay style: {e:?}"))?;
    // One level above status windows, so it also shows over full-screen apps.
    panel.set_level(PanelLevel::Status.value() + 1);
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .value(),
    );
    panel.set_ignores_mouse_events(true);
    panel.set_hides_on_deactivate(false);
    panel.set_has_shadow(false);
    panel.hide();
    Ok(())
}

/// Show the overlay in the given state, without taking focus.
pub fn show(app: &AppHandle, state: State) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    present(app, state, None);
}

/// Show an error for 2 s, then hide.
pub fn show_error(app: &AppHandle, message: &str) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    present(app, State::Error, Some(message));

    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(ERROR_DURATION);
        // Leave it alone if a new dictation has taken over the overlay.
        if GENERATION.load(Ordering::SeqCst) == generation {
            order_out(&app);
        }
    });
}

pub fn hide(app: &AppHandle) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    order_out(app);
}

fn present(app: &AppHandle, state: State, message: Option<&str>) {
    let _ = app.emit_to(LABEL, "overlay-state", StatePayload { state, message });

    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        position(&handle);
        match handle.get_webview_panel(LABEL) {
            // Orders the panel front without making it key.
            Ok(panel) => panel.show(),
            Err(e) => log::error!("Overlay panel not found: {e:?}"),
        }
    });
    if let Err(e) = result {
        log::error!("Could not show the overlay: {e}");
    }
}

fn order_out(app: &AppHandle) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Ok(panel) = handle.get_webview_panel(LABEL) {
            panel.hide();
        }
    });
    if let Err(e) = result {
        log::error!("Could not hide the overlay: {e}");
    }
}

/// Move the overlay to the bottom-center of the primary monitor's work area.
fn position(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let (Ok(Some(monitor)), Ok(size)) = (app.primary_monitor(), window.outer_size()) else {
        return;
    };
    let area = monitor.work_area();
    let (x, y) = bottom_center(
        (area.position.x, area.position.y),
        (area.size.width, area.size.height),
        (size.width, size.height),
        monitor.scale_factor(),
    );
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Top-left corner, in physical px, that centers a window horizontally in the
/// work area and leaves `BOTTOM_MARGIN` logical px below it.
fn bottom_center(
    area_pos: (i32, i32),
    area_size: (u32, u32),
    window_size: (u32, u32),
    scale_factor: f64,
) -> (i32, i32) {
    let margin = (BOTTOM_MARGIN * scale_factor).round() as i32;
    let x = area_pos.0 + (area_size.0 as i32 - window_size.0 as i32) / 2;
    let y = area_pos.1 + area_size.1 as i32 - window_size.1 as i32 - margin;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_at_the_bottom_of_the_work_area() {
        // 1440×900 pt display at 2x, menu bar 25 pt, Dock 70 pt, 200×44 pt window.
        let (x, y) = bottom_center((0, 50), (2880, 1610), (400, 88), 2.0);
        assert_eq!(x, 1240);
        // Work area bottom is 1660. Minus the 88 px window and the 96 px margin.
        assert_eq!(y, 1476);
    }

    #[test]
    fn respects_the_work_area_origin() {
        let (x, y) = bottom_center((100, 0), (1000, 800), (200, 44), 1.0);
        assert_eq!((x, y), (500, 708));
    }

    #[test]
    fn state_payload_shape() {
        let json = serde_json::to_value(StatePayload {
            state: State::Recording,
            message: None,
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({ "state": "recording" }));
        let json = serde_json::to_value(StatePayload {
            state: State::Error,
            message: Some("boom"),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "state": "error", "message": "boom" })
        );
    }
}
