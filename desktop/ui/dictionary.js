// Personal Dictionary and Spoken Shortcuts panels for the main window.
//
// Loaded by menu.html after menu.js (which provides or reuses GF.el / GF.call /
// GF.isMissing / GF.unavailable / GF.say). Registers GF.views.dictionary;
// spoken.js registers GF.views.spoken using GF.personal from this file.
//
// Backend contract (desktop/src-tauri/src/dictionary_commands.rs):
//   dictionary_state() -> { dictionary: Entry[], spoken_shortcuts: Shortcut[] }
//   dictionary_save({ dictionary, spokenShortcuts }) -> same shape, normalized,
//     or rejects with a readable validation message.
//   Entry    = { canonical: string, variants: string[], enabled: bool }
//   Shortcut = { phrase: string, text: string, enabled: bool }
(() => {
  const GF = window.GF;

  // Panel-local layout; everything else reuses menu.css classes.
  const style = document.createElement("style");
  style.textContent = `
    .personal-editor { border: 2px solid var(--ink, #fff); border-radius: 4px; padding: 8px 10px; margin: 4px 0 8px; }
    .personal-field { display: flex; flex-direction: column; gap: 2px; margin: 0 0 6px; }
    .personal-field .field { width: 100%; min-width: 0; font: inherit; color: inherit; }
    .personal-field textarea.field { resize: vertical; }
    .personal-list .item .label { overflow-wrap: anywhere; }
    .personal-list .empty { color: var(--dim, #888); padding: 4px 6px; }
  `;
  document.head.appendChild(style);

  // Shared list editor: rows of entries, one inline editor at a time.
  GF.personal = (opts) => {
    let items = [];
    let editing = null; // index, or "new"
    let root = null;
    let msg = null;

    async function load() {
      const s = await GF.call("dictionary_state");
      items = s[opts.key];
      return s;
    }

    // Save one list without clobbering the other: re-read, replace, write.
    async function commit(next) {
      try {
        const s = await GF.call("dictionary_state");
        s[opts.key] = next;
        const saved = await GF.call("dictionary_save", {
          dictionary: s.dictionary,
          spokenShortcuts: s.spoken_shortcuts,
        });
        items = saved[opts.key];
        editing = null;
        draw();
        GF.say(msg, "Saved. Applies to your next dictation.");
        return true;
      } catch (e) {
        GF.say(msg, String(e), true);
        return false;
      }
    }

    function row(item, i) {
      const unset = opts.unset(item);
      const note = !item.enabled ? "off" : unset ? "needs text" : "on";
      return GF.el(
        "div",
        { class: "item" + (item.enabled ? " active" : ""), onclick: () => { editing = i; draw(); } },
        GF.el("span", { class: "bullet", text: item.enabled ? "▸" : "•" }),
        GF.el("span", { class: "label", text: opts.summary(item) }),
        GF.el("span", {
          class: unset && item.enabled ? "problem" : "note",
          text: note,
          title: item.enabled ? "Turn off" : "Turn on",
          onclick: (ev) => {
            ev.stopPropagation();
            const next = items.map((x, j) => (j === i ? { ...x, enabled: !x.enabled } : x));
            commit(next);
          },
        }),
      );
    }

    function editor(i) {
      const item = i === "new" ? opts.blank() : items[i];
      const inputs = opts.fields.map((f) =>
        GF.el(f.multiline ? "textarea" : "input", {
          class: "field",
          type: f.multiline ? undefined : "text",
          rows: f.multiline ? 2 : undefined,
          placeholder: f.placeholder(item),
          spellcheck: "false",
          autocapitalize: "off",
          "aria-label": f.label,
        }),
      );
      inputs.forEach((input, n) => { input.value = opts.fields[n].get(item); });
      const save = () => {
        let next = { ...item };
        opts.fields.forEach((f, n) => { next = f.set(next, inputs[n].value); });
        const list = items.slice();
        if (i === "new") list.push(next);
        else list[i] = next;
        commit(list);
      };
      inputs.forEach((input) =>
        input.addEventListener("keydown", (ev) => {
          if (ev.key === "Enter" && (!input.matches("textarea") || ev.metaKey)) {
            ev.preventDefault();
            save();
          } else if (ev.key === "Escape") {
            editing = null;
            draw();
          }
        }),
      );
      const form = GF.el(
        "div",
        { class: "personal-editor" },
        opts.fields.map((f, n) =>
          GF.el("label", { class: "personal-field" },
            GF.el("span", { class: "small", text: f.label }),
            inputs[n]),
        ),
        GF.el("div", { class: "row" },
          GF.el("button", { class: "btn", text: "save", onclick: save }),
          GF.el("button", { class: "btn", text: "cancel", onclick: () => { editing = null; draw(); } }),
          i === "new" ? null : GF.el("button", {
            class: "btn danger",
            text: "delete",
            onclick: () => commit(items.filter((_, j) => j !== i)),
          }),
        ),
      );
      setTimeout(() => inputs[0].focus(), 0);
      return form;
    }

    function draw() {
      if (!root) return;
      const list = GF.el("div", { class: "personal-list" });
      if (!items.length && editing === null) {
        list.append(GF.el("div", { class: "empty", text: opts.empty }));
      }
      items.forEach((item, i) => list.append(editing === i ? editor(i) : row(item, i)));
      if (editing === "new") list.append(editor("new"));
      const actions = [
        GF.el("div", {
          class: "item",
          onclick: () => { editing = "new"; draw(); },
        }, GF.el("span", { class: "bullet", text: "+" }), GF.el("span", { class: "label", text: opts.add })),
        ...(opts.extraActions ? opts.extraActions(items, commit) : []),
      ];
      root.replaceChildren(
        list,
        ...(editing === null ? actions : []),
        msg,
        GF.el("p", { class: "small", text: opts.footnote }),
      );
    }

    async function show() {
      root = document.getElementById(opts.panel);
      msg = msg || GF.el("div", { class: "msg", role: "status" });
      try {
        await load();
        draw();
      } catch (e) {
        if (GF.isMissing(e)) GF.unavailable(root, "This section isn't available in this version yet.");
        else GF.say(msg, String(e), true);
      }
    }

    return {
      show,
      async refresh() {
        // Don't discard an open editor when another part of the app refreshes.
        if (editing === null) await show();
      },
      hide() { editing = null; },
    };
  };

  GF.views.dictionary = GF.personal({
    key: "dictionary",
    panel: "dictionary-panel",
    empty: "no words yet",
    add: "add a word or name…",
    footnote:
      "When a transcript contains a “heard as” phrase as whole words, Greta types your spelling instead. " +
      "This corrects text after recognition; it doesn't retrain the speech model.",
    blank: () => ({ canonical: "", variants: [], enabled: true }),
    unset: () => false,
    summary: (e) => (e.variants.length ? e.variants.join(", ") + "  →  " : "") + e.canonical,
    fields: [
      {
        label: "correct spelling",
        placeholder: () => "e.g. Ada Lovelace",
        get: (e) => e.canonical,
        set: (e, v) => ({ ...e, canonical: v }),
      },
      {
        label: "heard as (separate with commas)",
        placeholder: () => "e.g. Ada Love Lace, Ada Lovelase",
        get: (e) => e.variants.join(", "),
        set: (e, v) => ({ ...e, variants: v.split(",").map((s) => s.trim()).filter(Boolean) }),
      },
    ],
  });
})();
