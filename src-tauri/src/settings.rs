//! Persistent settings, stored in `settings.json` via tauri-plugin-store.

use serde::Serialize;
use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

const FILE: &str = "settings.json";
const KEY_SHORTCUT: &str = "shortcut";
const KEY_MICROPHONE: &str = "microphone";
const KEY_UNLOAD: &str = "unload_after_min";

pub const DEFAULT_SHORTCUT: &str = "Alt+Space";
pub const DEFAULT_UNLOAD_MIN: u32 = 5;
pub const UNLOAD_CHOICES: [u32; 4] = [5, 15, 30, 60];

#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub shortcut: String,
    pub microphone: Option<String>,
    pub unload_after_min: Option<u32>,
}

fn get(app: &AppHandle, key: &str) -> Option<Value> {
    app.store(FILE).ok()?.get(key)
}

fn set(app: &AppHandle, key: &str, value: Value) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set(key, value);
    store.save().map_err(|e| e.to_string())
}

pub fn all(app: &AppHandle) -> Settings {
    Settings {
        shortcut: shortcut(app),
        microphone: microphone(app),
        unload_after_min: unload_after_min(app),
    }
}

pub fn shortcut(app: &AppHandle) -> String {
    match get(app, KEY_SHORTCUT) {
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => DEFAULT_SHORTCUT.to_string(),
    }
}

pub fn set_shortcut(app: &AppHandle, accel: &str) -> Result<(), String> {
    set(app, KEY_SHORTCUT, json!(accel))
}

/// The saved input device name. `None` means the system default.
pub fn microphone(app: &AppHandle) -> Option<String> {
    match get(app, KEY_MICROPHONE) {
        Some(Value::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

pub fn set_microphone(app: &AppHandle, name: Option<String>) -> Result<(), String> {
    set(app, KEY_MICROPHONE, json!(name))
}

/// Minutes of idle time before the model is unloaded. `None` means never.
pub fn unload_after_min(app: &AppHandle) -> Option<u32> {
    parse_unload(get(app, KEY_UNLOAD))
}

pub fn set_unload_after_min(app: &AppHandle, minutes: Option<u32>) -> Result<(), String> {
    if !is_valid_unload(minutes) {
        return Err("Unload time must be Never, 5, 15, 30 or 60 minutes".into());
    }
    set(app, KEY_UNLOAD, json!(minutes))
}

fn is_valid_unload(minutes: Option<u32>) -> bool {
    minutes.is_none_or(|m| UNLOAD_CHOICES.contains(&m))
}

/// A missing or invalid value gives the default. An explicit `null` means never.
fn parse_unload(value: Option<Value>) -> Option<u32> {
    match value {
        Some(Value::Null) => None,
        Some(Value::Number(n)) => match n.as_u64().and_then(|m| u32::try_from(m).ok()) {
            Some(m) if UNLOAD_CHOICES.contains(&m) => Some(m),
            _ => Some(DEFAULT_UNLOAD_MIN),
        },
        _ => Some(DEFAULT_UNLOAD_MIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unload_defaults_to_five_minutes() {
        assert_eq!(parse_unload(None), Some(5));
        assert_eq!(parse_unload(Some(json!("x"))), Some(5));
        assert_eq!(parse_unload(Some(json!(7))), Some(5));
    }

    #[test]
    fn unload_null_means_never() {
        assert_eq!(parse_unload(Some(Value::Null)), None);
    }

    #[test]
    fn unload_accepts_only_listed_values() {
        for m in UNLOAD_CHOICES {
            assert_eq!(parse_unload(Some(json!(m))), Some(m));
            assert!(is_valid_unload(Some(m)));
        }
        assert!(is_valid_unload(None));
        assert!(!is_valid_unload(Some(0)));
        assert!(!is_valid_unload(Some(10)));
    }
}
