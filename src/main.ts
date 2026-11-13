import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { ChevronsUpDown, createIcons, Download, Trash2 } from "lucide";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
  requestAccessibilityPermission,
} from "tauri-plugin-macos-permissions-api";
import { acceleratorFromEvent, DEFAULT_SHORTCUT, formatShortcut } from "./shortcut";
import "./theme.css";
import "./settings.css";

interface Settings {
  shortcut: string;
  microphone: string | null;
  unload_after_min: number | null;
}

interface ModelStatus {
  downloaded: boolean;
  loaded: boolean;
  loading: boolean;
  downloading: boolean;
  error: string | null;
}

interface Progress {
  downloaded: number;
  total: number;
}

type Tone = "ok" | "error" | "running" | "dim";

const MODEL_BYTES = 731_357_568;

function $<T extends HTMLElement>(selector: string): T {
  const el = document.querySelector<T>(selector);
  if (!el) throw new Error(`Missing element ${selector}`);
  return el;
}

/** Megabytes, as in "731 MB". */
function megabytes(bytes: number): number {
  return Math.round(bytes / 1_000_000);
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Show `text` in an inline error line, or hide the line when it is empty. */
function showError(el: HTMLElement, text: string | null): void {
  el.textContent = text ?? "";
  el.hidden = !text;
}

function setStatus(el: HTMLElement, text: string, tone: Tone): void {
  el.textContent = text;
  el.dataset.tone = tone;
}

// ---------------------------------------------------------------- Model

const modelStatusEl = $("#model-status");
const modelErrorEl = $("#model-error");
const downloadBtn = $<HTMLButtonElement>("#download");
const deleteBtn = $<HTMLButtonElement>("#delete");
const progressEl = $("#progress");
const progressFill = $("#progress-fill");
const progressText = $("#progress-text");
const progressTrack = $(".progress-track");
const unloadSelect = $<HTMLSelectElement>("#unload");
const confirmDialog = $<HTMLDialogElement>("#confirm-delete");

/** An error from the last download or delete. Kept until the next attempt. */
let modelActionError: string | null = null;

function modelLabel(s: ModelStatus): [string, Tone] {
  if (s.downloading) return ["Downloading", "running"];
  if (!s.downloaded) return ["Not downloaded", "dim"];
  if (s.loading) return ["Loading", "running"];
  if (s.loaded) return ["Ready", "ok"];
  return ["Unloaded (idle)", "dim"];
}

function renderModel(s: ModelStatus): void {
  setStatus(modelStatusEl, ...modelLabel(s));
  downloadBtn.hidden = s.downloaded || s.downloading;
  progressEl.hidden = !s.downloading;
  deleteBtn.hidden = !s.downloaded || s.downloading;
  showError(modelErrorEl, modelActionError ?? s.error);
}

function renderProgress({ downloaded, total }: Progress): void {
  const percent = total > 0 ? Math.min(100, (downloaded / total) * 100) : 0;
  progressFill.style.width = `${percent}%`;
  progressTrack.setAttribute("aria-valuenow", String(Math.round(percent)));
  progressText.textContent = `${megabytes(downloaded)} / ${megabytes(total)} MB`;
}

async function refreshModel(): Promise<void> {
  renderModel(await invoke<ModelStatus>("model_status"));
}

async function downloadModel(): Promise<void> {
  modelActionError = null;
  renderProgress({ downloaded: 0, total: MODEL_BYTES });
  try {
    // Resolves when the download ends. Progress arrives as events meanwhile.
    await invoke<boolean>("download_model");
  } catch (e) {
    modelActionError = message(e);
  }
  await refreshModel();
}

async function deleteModel(): Promise<void> {
  modelActionError = null;
  try {
    await invoke("delete_model");
  } catch (e) {
    modelActionError = message(e);
  }
  await refreshModel();
}

downloadBtn.addEventListener("click", () => {
  // Show the progress bar at once, before the first status event arrives.
  renderModel({ downloaded: false, loaded: false, loading: false, downloading: true, error: null });
  void downloadModel();
});
$("#cancel-download").addEventListener("click", () => void invoke("cancel_download"));
// `window.confirm()` does nothing in a Tauri webview on macOS, so ask with a dialog.
deleteBtn.addEventListener("click", () => confirmDialog.showModal());
$("#confirm-delete-cancel").addEventListener("click", () => confirmDialog.close());
$("#confirm-delete-ok").addEventListener("click", () => {
  confirmDialog.close();
  void deleteModel();
});

unloadSelect.addEventListener("change", () => {
  const minutes = unloadSelect.value === "never" ? null : Number(unloadSelect.value);
  invoke("set_unload_after", { minutes }).then(
    () => showError(modelErrorEl, null),
    (e) => showError(modelErrorEl, message(e)),
  );
});

// ---------------------------------------------------------------- Microphone

const micSelect = $<HTMLSelectElement>("#microphone");
const micErrorEl = $("#microphone-error");
/** The saved device name. Empty means the system default. */
let savedMicrophone = "";

async function refreshMicrophones(): Promise<void> {
  const names = await invoke<string[]>("list_microphones");
  const options = [new Option("System default", "")];
  for (const name of names) options.push(new Option(name, name));
  // Keep the saved device selectable while it is unplugged.
  if (savedMicrophone && !names.includes(savedMicrophone)) {
    options.push(new Option(`${savedMicrophone} (not connected)`, savedMicrophone));
  }
  micSelect.replaceChildren(...options);
  micSelect.value = savedMicrophone;
}

micSelect.addEventListener("change", () => {
  const name = micSelect.value;
  invoke("set_microphone", { name: name || null }).then(
    () => {
      savedMicrophone = name;
      showError(micErrorEl, null);
    },
    (e) => {
      micSelect.value = savedMicrophone;
      showError(micErrorEl, message(e));
    },
  );
});

// ---------------------------------------------------------------- Shortcut

const shortcutEl = $("#shortcut");
const shortcutErrorEl = $("#shortcut-error");
const changeBtn = $<HTMLButtonElement>("#change-shortcut");
const resetBtn = $<HTMLButtonElement>("#reset-shortcut");
let shortcut = DEFAULT_SHORTCUT;
let capturing = false;

function renderShortcut(): void {
  shortcutEl.textContent = capturing ? "Press keys…" : formatShortcut(shortcut);
  shortcutEl.classList.toggle("capturing", capturing);
  changeBtn.textContent = capturing ? "Cancel" : "Change";
  resetBtn.disabled = capturing || shortcut === DEFAULT_SHORTCUT;
}

/** Show why the stored shortcut is not registered, if it is not. */
async function refreshShortcutError(): Promise<void> {
  if (capturing) return;
  showError(shortcutErrorEl, await invoke<string | null>("shortcut_error"));
}

async function applyShortcut(accel: string): Promise<void> {
  try {
    await invoke("set_shortcut", { accel });
    shortcut = accel;
    showError(shortcutErrorEl, null);
  } catch (e) {
    // The old shortcut stays in place.
    showError(shortcutErrorEl, message(e));
  }
  renderShortcut();
}

async function startCapture(): Promise<void> {
  // Release the global shortcut, so pressing it reaches this window instead.
  await invoke("pause_shortcut");
  capturing = true;
  showError(shortcutErrorEl, null);
  renderShortcut();
}

async function endCapture(accel: string | null): Promise<void> {
  if (!capturing) return;
  capturing = false;
  renderShortcut();
  let failed = false;
  if (accel) {
    await applyShortcut(accel);
    failed = !shortcutErrorEl.hidden;
  }
  await invoke("resume_shortcut");
  if (!failed) await refreshShortcutError();
}

changeBtn.addEventListener("click", () => {
  void (capturing ? endCapture(null) : startCapture());
  changeBtn.blur();
});
resetBtn.addEventListener("click", () => void applyShortcut(DEFAULT_SHORTCUT));

window.addEventListener(
  "keydown",
  (e) => {
    if (!capturing) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "Escape") {
      void endCapture(null);
      return;
    }
    const capture = acceleratorFromEvent(e);
    if (!capture) return;
    if ("error" in capture) {
      // Keep waiting for a valid combination.
      showError(shortcutErrorEl, capture.error);
      return;
    }
    void endCapture(capture.accel);
  },
  true,
);
// Stop capturing if the user clicks away to another app.
window.addEventListener("blur", () => void endCapture(null));

