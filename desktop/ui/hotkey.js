const { invoke } = window.__TAURI__.core;
const combo = document.getElementById("combo");
const error = document.getElementById("error");
const note = document.getElementById("note");
const save = document.getElementById("save");
let accel = null;

const SYMBOLS = { Cmd: "⌘", Ctrl: "⌃", Alt: "⌥", Shift: "⇧" };
const KEYMAP = {
  Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace",
  Delete: "Delete", ArrowUp: "Up", ArrowDown: "Down",
  ArrowLeft: "Left", ArrowRight: "Right",
  Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/",
  Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]",
  Backslash: "\\", Backquote: "`",
};

function keyName(e) {
  const c = e.code;
  if (c.startsWith("Key")) return c.slice(3);
  if (c.startsWith("Digit")) return c.slice(5);
  if (/^F\d+$/.test(c)) return c;
  return KEYMAP[c] || null;
}

window.addEventListener("keydown", (e) => {
  e.preventDefault();
  if (e.key === "Escape") { invoke("cancel_custom_hotkey"); return; }
  error.textContent = "";
  note.textContent = "";
  const mods = [];
  if (e.metaKey) mods.push("Cmd");
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  const key = keyName(e);
  if (!key) {
    // Modifier-only so far — show progress. Releasing it alone (keyup
    // handler below) offers the bare modifier itself as the hotkey.
    combo.textContent = mods.length
      ? mods.map((m) => SYMBOLS[m]).join("") + " …" : "…";
    save.disabled = true; accel = null;
    return;
  }
  // A single key (no modifier) is allowed — handy for a dedicated push-to-talk
  // key. Warn that a bare letter/digit/punctuation key gets captured globally
  // so it won't type normally while it's your hotkey (F-keys are usually safe).
  accel = [...mods, key].join("+");
  combo.textContent = (mods.length ? mods.map((m) => SYMBOLS[m]).join("") + " " : "") + key;
  save.disabled = false;
  if (!mods.length && !/^F\d+$/.test(key)) {
    note.textContent = `“${key}” will be a dedicated hotkey — it won’t type normally while set`;
  }
});

// A modifier pressed and released on its own (no other key, no other
// modifiers still held) becomes a bare-modifier hotkey — like Fn, it's
// watched by a low-level listener, so ⌃/⌘/⌥/⇧ alone work as push-to-talk.
const MOD_CODES = {
  ControlLeft: "Ctrl", ControlRight: "Ctrl",
  MetaLeft: "Cmd", MetaRight: "Cmd",
  AltLeft: "Alt", AltRight: "Alt",
  ShiftLeft: "Shift", ShiftRight: "Shift",
};
window.addEventListener("keyup", (e) => {
  const mod = MOD_CODES[e.code];
  if (!mod || accel) return; // not a modifier, or a combo was already captured
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return; // others still held
  accel = mod;
  combo.textContent = SYMBOLS[mod];
  save.disabled = false;
  note.textContent = `${SYMBOLS[mod]} held alone will be push-to-talk — combos using it still work normally`;
});

save.onclick = () => {
  if (!accel) return;
  invoke("apply_custom_hotkey", { accel }).catch((err) => {
    error.textContent = String(err);
  });
};
document.getElementById("fn").onclick = () =>
  invoke("apply_custom_hotkey", { accel: "Fn" }).catch((err) => {
    error.textContent = String(err);
  });
document.getElementById("cancel").onclick = () =>
  invoke("cancel_custom_hotkey");
document.getElementById("remove").onclick = () =>
  invoke("remove_pending_hotkey");

// If we were opened to change an existing shortcut, adjust the wording
// and reveal the Remove button.
invoke("hotkey_replace_target").then((target) => {
  if (!target) return;
  const shown = target === "Fn" ? "Fn 🌐" : target;
  document.getElementById("prompt").textContent =
    "Press a new key or combination to replace " + shown;
  save.textContent = "Save Shortcut";
  document.getElementById("remove").style.display = "";
  document.title = "Change Shortcut — Greta Flow";
});
