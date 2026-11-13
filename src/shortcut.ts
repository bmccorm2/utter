/** Shortcut display and capture helpers. No DOM or Tauri access, so they are easy to test. */

export const DEFAULT_SHORTCUT = "Alt+Space";

const SYMBOLS: Record<string, string> = {
  alt: "⌥",
  option: "⌥",
  super: "⌘",
  cmd: "⌘",
  command: "⌘",
  meta: "⌘",
  control: "⌃",
  ctrl: "⌃",
  shift: "⇧",
};

/** "Alt+Space" → "⌥ Space", "Control+Alt+D" → "⌃ ⌥ D". */
export function formatShortcut(accel: string): string {
  return accel
    .split("+")
    .map((part) => SYMBOLS[part.toLowerCase()] ?? part)
    .join(" ");
}

/** The accelerator key name for a `KeyboardEvent.code`, or null if the key is not supported. */
export function keyFromCode(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  if (code === "Space") return "Space";
  const fn = /^F([1-9]|1[0-9]|20)$/.exec(code);
  if (fn) return code;
  return null;
}

const MODIFIER_CODES = /^(Alt|Control|Meta|Shift|OS)(Left|Right)$/;

export interface KeyInput {
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

export type Capture = { accel: string } | { error: string } | null;

/**
 * Build an accelerator from a keydown event.
 * Returns null while only modifier keys are down, so capture keeps waiting.
 */
export function acceleratorFromEvent(e: KeyInput): Capture {
  if (MODIFIER_CODES.test(e.code)) return null;
  const key = keyFromCode(e.code);
  if (!key) return { error: "Use a letter, a digit, Space or F1–F20." };

  const modifiers: string[] = [];
  if (e.ctrlKey) modifiers.push("Control");
  if (e.altKey) modifiers.push("Alt");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey) modifiers.push("Super");

  const isFunctionKey = /^F\d+$/.test(key);
  if (modifiers.length === 0 && !isFunctionKey) {
    return { error: "Add at least one modifier key (⌃, ⌥, ⇧ or ⌘)." };
  }
  return { accel: [...modifiers, key].join("+") };
}
