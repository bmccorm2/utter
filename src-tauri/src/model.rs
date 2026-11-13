//! The speech model: download, load, unload and transcribe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;
use transcribe_cpp::{Model, RunOptions, Session};

use crate::audio::TARGET_RATE;
use crate::{dictation, settings, tray};

const FILE_NAME: &str = "parakeet-unified-en-0.6b-Q8_0.gguf";
const URL: &str = "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/main/parakeet-unified-en-0.6b-Q8_0.gguf";
const SIZE: u64 = 731_357_568;
const SHA256: &str = "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795";

/// Clips shorter than this (1.25 s) are padded with silence. On very short
/// clips the model invents words like "Okay." from background noise alone.
const MIN_INPUT_SAMPLES: usize = TARGET_RATE as usize * 5 / 4;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const REAPER_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Unloaded,
    Loading,
    Loaded,
}

struct Load {
    state: LoadState,
    /// Why the last load failed, if it did.
    error: Option<String>,
    /// Time of the last load or transcription.
    last_used: Instant,
}

/// Lock order, where more than one is held: dictation phase, then `load`, then `session`.
pub struct ModelState {
    session: Mutex<Option<Session>>,
    load: Mutex<Load>,
    load_changed: Condvar,
    downloading: AtomicBool,
    cancel_download: AtomicBool,
}

