# Utter: local speech-to-text menu bar app for macOS

## Goal
Build **Utter**, a macOS menu-bar app (Tauri 2 + Rust) that records from a chosen mic when a global shortcut is pressed. It transcribes the audio locally with NVIDIA Parakeet Unified English 0.6B and pastes the text into the focused app. It replaces the user's current use of Handy (cjpais/Handy, open source, same engine; use its source as a reference when stuck).

## Context
- Machine: Apple Silicon (arm64), macOS 27, Rust 1.99 (`~/.cargo/bin`), bun, node. User shell is fish.
- **CMake is NOT installed** and is required by `transcribe-cpp-sys`. Run `brew install cmake` first.
- Project root: `/Users/bryan/src/speech-to-text`. It is currently empty except this file and is not a git repo.
- Commands (after scaffold):
  - Dev: `bun tauri dev`
  - Build: `bun tauri build` (produces `src-tauri/target/release/bundle/macos/Utter.app`)
  - Rust checks: `cd src-tauri; cargo clippy -- -D warnings; cargo test`
  - Frontend type check: `bunx tsc --noEmit`
- Never run a scaffolder with `--force` in a non-empty dir. Scaffold into a temp dir and copy the files in.
- Styling: copy `/Users/bryan/.pi/agent/skills/dark-theme/theme.css` into `src/theme.css` and read `/Users/bryan/.pi/agent/skills/dark-theme/SKILL.md`. The app is dark-only (`<html data-theme="dark">`). Use tokens (`--bg`, `--accent`, `--running`, …), never hard-coded hexes in components. Font: `@fontsource-variable/open-sans`. Icons: `lucide` (vanilla package).
- Model: `handy-computer/parakeet-unified-en-0.6b-gguf`, file `parakeet-unified-en-0.6b-Q8_0.gguf`
  - URL: `https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/main/parakeet-unified-en-0.6b-Q8_0.gguf`
  - Size: `731357568` bytes. SHA-256: `4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795`
- Inference crate: `transcribe-cpp` 0.4.x (crates.io; the `metal` feature is the default). API:
  ```rust
  use transcribe_cpp::{Model, RunOptions};
  let mut session = Model::load(path)?.session()?;   // Session is Send
  let t = session.run(&pcm_16k_mono_f32, &RunOptions::default())?; // t.text
  ```
  Weights are mmapped. Do not overwrite a loaded model file in place.

## Decisions
- **Name** `Utter`; bundle id `com.bryan.utter`; productName `Utter`. Keep the repo directory name as-is.
- **Stack:** Tauri 2, Vite, vanilla TypeScript, bun. No frontend framework, no Tailwind.
- **Engine:** `transcribe-cpp` with Metal, model Q8_0 GGUF. This is the fastest option on Apple Silicon (~200× realtime on Metal), with WER the same as F32.
  - Load the model at app start (background thread) if the file exists.
  - Right after every load, run one warm-up `session.run` on 1 s of silence so the next dictation is fast.
  - Use offline (non-streaming) transcription of the whole recording on stop, with `RunOptions::default()` (punctuation and capitalization on).
- **Unload when idle:** a setting with the options Never / 5 / 15 / 30 / 60 minutes, default **5 min**.
  - Once the app has been `Idle` that long since the last load or transcription, drop the session to free memory.
  - If the model is unloaded when the shortcut is pressed, start recording immediately **and** reload the model in the background at the same time.
  - On stop, wait for the load to finish (overlay shows "Transcribing…"), then transcribe.
  - Never load twice at once.
- **One model only:** no picker. Download/Delete on the settings page. Store it at `<app_data_dir>/models/parakeet-unified-en-0.6b-Q8_0.gguf` (`~/Library/Application Support/com.bryan.utter/models/`). Download fresh; do NOT reuse Handy's HF cache.
- **Shortcut:** default `Alt+Space` (⌥Space). It toggles: one press starts recording, the next press stops and transcribes.
  - Act only on `ShortcutState::Pressed`.
  - Ignore presses while transcribing.
  - The shortcut can be changed in settings.
- **Esc cancels** a recording: discard the audio, no transcription. Register `Escape` as a global shortcut only while recording, and unregister it when recording ends, so Esc is never hijacked otherwise.
- **Output:**
  - Trim the transcript. If it is empty, do nothing.
  - Otherwise append one trailing space.
  - Save the current clipboard text, write the transcript to the clipboard, send ⌘V, wait 150 ms, then restore the previous clipboard text. If the previous clipboard held non-text, it is not restored (accepted limitation).