// ---------------------------------------------------------------- Startup

const autostartToggle = $<HTMLInputElement>("#autostart");
const autostartErrorEl = $("#autostart-error");

async function refreshAutostart(): Promise<void> {
  autostartToggle.checked = await isEnabled();
}

autostartToggle.addEventListener("change", () => {
  const wanted = autostartToggle.checked;
  (wanted ? enable() : disable()).then(
    () => showError(autostartErrorEl, null),
    (e) => {
      autostartToggle.checked = !wanted;
      showError(autostartErrorEl, message(e));
    },
  );
});

// ---------------------------------------------------------------- Permissions

const micPermissionEl = $("#mic-permission");
const axPermissionEl = $("#ax-permission");
const grantMicBtn = $<HTMLButtonElement>("#grant-mic");
const grantAxBtn = $<HTMLButtonElement>("#grant-ax");
let allGranted = false;

function renderPermission(statusEl: HTMLElement, button: HTMLElement, granted: boolean): void {
  setStatus(statusEl, granted ? "Granted" : "Not granted", granted ? "ok" : "error");
  button.hidden = granted;
}

async function refreshPermissions(): Promise<void> {
  const [mic, ax] = await Promise.all([checkMicrophonePermission(), checkAccessibilityPermission()]);
  renderPermission(micPermissionEl, grantMicBtn, mic);
  renderPermission(axPermissionEl, grantAxBtn, ax);
  allGranted = mic && ax;
}

grantMicBtn.addEventListener("click", () => {
  void invoke("request_microphone").then(refreshPermissions);
});
grantAxBtn.addEventListener("click", () => {
  void requestAccessibilityPermission().then(refreshPermissions);
});

// ---------------------------------------------------------------- Start

/** Re-read everything that can change outside this window. */
function refreshExternal(): void {
  void refreshMicrophones();
  void refreshPermissions();
  void refreshModel();
  void refreshShortcutError();
  void refreshAutostart();
}

async function init(): Promise<void> {
  createIcons({ icons: { ChevronsUpDown, Download, Trash2 }, attrs: { class: "icon" } });

  void listen<ModelStatus>("model-status", (e) => renderModel(e.payload));
  void listen<Progress>("download-progress", (e) => renderProgress(e.payload));
  void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (focused) refreshExternal();
  });
  // A grant in System Settings does not always refocus this window, so
  // poll while a permission is missing and the window is showing.
  setInterval(() => {
    if (!allGranted && document.visibilityState === "visible") void refreshPermissions();
  }, 2000);

  const settings = await invoke<Settings>("get_settings");
  shortcut = settings.shortcut;
  savedMicrophone = settings.microphone ?? "";
  unloadSelect.value = settings.unload_after_min === null ? "never" : String(settings.unload_after_min);
  renderShortcut();
  refreshExternal();

  $("#version").textContent = `Utter v${await getVersion()}`;
}

void init();
