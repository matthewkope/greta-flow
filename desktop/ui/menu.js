const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const GF = (window.GF = window.GF || {});
GF.views = GF.views || {};
GF.el = GF.el || ((tag, attrs, ...kids) => {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === undefined || v === null) continue;
    if (k === "text") node.textContent = v;
    else if (k === "class") node.className = v;
    else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v);
  }
  for (const kid of kids.flat()) if (kid !== null && kid !== undefined) node.append(kid);
  return node;
});
GF.call = GF.call || ((cmd, args) => invoke(cmd, args));
GF.isMissing = GF.isMissing || ((e) => /not (found|allowed)|unknown command/i.test(String(e)));
GF.unavailable = GF.unavailable || ((root, text) => root.replaceChildren(GF.el("div", { class: "empty", text })));
GF.say = GF.say || ((node, text, bad) => {
  node.textContent = text;
  node.classList.toggle("error", !!bad);
});

let currentView = "main";
function showView(name) {
  const prev = GF.views[currentView];
  if (prev && prev.hide) prev.hide();
  currentView = GF.views[name] ? name : "main";
  for (const section of document.querySelectorAll("section[id^='view-']")) {
    section.hidden = section.id !== "view-" + currentView;
  }
  const view = GF.views[currentView];
  if (view) view.show();
}

function row(opts) {
  const el = document.createElement("div");
  el.className = "item" + (opts.active ? " active" : "");
  const bullet = document.createElement("span");
  bullet.className = "bullet";
  // Selected/active rows are marked with a sideways triangle.
  bullet.textContent = opts.active ? "▸" : "•";
  const label = document.createElement("span");
  label.className = "label";
  label.textContent = opts.label;
  el.appendChild(bullet);
  el.appendChild(label);
  if (opts.note) {
    const note = document.createElement("span");
    note.className = "note";
    note.textContent = opts.note;
    el.appendChild(note);
  }
  el.onclick = opts.onclick;
  return el;
}

async function render() {
  const s = await invoke("menu_state");
  const status = document.getElementById("status");
  status.textContent = s.status;
  status.className = "status" + (s.has_engine ? "" : " warn");

  const models = document.getElementById("models");
  models.innerHTML = "";
  for (const m of s.models) {
    models.appendChild(row({
      label: m.label, active: m.active, note: m.note,
      onclick: () => invoke("menu_choose_model", { name: m.id }),
    }));
  }
  if (s.custom_model) {
    models.appendChild(row({ label: s.custom_model, active: true, note: "", onclick: () => {} }));
  }

  const hotkeys = document.getElementById("hotkeys");
  hotkeys.innerHTML = "";
  const removable = s.shortcuts.length > 1;
  for (const accel of s.shortcuts) {
    const label = accel === "Fn" ? "Fn  (🌐 Globe key)" : accel;
    const el = document.createElement("div");
    el.className = "item active";
    const bullet = document.createElement("span");
    bullet.className = "bullet"; bullet.textContent = "•";
    const lab = document.createElement("span");
    lab.className = "label"; lab.textContent = label;
    el.appendChild(bullet); el.appendChild(lab);
    // Click the row to re-record this shortcut as anything you like.
    el.onclick = () => invoke("menu_change_hotkey", { accel }).catch(() => {});
    if (removable) {
      const note = document.createElement("span");
      note.className = "note"; note.textContent = "remove";
      note.onclick = (ev) => {
        ev.stopPropagation();
        invoke("menu_remove_hotkey", { accel }).catch(() => {});
      };
      el.appendChild(note);
    }
    hotkeys.appendChild(el);
  }
  // "Add shortcut" is disabled once the maximum is reached.
  const add = document.getElementById("hotkey-record");
  add.classList.toggle("disabled", !s.can_add_shortcut);
  add.querySelector(".label").textContent =
    s.can_add_shortcut ? "add shortcut…" : "max 3 shortcuts";

  const recent = document.getElementById("recent");
  recent.innerHTML = "";
  if (!s.recent.length) {
    const e = document.createElement("div");
    e.className = "empty";
    e.textContent = "no dictations yet";
    recent.appendChild(e);
  } else {
    s.recent.forEach((text, i) => {
      const short = text.length > 40 ? text.slice(0, 40) + "…" : text;
      recent.appendChild(row({
        label: short, active: false, note: "copy",
        onclick: (e) => {
          invoke("menu_copy_recent", { index: i });
          const n = e.currentTarget.querySelector(".note");
          if (n) { n.textContent = "copied!"; setTimeout(() => { n.textContent = "copy"; }, 1000); }
        },
      }));
    });
  }
  // Offer "clear history" only when there's something to clear.
  document.getElementById("recent-clear").style.display =
    s.recent.length ? "" : "none";
}

document.getElementById("model-file").onclick = () => invoke("menu_model_from_file");
document.getElementById("model-custom").onclick = () => invoke("open_models_page");
document.getElementById("hotkey-record").onclick = (e) => {
  if (e.currentTarget.classList.contains("disabled")) return;
  invoke("menu_record_hotkey");
};
document.getElementById("setup").onclick = () => invoke("menu_open_setup");
document.getElementById("reload").onclick = () => invoke("menu_reload_config");
document.getElementById("recent-clear").onclick = () => invoke("menu_clear_history");
document.getElementById("quit").onclick = () => invoke("menu_quit");
for (const el of document.querySelectorAll("[data-view]")) {
  el.onclick = () => showView(el.dataset.view);
}
for (const el of document.querySelectorAll(".back")) el.onclick = () => showView("main");

listen("menu-updated", () => {
  render();
  const view = GF.views[currentView];
  if (view) view.refresh();
});
render();