- **Microphone:** settings lists every input device, with "System default" first. Persist the device name, or `null` for the default. If the saved device is missing at record time, silently use the system default.
- **Audio capture:** open the input stream only while recording, so the macOS mic indicator appears only then.
- **Overlay:** a small pill window at the bottom-center of the **primary monitor's work area**, 48 px above its bottom edge. It never takes focus.
  - Recording: live waveform bars in `--accent`.
  - Transcribing: a `--running` spinner plus "Transcribing…".
  - On success it hides. On error it shows the message in `--error` for 2 s, then hides.
- **No Dock icon:** `ActivationPolicy::Accessory`. Menu bar (tray) icon only, plus a settings window.
- **Tray icon is always WHITE:** not a template image (`icon_as_template(false)`). Two variants:
  - Idle: white waveform.
  - Recording: white waveform plus a small tangerine `#f06c00` dot.
- **Autostart:** `tauri-plugin-autostart` (LaunchAgent), toggled in settings. Default off.
- **Permissions:** the settings page has a Permissions section with Microphone and Accessibility rows (status plus a "Grant" button), using `tauri-plugin-macos-permissions`.
- **Single instance:** `tauri-plugin-single-instance`. A second launch opens the settings window.
- **Distribution:** local unsigned build only. No notarization, auto-update, history, multi-language, or sounds.
- **App icon:** custom SVG designed in this task (see Task 3), generated with `bun tauri icon`.

## Tasks
**Progress tracking (required):** the moment a task/milestone is done, tick it (`- [x]`) in this file, before starting the next one. Never batch ticks at the end; the user watches this file to see progress.

### M1: Scaffold
- [x] 1. `brew install cmake`. Run `git init`. Add `.gitignore` (`node_modules/`, `dist/`, `src-tauri/target/`, `.DS_Store`). Don't commit.
- [x] 2. Scaffold into a temp dir: `bun create tauri-app /tmp/utter-scaffold --template vanilla-ts --manager bun --identifier com.bryan.utter -y`. Copy its contents into the project root without overwriting `todo.md`. Then in `src-tauri/tauri.conf.json`:
  - `productName: "Utter"`
  - `app.macOSPrivateApi: true`
  - Define two windows:
    - `settings`: `url: "index.html"`, title "Utter Settings", 520×640, `resizable: false`, `visible: false`
    - `overlay`: `url: "overlay.html"`, 200×44, `transparent: true`, `decorations: false`, `shadow: false`, `alwaysOnTop: true`, `skipTaskbar: true`, `focus: false`, `resizable: false`, `visible: false`, `visibleOnAllWorkspaces: true`

  Make Vite multi-page: `build.rollupOptions.input = { main: "index.html", overlay: "overlay.html" }`. Confirm that `bun tauri dev` launches.
- [x] 3. Icons. Create `design/icon.svg` (1024×1024):
  - A macOS-style rounded square (824×824 at offset 100, corner radius ~185) filled with a vertical gradient `#30333b` → `#1f2227`.
  - Centered, 5 vertical rounded bars (a waveform) with a tangerine gradient `#ff8a2a` → `#f06c00`: width 56, gap 40, heights 150/290/420/290/150.

  Then:
  - Run `bun tauri icon design/icon.svg` to replace `src-tauri/icons/*`.
  - Create `design/tray.svg` (viewBox 0 0 22 22): the same 5 bars in pure white `#ffffff`, transparent background, bars ~2.5 wide, heights ~6/11/16/11/6.
  - Create `design/tray-recording.svg`: the same, plus a 6×6 `#f06c00` circle at the top-right.
  - Render both to 44×44 PNGs: `src-tauri/icons/tray.png` and `src-tauri/icons/tray-recording.png`. Use `bun tauri icon design/tray.svg -o /tmp/tray --png 44` and copy the 44px PNG. If that flag isn't supported, use `brew install librsvg` + `rsvg-convert -w 44 -h 44`.

