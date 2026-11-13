//! Utter: a menu bar app that records on a global shortcut, transcribes
//! locally, and pastes the text into the focused app.

mod audio;
mod dictation;
mod model;
mod overlay;
mod paste;
mod settings;
mod shortcut;
mod tray;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_macos_permissions::{check_accessibility_permission, check_microphone_permission};

const SETTINGS_WINDOW: &str = "settings";

/// Prints Utter's own logs at info level and everything else at warn level to stderr.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let max = if metadata.target().starts_with("utter") {
            log::Level::Info
        } else {
            log::Level::Warn
        };
        metadata.level() <= max
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

/// Show the settings window and bring it to the front.
pub fn show_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        // Also activates the app, which matters because it has no Dock icon.
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<settings::Settings, String> {
    Ok(settings::all(&app))
}

/// macOS only shows the microphone prompt while the status is undecided.
/// After a denial it never prompts again, so open System Settings instead.
#[tauri::command]
async fn request_microphone() -> Result<(), String> {
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    const NOT_DETERMINED: isize = 0;
    let status: isize = {
        let audio = NSString::from_str("soun");
        unsafe { msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: &*audio] }
    };
    if status == NOT_DETERMINED {
        return tauri_plugin_macos_permissions::request_microphone_permission().await;
    }
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .status()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn list_microphones() -> Result<Vec<String>, String> {
    Ok(audio::list_input_devices())
}

#[tauri::command]
fn set_microphone(app: AppHandle, name: Option<String>) -> Result<(), String> {
    settings::set_microphone(&app, name)
}

#[tauri::command(async)]
fn set_shortcut(app: AppHandle, accel: String) -> Result<(), String> {
    shortcut::change(&app, &accel)
}

#[tauri::command(async)]
fn pause_shortcut(app: AppHandle) -> Result<(), String> {
    shortcut::pause(&app);
    Ok(())
}

#[tauri::command(async)]
fn resume_shortcut(app: AppHandle) -> Result<(), String> {
    shortcut::resume(&app);
    Ok(())
}

#[tauri::command]
fn shortcut_error(app: AppHandle) -> Result<Option<String>, String> {
    Ok(shortcut::error(&app))
}

#[tauri::command]
fn model_status(app: AppHandle) -> Result<model::Status, String> {
    Ok(model::status(&app))
}

#[tauri::command]
fn set_unload_after(app: AppHandle, minutes: Option<u32>) -> Result<(), String> {
    settings::set_unload_after_min(&app, minutes)
}

/// Resolves to `false` if the download was cancelled.
#[tauri::command]
async fn download_model(app: AppHandle) -> Result<bool, String> {
    model::download(&app).await
}

#[tauri::command]
fn cancel_download(app: AppHandle) -> Result<(), String> {
    model::cancel_download(&app);
    Ok(())
}

#[tauri::command(async)]
fn delete_model(app: AppHandle) -> Result<(), String> {
    if !dictation::is_idle(&app) {
        return Err("Can't delete the model during a dictation".into());
    }
    model::delete(&app)
}

fn has_permissions() -> bool {
    tauri::async_runtime::block_on(async {
        check_microphone_permission().await && check_accessibility_permission().await
    })
}

fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    app.manage(model::ModelState::default());
    app.manage(dictation::Dictation::default());
    app.manage(shortcut::ShortcutError::default());
    app.manage(audio::Recorder::spawn(app.clone()));

    tray::create(app)?;
    overlay::setup(app)?;
    shortcut::register_stored(app);

    // Load in the background so the first dictation doesn't wait for it.
    model::ensure_loading(app);
    model::spawn_reaper(app.clone());

    if !model::is_downloaded(app) || !has_permissions() {
        show_settings(app);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    static LOGGER: StderrLogger = StderrLogger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    // Send the native library's messages through `log` instead of raw stderr.
    transcribe_cpp::init_logging();

    let mut app = tauri::Builder::default()
        // Must be the first plugin. A second launch opens the settings window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(app);
        }))
        .plugin(shortcut::plugin())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_nspanel::init())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            list_microphones,
            request_microphone,
            set_microphone,
            set_shortcut,
            pause_shortcut,
            resume_shortcut,
            shortcut_error,
            model_status,
            set_unload_after,
            download_model,
            cancel_download,
            delete_model,
        ])
        .on_window_event(|window, event| {
            // Closing the settings window only hides it. The app keeps running.
            if window.label() == SETTINGS_WINDOW
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| setup(app.handle()))
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Menu bar only. Set before the event loop starts, so no Dock icon ever flashes.
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    app.run(|app, event| {
        // macOS does not start a second process when the app is opened again.
        // It sends a reopen event to the running one instead.
        if let RunEvent::Reopen { .. } = event {
            show_settings(app);
        }
    });
}
