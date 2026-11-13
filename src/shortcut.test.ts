import { expect, test } from "bun:test";
import { acceleratorFromEvent, formatShortcut, keyFromCode, type KeyInput } from "./shortcut";

const key = (code: string, mods: Partial<KeyInput> = {}): KeyInput => ({
  code,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...mods,
});

test("formats accelerators with Mac symbols", () => {
  expect(formatShortcut("Alt+Space")).toBe("⌥ Space");
  expect(formatShortcut("Control+Alt+D")).toBe("⌃ ⌥ D");
  expect(formatShortcut("Super+Shift+1")).toBe("⌘ ⇧ 1");
  expect(formatShortcut("Cmd+K")).toBe("⌘ K");
  expect(formatShortcut("F5")).toBe("F5");
});

test("maps key codes to accelerator keys", () => {
  expect(keyFromCode("KeyA")).toBe("A");
  expect(keyFromCode("Digit1")).toBe("1");
  expect(keyFromCode("Space")).toBe("Space");
  expect(keyFromCode("F1")).toBe("F1");
  expect(keyFromCode("F20")).toBe("F20");
  expect(keyFromCode("F21")).toBeNull();
  expect(keyFromCode("F0")).toBeNull();
  expect(keyFromCode("Enter")).toBeNull();
  expect(keyFromCode("Escape")).toBeNull();
});

test("builds an accelerator from modifiers and a key", () => {
  expect(acceleratorFromEvent(key("Space", { altKey: true }))).toEqual({ accel: "Alt+Space" });
  expect(acceleratorFromEvent(key("KeyD", { ctrlKey: true, altKey: true }))).toEqual({
    accel: "Control+Alt+D",
  });
  expect(
    acceleratorFromEvent(key("Digit1", { metaKey: true, shiftKey: true })),
  ).toEqual({ accel: "Shift+Super+1" });
});

test("function keys need no modifier", () => {
  expect(acceleratorFromEvent(key("F5"))).toEqual({ accel: "F5" });
});

test("other keys need a modifier", () => {
  expect(acceleratorFromEvent(key("KeyA"))).toHaveProperty("error");
  expect(acceleratorFromEvent(key("Space"))).toHaveProperty("error");
});

test("keeps waiting while only modifiers are down", () => {
  expect(acceleratorFromEvent(key("AltLeft", { altKey: true }))).toBeNull();
  expect(acceleratorFromEvent(key("MetaRight", { metaKey: true }))).toBeNull();
});

test("rejects unsupported keys", () => {
  expect(acceleratorFromEvent(key("Enter", { altKey: true }))).toHaveProperty("error");
});