### M2: Tray, windows, settings storage
- [x] 4. `src-tauri/Cargo.toml`: add the dependencies listed below. If a pinned version is outdated, use the latest compatible one.
  - `tauri` features `tray-icon`, `macos-private-api`, `image-png`
  - Tauri plugins: `tauri-plugin-global-shortcut`, `tauri-plugin-autostart`, `tauri-plugin-store`, `tauri-plugin-clipboard-manager`, `tauri-plugin-macos-permissions`, `tauri-plugin-single-instance`
  - `tauri-nspanel = { git = "https://github.com/ahkohd/tauri-nspanel", branch = "v2.1" }`
  - Other crates: `cpal`, `rubato`, `transcribe-cpp = "0.4"`, `reqwest` (features `stream`, `rustls-tls`; no default features), `futures-util`, `sha2`, `enigo`, `serde`/`serde_json`, `anyhow`, `tokio`, `log`

  Add matching JS packages: `@tauri-apps/plugin-*`, `tauri-plugin-macos-permissions-api`, `lucide`, `@fontsource-variable/open-sans`. Add the required capabilities in `src-tauri/capabilities/default.json` for both windows.
- [x] 5. Create `src-tauri/Info.plist` with `NSMicrophoneUsageDescription` = "Utter uses the microphone to transcribe what you say." (Tauri 2 merges it into the bundle.)
- [x] 6. Split Rust into modules under `src-tauri/src/`:
  - `lib.rs` (setup and wiring)
  - `tray.rs`
  - `settings.rs` (store)
  - `audio.rs`
  - `model.rs` (download, load, transcribe)
  - `dictation.rs` (state machine)
  - `paste.rs`
  - `overlay.rs`
- [x] 7. `lib.rs` setup:
  - Set `ActivationPolicy::Accessory`.
  - Register the plugins.
  - On settings-window `CloseRequested`: call `prevent_close()` and hide.
  - On startup, show the settings window if the model isn't downloaded or a permission is missing.
  - To show settings, call `window.show()` + `set_focus()` (and activate the app if needed so it comes to the front).
- [x] 8. `settings.rs`: use `tauri-plugin-store` file `settings.json` with keys `shortcut: String` (default `"Alt+Space"`), `microphone: Option<String>` (default `null`) and `unload_after_min: Option<u32>` (default `Some(5)`; `None` = Never; allowed values 5/15/30/60). Typed getters and setters.
- [x] 9. `tray.rs`:
  - The tray uses `icons/tray.png` with `icon_as_template(false)`.
  - Menu:
    - disabled status item: "Utter: Ready" / "Recording…" / "Transcribing…" / "Model not downloaded"
    - separator
    - "Start Dictation" / "Stop Dictation" (label follows the state)
    - "Settings…"
    - separator
    - "Quit Utter"
  - Expose `set_tray_state(app, State)`, which swaps the icon (`tray-recording.png` while recording) and updates the labels.

### M3: Audio, model, dictation
- [x] 10. `audio.rs`. `cpal::Stream` is `!Send`, so own it on a dedicated audio thread controlled through an `mpsc` channel (`Start { device: Option<String> }`, `Stop -> Vec<f32>`, `Cancel`).
  - `list_input_devices() -> Vec<String>`.
  - In the callback, downmix to mono f32 and append to the buffer.
  - Every ~33 ms of audio, compute RMS and send it (non-blocking) to a thread that emits `level` events (`{ rms: f32 }`) to the `overlay` window.
  - On stop, resample to 16 kHz mono with `rubato` (FFT resampler) unless the device is already 16 kHz.
- [x] 11. `model.rs`:
  - `model_path(app)`.
  - `download(app)`:
    - Stream to `<path>.part` with reqwest and emit `download-progress { downloaded, total }` at most every 100 ms.
    - Support cancellation through an `AtomicBool`.
    - Verify the size and SHA-256 (values in Context). On mismatch, delete and emit an error. On success, rename to the final path.
  - Model state in app state:
    - `Mutex<Option<Session>>` for the session
    - a load state `Unloaded | Loading | Loaded`, with a `Condvar` (or `tokio::sync::watch`) so callers can wait for an in-progress load
    - `last_used: Instant`
  - `ensure_loading(app)`: if `Unloaded` and the file exists, spawn a load (`Model::load` → `session()` → warm-up run). It's a no-op if already `Loading`/`Loaded`, so there is never a double load.
  - `wait_loaded(app) -> Result<()>`: blocks until `Loaded`, or returns the load error.
  - `unload(app)`: drop the session and set `Unloaded`.
  - Idle reaper thread: every 30 s, if `unload_after_min` is `Some(m)`, the model is `Loaded`, dictation is `Idle`, and `last_used.elapsed() >= m` minutes, call `unload`.
  - Update `last_used` after each load and each transcription.
  - `delete(app)`: `unload`, then remove the file.
  - `transcribe(pcm) -> Result<String>`.
