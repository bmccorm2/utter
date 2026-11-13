//! Global shortcuts: the dictation toggle, and Escape while recording.

use std::sync::Mutex;

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{dictation, settings};

/// Why the dictation shortcut could not be registered, if it could not.
#[derive(Default)]
pub struct ShortcutError(Mutex<Option<String>>);

fn escape() -> Shortcut {
    Shortcut::new(None, Code::Escape)
}

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            // Act on key down only. Key up and repeats are ignored.
            if event.state != ShortcutState::Pressed {
                return;
            }
            if *shortcut == escape() {
                dictation::cancel_async(app);
            } else {
                dictation::toggle_async(app);
            }
        })
        .build()
}

fn parse(accel: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = accel
        .parse()
        .map_err(|e| format!("\"{accel}\" is not a valid shortcut: {e}"))?;
    if shortcut == escape() {
        return Err("Escape is reserved for cancelling a recording".into());
    }
    Ok(shortcut)
}

fn set_error(app: &AppHandle, error: Option<String>) {
    let state = app.state::<ShortcutError>();
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = error;
}

pub fn error(app: &AppHandle) -> Option<String> {
    let state = app.state::<ShortcutError>();
    let error = state.0.lock().unwrap_or_else(|e| e.into_inner());
    error.clone()
}

fn register(app: &AppHandle, accel: &str) -> Result<(), String> {
    let shortcut = parse(accel)?;
    let shortcuts = app.global_shortcut();
    if shortcuts.is_registered(shortcut) {
        return Ok(());
    }
    shortcuts
        .register(shortcut)
        .map_err(|e| format!("Could not register {accel}: {e}. Another app may be using it."))
}

fn unregister(app: &AppHandle, accel: &str) {
    if let Ok(shortcut) = parse(accel) {
        let shortcuts = app.global_shortcut();
        if shortcuts.is_registered(shortcut) {
            let _ = shortcuts.unregister(shortcut);
        }
    }
}

/// Register the stored shortcut. On failure the error is kept for the
/// settings page to show.
pub fn register_stored(app: &AppHandle) {
    let result = register(app, &settings::shortcut(app));
    if let Err(e) = &result {
        log::error!("{e}");
    }
    set_error(app, result.err());
}

/// Switch to a new shortcut. On failure the old one stays in place.
pub fn change(app: &AppHandle, accel: &str) -> Result<(), String> {
    let old = settings::shortcut(app);
    parse(accel)?;
    unregister(app, &old);
    if let Err(e) = register(app, accel) {
        register_stored(app);
        return Err(e);
    }
    if let Err(e) = settings::set_shortcut(app, accel) {
        unregister(app, accel);
        register_stored(app);
        return Err(e);
    }
    set_error(app, None);
    Ok(())
}

/// Release the dictation shortcut while the settings page captures keys.
pub fn pause(app: &AppHandle) {
    unregister(app, &settings::shortcut(app));
}

pub fn resume(app: &AppHandle) {
    register_stored(app);
}

/// Escape is a global shortcut only while recording, so it is never
/// taken away from other apps otherwise.
pub fn register_escape(app: &AppHandle) {
    if let Err(e) = app.global_shortcut().register(escape()) {
        log::warn!("Could not register Escape: {e}");
    }
}

pub fn unregister_escape(app: &AppHandle) {
    let shortcuts = app.global_shortcut();
    if shortcuts.is_registered(escape()) {
        let _ = shortcuts.unregister(escape());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Modifiers;

    #[test]
    fn parses_the_default_shortcut() {
        assert_eq!(
            parse(settings::DEFAULT_SHORTCUT).unwrap(),
            Shortcut::new(Some(Modifiers::ALT), Code::Space)
        );
    }

    #[test]
    fn parses_what_the_settings_page_builds() {
        assert_eq!(
            parse("Control+Alt+D").unwrap(),
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyD)
        );
        assert_eq!(
            parse("Super+Shift+1").unwrap(),
            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Digit1)
        );
        assert_eq!(parse("F5").unwrap(), Shortcut::new(None, Code::F5));
    }

    #[test]
    fn rejects_invalid_and_reserved_shortcuts() {
        assert!(parse("").is_err());
        assert!(parse("Alt+").is_err());
        assert!(parse("Alt+Nope").is_err());
        assert!(parse("Escape").is_err());
    }
}
