//! Menu bar (tray) icon and its menu.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::{dictation, model};

const TRAY_ID: &str = "main";
const ICON_IDLE: &[u8] = include_bytes!("../icons/tray.png");
const ICON_RECORDING: &[u8] = include_bytes!("../icons/tray-recording.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    Recording,
    Transcribing,
    NoModel,
}

impl State {
    fn status_label(self) -> &'static str {
        match self {
            State::Ready => "Utter: Ready",
            State::Recording => "Recording…",
            State::Transcribing => "Transcribing…",
            State::NoModel => "Model not downloaded",
        }
    }

    fn toggle_label(self) -> &'static str {
        match self {
            State::Recording => "Stop Dictation",
            _ => "Start Dictation",
        }
    }
}

/// Menu items whose text changes with the state.
struct TrayItems {
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let state = idle_state(app);
    let status = MenuItem::with_id(app, "status", state.status_label(), false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", state.toggle_label(), true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Utter", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_IDLE)?)
        // The icon is always white, so it must not be a template image.
        .icon_as_template(false)
        .tooltip("Utter")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => dictation::toggle_async(app),
            "settings" => crate::show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    app.manage(TrayItems { status, toggle });
    Ok(())
}

/// The state to show while no dictation is in progress.
pub fn idle_state(app: &AppHandle) -> State {
    if model::is_downloaded(app) {
        State::Ready
    } else {
        State::NoModel
    }
}

/// Swap the tray icon and update the menu labels.
pub fn set_tray_state(app: &AppHandle, state: State) {
    let app = app.clone();
    let handle = app.clone();
    // Menu and tray changes must happen on the main thread.
    let result = handle.run_on_main_thread(move || {
        if let Some(items) = app.try_state::<TrayItems>() {
            let _ = items.status.set_text(state.status_label());
            let _ = items.toggle.set_text(state.toggle_label());
            let _ = items.toggle.set_enabled(state != State::Transcribing);
        }
        let bytes = if state == State::Recording {
            ICON_RECORDING
        } else {
            ICON_IDLE
        };
        if let (Some(tray), Ok(icon)) = (app.tray_by_id(TRAY_ID), Image::from_bytes(bytes)) {
            let _ = tray.set_icon_with_as_template(Some(icon), false);
        }
    });
    if let Err(e) = result {
        log::error!("Failed to update the tray: {e}");
    }
}