- [x] 12. `dictation.rs`: state machine `Idle → Recording → Transcribing → Idle` in app state.
  - Shortcut press while `Idle`:
    - If the model file isn't downloaded, open settings and return.
    - Otherwise: call `model::ensure_loading`, start audio, register `Escape`, set the tray to recording, show the overlay in recording state.
  - Shortcut press while `Recording`:
    - Stop audio and unregister `Escape`.
    - Set the overlay to transcribing and the tray to transcribing.
    - On a blocking thread: `model::wait_loaded`, transcribe, then paste.
    - Hide the overlay and return to `Idle`.
  - Escape while `Recording`: cancel audio, unregister `Escape`, hide the overlay, go `Idle`.
  - Shortcut press while `Transcribing`: ignored.
  - On error: overlay error state (2 s), back to `Idle`, log it.
  - The tray Start/Stop menu item calls the same toggle function.
- [x] 13. `paste.rs`:
  - Read the clipboard text (clipboard-manager plugin).
  - Write the transcript.
  - Send ⌘V via `enigo` (run on the main thread via `app.run_on_main_thread`).
  - Sleep 150 ms, then restore the previous text if there was any.
- [x] 14. Global shortcut: on startup, register the stored shortcut. If registration fails (e.g. Handy still holds ⌥Space), log it and show the error in settings.

### M4: Overlay
- [x] 15. `overlay.rs`:
  - On setup, convert the `overlay` window to a non-activating `NSPanel` with `tauri-nspanel`:
    - style mask: non-activating panel
    - window level above status windows
    - collection behavior: `canJoinAllSpaces` + `fullScreenAuxiliary`
    - ignores mouse events
  - `show_overlay(app)`: position at the bottom-center of `primary_monitor().work_area()`, 48 px above the bottom, then order front **without** making it key.
  - `hide_overlay`.
  - Emit `overlay-state { state: "recording" | "transcribing" | "error", message?: string }`.
  - Acceptance: the focused app keeps keyboard focus while the overlay is visible.
- [x] 16. `overlay.html` + `src/overlay.ts` + `src/overlay.css`:
  - Pill: full size, `--bg` at 92% opacity, 1px `--border`, radius 22px, transparent page background.
  - Recording: 24 bars, 3px wide, 2px gap, `--accent`, rounded. Keep a rolling buffer of the last 24 `level` values. Height = `max(2px, sqrt(rms) * k)`, clamped to 28px. Pick `k` so normal speech fills about 60–80% of the height.
  - Transcribing: a 12px `--running` spinner plus "Transcribing…" (12px, `--text-dim`).
  - Error: the message in `--error`.
  - Respect `prefers-reduced-motion` for the spinner.

### M5: Settings page
- [x] 17. Tauri commands (all `Result<_, String>`):
  - `get_settings`
  - `list_microphones`
  - `set_microphone(name: Option<String>)`
  - `set_shortcut(accel: String)`: unregister the old shortcut, register the new one; on failure, re-register the old one and return an error.
  - `pause_shortcut` / `resume_shortcut` (used during key capture)
  - `model_status -> { downloaded, loaded, loading, downloading }`
  - `set_unload_after(minutes: Option<u32>)`: rejects any value other than `None`/5/15/30/60.
  - `download_model`, `cancel_download`, `delete_model`
  - `shortcut_error -> Option<String>`
- [x] 18. `index.html` + `src/main.ts` + `src/settings.css`, styled with the dark-theme tokens, 13px base. Sections, each with an 11px uppercase label:
  1. **Model**:
     - Shows "Parakeet Unified English 0.6B · Q8_0 · 731 MB" and its status (Not downloaded / Downloading / Loading / Ready / Unloaded (idle)).
     - "Unload when idle" `<select>`: Never / 5 min / 15 min / 30 min / 60 min (default 5 min). Saves on change via `set_unload_after`.
     - Download button → progress bar (`--accent`), "x / 731 MB", and a Cancel button.
     - When ready: a Delete button with a `confirm()` prompt.
  2. **Microphone**:
     - A `<select>` with "System default" first, then the device names.
     - Refresh the list when the window gains focus.
     - Save on change.
  3. **Shortcut**:
     - Shows the current shortcut with Mac symbols (`Alt→⌥`, `Super/Cmd→⌘`, `Control→⌃`, `Shift→⇧`, e.g. "⌥ Space").
     - "Change" → calls `pause_shortcut` and shows "Press keys…". Capture the next `keydown`:
       - Build the accelerator from the modifier flags plus `e.code` (`KeyA→A`, `Digit1→1`, `Space`, `F1–F20`).
       - Require at least one modifier unless the key is F1–F20.
       - Esc cancels capture.
     - Call `set_shortcut`, then `resume_shortcut`. Show any error inline in `--error`.
     - "Reset to default" button.
  4. **Startup**: a "Launch at login" toggle wired to the autostart plugin (`isEnabled` / `enable` / `disable`).
  5. **Permissions**:
     - Rows for Microphone and Accessibility: status ("Granted" in `--ok` / "Not granted" in `--error`) and a "Grant" button (`requestMicrophonePermission` / `requestAccessibilityPermission`).
     - Re-check when the window gains focus.
     - Small note: "Accessibility is needed to paste text into other apps."

  Footer: "Utter v<version>" in `--text-faint`.

