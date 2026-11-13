//! The dictation state machine: Idle → Recording → Transcribing → Idle.

use std::sync::{Mutex, MutexGuard};
use std::thread;

use tauri::{AppHandle, Manager};

use crate::audio::Recorder;
use crate::{model, overlay, paste, settings, shortcut, tray};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Recording,
    Transcribing,
}

pub struct Dictation {
    phase: Mutex<Phase>,
}

impl Default for Dictation {
    fn default() -> Self {
        Dictation {
            phase: Mutex::new(Phase::Idle),
        }
    }
}

fn lock_phase(app: &AppHandle) -> MutexGuard<'_, Phase> {
    let dictation: &Dictation = app.state::<Dictation>().inner();
    dictation.phase.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_idle(app: &AppHandle) -> bool {
    *lock_phase(app) == Phase::Idle
}

/// Run `f` only if no dictation is in progress. None can start while it runs.
pub fn with_idle(app: &AppHandle, f: impl FnOnce()) {
    let phase = lock_phase(app);
    if *phase == Phase::Idle {
        f();
    }
}

/// Start or stop dictation. Used by the global shortcut and the tray menu.
pub fn toggle_async(app: &AppHandle) {
    let app = app.clone();
    // Opening the mic and transcribing both block, so stay off the main thread.
    thread::spawn(move || toggle(&app));
}

/// Cancel a recording in progress. Used by the Escape shortcut.
pub fn cancel_async(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || cancel(&app));
}

fn toggle(app: &AppHandle) {
    let mut phase = lock_phase(app);
    match *phase {
        Phase::Idle => {
            if !model::is_downloaded(app) {
                drop(phase);
                crate::show_settings(app);
                return;
            }
            // Record right away. If the model was unloaded it loads meanwhile.
            model::ensure_loading(app);
            if let Err(e) = app.state::<Recorder>().start(settings::microphone(app)) {
                log::error!("Could not start recording: {e}");
                overlay::show_error(app, &e);
                return;
            }
            shortcut::register_escape(app);
            *phase = Phase::Recording;
            tray::set_tray_state(app, tray::State::Recording);
            overlay::show(app, overlay::State::Recording);
        }
        Phase::Recording => {
            *phase = Phase::Transcribing;
            drop(phase);
            shortcut::unregister_escape(app);
            tray::set_tray_state(app, tray::State::Transcribing);
            overlay::show(app, overlay::State::Transcribing);

            match transcribe_and_paste(app) {
                Ok(()) => overlay::hide(app),
                Err(e) => {
                    log::error!("Dictation failed: {e}");
                    overlay::show_error(app, &e);
                }
            }
            *lock_phase(app) = Phase::Idle;
            tray::set_tray_state(app, tray::idle_state(app));
        }
        // Presses while transcribing are ignored.
        Phase::Transcribing => {}
    }
}

fn cancel(app: &AppHandle) {
    let mut phase = lock_phase(app);
    if *phase != Phase::Recording {
        return;
    }
    app.state::<Recorder>().cancel();
    shortcut::unregister_escape(app);
    overlay::hide(app);
    *phase = Phase::Idle;
    tray::set_tray_state(app, tray::idle_state(app));
}

fn transcribe_and_paste(app: &AppHandle) -> Result<(), String> {
    let pcm = app.state::<Recorder>().stop()?;
    if pcm.is_empty() {
        return Ok(());
    }
    // The idle reaper may have unloaded the model. Reload it if so, then
    // wait for any load in progress to finish.
    model::ensure_loading(app);
    model::wait_loaded(app)?;
    let transcript = model::transcribe(app, pcm)?;
    match paste::post_process(&transcript) {
        Some(text) => paste::paste(app, &text),
        None => Ok(()),
    }
}