impl Default for ModelState {
    fn default() -> Self {
        ModelState {
            session: Mutex::new(None),
            load: Mutex::new(Load {
                state: LoadState::Unloaded,
                error: None,
                last_used: Instant::now(),
            }),
            load_changed: Condvar::new(),
            downloading: AtomicBool::new(false),
            cancel_download: AtomicBool::new(false),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub downloaded: bool,
    pub loaded: bool,
    pub loading: bool,
    pub downloading: bool,
    /// Why the last load failed, if it did.
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn model_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("app data dir is always available on macOS")
        .join("models")
        .join(FILE_NAME)
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

pub fn is_downloaded(app: &AppHandle) -> bool {
    model_path(app).is_file()
}

pub fn status(app: &AppHandle) -> Status {
    let state = app.state::<ModelState>();
    let load = lock(&state.load);
    Status {
        downloaded: is_downloaded(app),
        loaded: load.state == LoadState::Loaded,
        loading: load.state == LoadState::Loading,
        downloading: state.downloading.load(Ordering::SeqCst),
        error: load.error.clone(),
    }
}

/// Tell the settings page and the tray that the model status changed.
fn notify(app: &AppHandle) {
    let _ = app.emit_to("settings", "model-status", status(app));
    dictation::with_idle(app, || tray::set_tray_state(app, tray::idle_state(app)));
}

/// Start loading the model in the background if it is downloaded and not
/// already loaded or loading. Never starts a second load.
pub fn ensure_loading(app: &AppHandle) {
    let state = app.state::<ModelState>();
    {
        let mut load = lock(&state.load);
        if load.state != LoadState::Unloaded || !is_downloaded(app) {
            return;
        }
        load.state = LoadState::Loading;
        load.error = None;
    }
    let _ = app.emit_to("settings", "model-status", status(app));

    let app = app.clone();
    thread::spawn(move || {
        let started = Instant::now();
        let result = load_session(&model_path(&app));
        let state = app.state::<ModelState>();
        {
            let mut load = lock(&state.load);
            match result {
                Ok(session) => {
                    *lock(&state.session) = Some(session);
                    load.state = LoadState::Loaded;
                    load.last_used = Instant::now();
                    log::info!("Model loaded in {:.1?}", started.elapsed());
                }
                Err(e) => {
                    log::error!("Model load failed: {e}");
                    load.state = LoadState::Unloaded;
                    load.error = Some(e);
                }
            }
        }
        state.load_changed.notify_all();
        let _ = app.emit_to("settings", "model-status", status(&app));
    });
}

fn load_session(path: &Path) -> Result<Session, String> {
    let model = Model::load(path).map_err(|e| format!("Could not load the model: {e}"))?;
    let mut session = model
        .session()
        .map_err(|e| format!("Could not start the model: {e}"))?;
    // Warm up on 1 s of silence so the first real dictation is fast.
    let silence = vec![0.0_f32; TARGET_RATE as usize];
    if let Err(e) = session.run(&silence, &RunOptions::default()) {
        log::warn!("Model warm-up failed: {e}");
    }
    Ok(session)
}

/// Block until the model is loaded. Returns the load error if loading failed
/// or was never started.
pub fn wait_loaded(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<ModelState>();
    let mut load = lock(&state.load);
    loop {
        match load.state {
            LoadState::Loaded => return Ok(()),
            LoadState::Loading => {
                load = state
                    .load_changed
                    .wait(load)
                    .unwrap_or_else(|e| e.into_inner());
            }
            LoadState::Unloaded => {
                return Err(load
                    .error
                    .clone()
                    .unwrap_or_else(|| "The model is not loaded".to_string()));
            }
        }
    }
}

/// Drop the session to free its memory.
pub fn unload(app: &AppHandle) {
    let state = app.state::<ModelState>();
    {
        let mut load = lock(&state.load);
        // A load in progress would put the session back; let it finish first.
        while load.state == LoadState::Loading {
            load = state
                .load_changed
                .wait(load)
                .unwrap_or_else(|e| e.into_inner());
        }
        lock(&state.session).take();
        load.state = LoadState::Unloaded;
    }
    state.load_changed.notify_all();
    let _ = app.emit_to("settings", "model-status", status(app));
}

/// Unload the model and remove its file.
pub fn delete(app: &AppHandle) -> Result<(), String> {
    // The file is mmapped while loaded, so drop the session before removing it.
    unload(app);
    let path = model_path(app);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Could not delete the model: {e}"))?;
    }
    notify(app);
    Ok(())
}

/// Pad a short clip with trailing silence up to `MIN_INPUT_SAMPLES`.
fn pad_short(mut pcm: Vec<f32>) -> Vec<f32> {
    if pcm.len() < MIN_INPUT_SAMPLES {
        pcm.resize(MIN_INPUT_SAMPLES, 0.0);
    }
    pcm
}

/// Transcribe 16 kHz mono audio. The model must be loaded.
pub fn transcribe(app: &AppHandle, pcm: Vec<f32>) -> Result<String, String> {
    let state = app.state::<ModelState>();
    let started = Instant::now();
    let seconds = pcm.len() as f32 / TARGET_RATE as f32;
    let pcm = pad_short(pcm);
    let result = {
        let mut session = lock(&state.session);
        let session = session
            .as_mut()
            .ok_or_else(|| "The model is not loaded".to_string())?;
        session.run(&pcm, &RunOptions::default())
    };
    lock(&state.load).last_used = Instant::now();

    let text = match result {
        Ok(transcript) => transcript.text,
        // A truncated decode still carries what was recognized. Keep it
        // instead of throwing the whole dictation away.
        Err(e) => match e.partial() {
            Some(partial) if !partial.text.trim().is_empty() => {
                log::warn!("Transcription ended early: {e}");
                partial.text.clone()
            }
            _ => return Err(format!("Transcription failed: {e}")),
        },
    };
    log::info!(
        "Transcribed {seconds:.1} s of audio in {:.0?}",
        started.elapsed()
    );
    Ok(text)
}

/// Every 30 s, unload the model if it has been idle for the configured time.
pub fn spawn_reaper(app: AppHandle) {
    thread::spawn(move || {
        loop {
            thread::sleep(REAPER_INTERVAL);
            let Some(minutes) = settings::unload_after_min(&app) else {
                continue;
            };
            let limit = Duration::from_secs(u64::from(minutes) * 60);
            // Holding the dictation phase at Idle keeps a recording from starting mid-unload.
            dictation::with_idle(&app, || {
                let due = {
                    let state = app.state::<ModelState>();
                    let load = lock(&state.load);
                    load.state == LoadState::Loaded && load.last_used.elapsed() >= limit
                };
                if due {
                    log::info!("Unloading the model after {minutes} min idle");
                    unload(&app);
                }
            });
        }
    });
}

pub fn cancel_download(app: &AppHandle) {
    app.state::<ModelState>()
        .cancel_download
        .store(true, Ordering::SeqCst);
}

/// Download the model. Returns `Ok(false)` if the download was cancelled.
pub async fn download(app: &AppHandle) -> Result<bool, String> {
    if is_downloaded(app) {
        return Ok(true);
    }
    let state = app.state::<ModelState>();
    if state.downloading.swap(true, Ordering::SeqCst) {
        return Err("A download is already running".into());
    }
    state.cancel_download.store(false, Ordering::SeqCst);
    notify(app);

    let path = model_path(app);
    let part = part_path(&path);
    let result = fetch(&path, &part, &state.cancel_download, |downloaded, total| {
        let _ = app.emit_to(
            "settings",
            "download-progress",
            Progress { downloaded, total },
        );
    })
    .await;
    if !matches!(result, Ok(true)) {
        let _ = tokio::fs::remove_file(&part).await;
    }
    state.downloading.store(false, Ordering::SeqCst);
    if let Err(e) = &result {
        log::error!("Model download failed: {e}");
    }
    ensure_loading(app);
    notify(app);
    result
}

/// Stream the model to `part`, verify it, then rename it to `path`.
/// `on_progress(downloaded, total)` is called at most every 100 ms.
/// Returns `Ok(false)` if `cancel` was set.
async fn fetch(
    path: &Path,
    part: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<bool, String> {
    let io = |e: std::io::Error| format!("Could not save the model: {e}");
    let net = |e: reqwest::Error| format!("Download failed: {e}");

    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    let client = reqwest::Client::builder()
        .connect_timeout(STALL_TIMEOUT)
        .build()
        .map_err(net)?;
    let response = client
        .get(URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(net)?;
    let total = response.content_length().unwrap_or(SIZE);

    let mut file = tokio::fs::File::create(part).await.map_err(io)?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emit: Option<Instant> = None;
    let mut last_data = Instant::now();
    let mut stream = response.bytes_stream();

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(false);
        }
        // Wake up regularly so a cancel is noticed even when no data arrives.
        let next = tokio::time::timeout(Duration::from_millis(250), stream.next()).await;
        let chunk = match next {
            Err(_) if last_data.elapsed() >= STALL_TIMEOUT => {
                return Err("Download failed: the connection stalled".into());
            }
            Err(_) => continue,
            Ok(None) => break,
            Ok(Some(chunk)) => chunk.map_err(net)?,
        };
        last_data = Instant::now();
        file.write_all(&chunk).await.map_err(io)?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if last_emit.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
            last_emit = Some(Instant::now());
            on_progress(downloaded, total);
        }
    }
    file.flush().await.map_err(io)?;
    drop(file);
    on_progress(downloaded, total);

    if downloaded != SIZE {
        return Err(format!(
            "Download failed: expected {SIZE} bytes but got {downloaded}"
        ));
    }
    let digest = hex(&hasher.finalize());
    if digest != SHA256 {
        return Err("Download failed: the file's checksum does not match".into());
    }
    tokio::fs::rename(part, path).await.map_err(io)?;
    Ok(true)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_and_zero_padded() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
    }