### M6: Verify
- [x] 19. Run `cargo clippy -- -D warnings` and `bunx tsc --noEmit` and fix all issues. Add unit tests in Rust for: the accelerator display mapping (if done in Rust), RMS calculation, the resampling length (48 kHz × 1 s → ~16000 samples), and the transcript post-processing (trim, empty → none, trailing space).
- [ ] 20. Run `bun tauri build`, copy `Utter.app` to `/Applications`, launch it, grant permissions, and run through every acceptance criterion manually. Quit Handy first.

## Acceptance criteria
- `cargo clippy -- -D warnings`, `cargo test`, and `bunx tsc --noEmit` all pass.
- `bun tauri build` produces `Utter.app` with the custom icon. There is no Dock icon while running, and a **white** tray icon appears in the menu bar.
- First launch with no model: the settings window opens. Download shows progress, completes, verifies the SHA-256, and the status becomes Ready.
- In TextEdit: press ⌥Space → the overlay appears bottom-center with moving bars, the tray icon shows the tangerine dot, and TextEdit keeps focus. Speak "Hello world, this is a test." Press ⌥Space → "Transcribing…" → the text, with a trailing space, is pasted into TextEdit within ~1 s for a 5 s clip. The previous clipboard text is restored afterwards.
- Pressing Esc while recording hides the overlay and pastes nothing. Esc works normally in other apps when not recording.
- After selecting the HyperX mic in settings, recording uses it. With it unplugged, recording falls back to the system default without an error.
- Changing the shortcut to e.g. ⌃⌥D works immediately and persists across relaunch. An invalid or taken shortcut shows an error and keeps the old one.
- Turning on "Launch at login" adds Utter to System Settings → General → Login Items, and it starts at login.
- Closing the settings window only hides it. "Quit Utter" in the tray quits. Launching the app a second time opens settings.
- A 2-minute recording transcribes successfully.
- With "Unload when idle" set to 5 min: after 5+ min idle, the settings status shows "Unloaded (idle)" and Utter's memory in Activity Monitor drops noticeably. The next ⌥Space starts recording instantly, and the transcript is pasted correctly (at most ~3 s extra wait on stop). With "Never", the model stays loaded.

## Out of scope
- Model picker, other models/quantizations, streaming/live preview, history, post-processing/LLM cleanup, sounds, multi-language, translation.
- Code signing, notarization, auto-update, Intel/Windows/Linux support.
- Hold-to-talk mode, a configurable overlay position, light theme.
- Any dependency not listed above.

## Notes / risks
- **Focus stealing:** a normal Tauri window `show()` makes the app key, and then ⌘V would go to the overlay instead of the target app. The overlay MUST be a non-activating NSPanel and must never call `set_focus`.
- **Accessibility and rebuilds:** unsigned/ad-hoc builds change their code signature on every rebuild, so macOS may silently drop the Accessibility grant. Remove and re-add Utter in System Settings → Privacy & Security → Accessibility after rebuilding. Under `bun tauri dev` the permission may be attributed to the terminal. Test pasting with the built `.app`.
- **Shortcut conflict:** Handy currently registers ⌥Space. Quit Handy (and disable its autostart) before testing, or registration fails.
- `cpal::Stream` is `!Send` on macOS. Keep it on its own thread (Task 10).
- The model file is mmapped. Delete it only after dropping the session, and download to `.part` then rename.
- Clipboard restore covers text only (accepted).
- If `transcribe-cpp` APIs differ from the snippet above, follow the crate docs (`cargo doc --open -p transcribe-cpp`) and its `examples/`. Handy's source (`github.com/cjpais/Handy/src-tauri`) shows a working integration of transcribe-cpp, tauri-nspanel, global shortcuts, and pasting.
