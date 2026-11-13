//! Put the transcript into the focused app: clipboard, ⌘V, restore clipboard.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use enigo::{Direction, Enigo, Key, Keyboard, NewConError, Settings};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

/// Time the target app gets to read the clipboard before it is restored.
const RESTORE_DELAY: Duration = Duration::from_millis(150);
/// macOS virtual key code of the V key on QWERTY-family layouts (`kVK_ANSI_V`).
const KEY_V: u32 = 9;

/// Trim the transcript and add one trailing space. `None` if nothing is left.
pub fn post_process(transcript: &str) -> Option<String> {
    let text = transcript.trim();
    if text.is_empty() {
        None
    } else {
        Some(format!("{text} "))
    }
}

pub fn paste(app: &AppHandle, text: &str) -> Result<(), String> {
    let clipboard = app.clipboard();
    // Fails when the clipboard holds non-text. Then nothing is restored.
    let previous = clipboard.read_text().ok();
    clipboard
        .write_text(text)
        .map_err(|e| format!("Clipboard error: {e}"))?;

    let pasted = send_cmd_v(app);
    thread::sleep(RESTORE_DELAY);
    if let Some(previous) = previous
        && let Err(e) = clipboard.write_text(previous)
    {
        log::warn!("Could not restore the clipboard: {e}");
    }
    pasted
}

/// Send ⌘V. enigo must run on the main thread on macOS.
fn send_cmd_v(app: &AppHandle) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(press_cmd_v());
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

fn press_cmd_v() -> Result<(), String> {
    let settings = Settings {
        // The settings page handles permissions. Don't pop a system prompt mid-dictation.
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    };
    let mut enigo = Enigo::new(&settings).map_err(|e| match e {
        NewConError::NoPermission => "Accessibility permission is needed to paste".to_string(),
        e => format!("Could not paste: {e}"),
    })?;
    let key = |enigo: &mut Enigo, key: Key, direction: Direction| {
        enigo
            .key(key, direction)
            .map_err(|e| format!("Could not paste: {e}"))
    };
    key(&mut enigo, Key::Meta, Direction::Press)?;
    let clicked = key(&mut enigo, Key::Other(KEY_V), Direction::Click);
    // Always release ⌘, even if the V press failed.
    key(&mut enigo, Key::Meta, Direction::Release)?;
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_adds_one_trailing_space() {
        assert_eq!(
            post_process("  Hello world, this is a test.\n").as_deref(),
            Some("Hello world, this is a test. ")
        );
        assert_eq!(post_process("Hi").as_deref(), Some("Hi "));
        assert_eq!(post_process("Hi   ").as_deref(), Some("Hi "));
    }

    #[test]
    fn empty_or_whitespace_gives_none() {
        assert_eq!(post_process(""), None);
        assert_eq!(post_process("   \n\t "), None);
    }

    #[test]
    fn keeps_inner_whitespace() {
        assert_eq!(post_process(" a  b ").as_deref(), Some("a  b "));
    }
}