    #[test]
    fn sha256_of_known_input() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn short_clips_are_padded_with_silence() {
        let padded = pad_short(vec![0.5; 8_000]);
        assert_eq!(padded.len(), 20_000);
        assert_eq!(padded[7_999], 0.5);
        assert!(padded[8_000..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn long_clips_are_not_padded() {
        assert_eq!(pad_short(vec![0.5; 20_000]).len(), 20_000);
        assert_eq!(pad_short(vec![0.5; 48_000]).len(), 48_000);
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(Path::new("/tmp/models/m.gguf")),
            PathBuf::from("/tmp/models/m.gguf.part")
        );
    }
}

/// Tests against the real model and network. They are slow, so they only run
/// on request:
///
/// ```fish
/// env UTTER_TEST_MODEL=/tmp/utter-test/model.gguf cargo test -- --ignored --test-threads 1
/// ```
#[cfg(test)]
mod engine_tests {
    use super::*;
    use std::process::Command;

    fn test_model() -> PathBuf {
        PathBuf::from(std::env::var("UTTER_TEST_MODEL").expect("set UTTER_TEST_MODEL"))
    }

    /// Speak `text` with macOS `say` and return it as 16 kHz mono f32.
    fn speak(text: &str) -> Vec<f32> {
        let wav = std::env::temp_dir().join(format!("utter-say-{}.wav", std::process::id()));
        let status = Command::new("say")
            .args([
                "-o",
                wav.to_str().unwrap(),
                "--data-format=LEI16@16000",
                text,
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let bytes = std::fs::read(&wav).unwrap();
        let _ = std::fs::remove_file(&wav);
        let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        bytes[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect()
    }

    #[test]
    #[ignore = "downloads 731 MB"]
    fn a_download_is_verified_and_saved() {
        let path = test_model();
        if path.is_file() {
            return;
        }
        let part = part_path(&path);
        let cancel = AtomicBool::new(false);
        let mut calls = 0_u32;
        let mut last = (0, 0);
        let started = Instant::now();
        let result = tauri::async_runtime::block_on(fetch(&path, &part, &cancel, |d, t| {
            calls += 1;
            assert!(d >= last.0, "progress went backwards");
            last = (d, t);
        }));
        assert_eq!(result, Ok(true));
        assert_eq!(last, (SIZE, SIZE));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), SIZE);
        assert!(!part.exists());
        // At most one progress call per 100 ms, plus the final one.
        assert!(u128::from(calls) <= started.elapsed().as_millis() / 100 + 2);
        println!(
            "downloaded in {:.0?} with {calls} progress calls",
            started.elapsed()
        );
    }

    #[test]
    #[ignore = "uses the network"]
    fn a_download_can_be_cancelled() {
        let dir = std::env::temp_dir().join(format!("utter-cancel-{}", std::process::id()));
        let path = dir.join("model.gguf");
        let part = part_path(&path);
        let cancel = AtomicBool::new(false);
        let result = tauri::async_runtime::block_on(fetch(&path, &part, &cancel, |_, _| {
            cancel.store(true, Ordering::SeqCst);
        }));
        assert_eq!(result, Ok(false));
        assert!(
            !path.exists(),
            "a cancelled download must not produce a model"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "needs the model"]
    fn transcribes_a_short_clip_fast() {
        let started = Instant::now();
        let mut session = load_session(&test_model()).unwrap();
        println!("load + warm-up: {:.2?}", started.elapsed());
        println!("limits: {:?}", session.limits());

        let pcm = speak("Hello world, this is a test.");
        let seconds = pcm.len() as f32 / TARGET_RATE as f32;
        let started = Instant::now();
        let text = session.run(&pcm, &RunOptions::default()).unwrap().text;
        let took = started.elapsed();
        println!("{seconds:.1} s clip in {took:.0?}: {text:?}");
        assert_eq!(text.trim(), "Hello world, this is a test.");
        assert!(took < Duration::from_secs(1), "took {took:?}");
    }

    #[test]
    #[ignore = "needs the model"]
    fn transcribes_a_two_minute_clip() {
        let mut session = load_session(&test_model()).unwrap();
        let sentence = "The quick brown fox jumps over the lazy dog near the quiet river bank. ";
        let mut pcm = speak(&format!("Start of recording. {}", sentence.repeat(30)));
        pcm.extend(speak("This is the end of the recording."));
        let seconds = pcm.len() as f32 / TARGET_RATE as f32;
        assert!(seconds >= 120.0, "clip is only {seconds:.0} s");

        let started = Instant::now();
        let text = session.run(&pcm, &RunOptions::default()).unwrap().text;
        println!("{seconds:.0} s clip in {:.1?}", started.elapsed());
        println!("{text}");
        let lower = text.to_lowercase();
        assert!(lower.starts_with("start of recording"), "{text}");
        assert!(
            lower.trim_end().ends_with("end of the recording."),
            "{text}"
        );
        assert!(lower.matches("quick brown fox").count() >= 28, "{text}");
    }

    /// Without padding, the model turns short noise-only clips into words
    /// like "Okay." or "Yeah.". An accidental tap must paste nothing.
    #[test]
    #[ignore = "needs the model"]
    fn short_noise_clips_give_no_text() {
        let mut session = load_session(&test_model()).unwrap();
        let mut seed = 999_u32;
        let mut noise = |samples: usize, amplitude: f32| -> Vec<f32> {
            (0..samples)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ((seed >> 8) as f32 / 8_388_608.0 - 1.0) * amplitude
                })
                .collect()
        };
        for amplitude in [0.0, 0.0005, 0.003, 0.01] {
            for samples in [1_600, 4_800, 8_000, 12_000, 16_000, 24_000, 80_000] {
                let pcm = pad_short(noise(samples, amplitude));
                let text = session.run(&pcm, &RunOptions::default()).unwrap().text;
                assert_eq!(text.trim(), "", "amplitude {amplitude}, {samples} samples");
            }
        }
    }

    #[test]
    #[ignore = "needs the model"]
    fn short_words_survive_padding() {
        let mut session = load_session(&test_model()).unwrap();
        for word in ["Yes.", "No.", "Okay.", "Stop.", "Thanks."] {
            let pcm = pad_short(speak(word));
            let text = session.run(&pcm, &RunOptions::default()).unwrap().text;
            assert_eq!(text.trim(), word);
        }
    }
}
