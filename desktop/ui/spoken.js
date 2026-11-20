// Spoken Shortcuts panel. Uses GF.personal from dictionary.js (loaded first).
(() => {
  const GF = window.GF;
  if (!GF.personal) return; // boot.js shows the "not available" fallback.

  // Common phrases offered with empty text; the user fills in their own.
  const STARTERS = [
    { phrase: "my email", placeholder: "you@example.com" },
    { phrase: "my LinkedIn", placeholder: "https://www.linkedin.com/in/your-name" },
    { phrase: "my Instagram", placeholder: "https://www.instagram.com/your-name" },
  ];
  const starter = (phrase) =>
    STARTERS.find((s) => s.phrase.toLowerCase() === phrase.trim().toLowerCase());
  const missing = (items) =>
    STARTERS.filter((s) => !items.some((x) => x.phrase.trim().toLowerCase() === s.phrase.toLowerCase()));

  GF.views.spoken = GF.personal({
    key: "spoken_shortcuts",
    panel: "spoken-panel",
    empty: "no shortcuts yet",
    add: "add a shortcut…",
    footnote:
      "Say the whole phrase and Greta types your text exactly as written. " +
      "Shortcuts without text stay inactive.",
    blank: () => ({ phrase: "", text: "", enabled: true }),
    unset: (s) => !s.text.trim(),
    summary: (s) => "“" + s.phrase + "”  →  " + (s.text.trim() ? s.text : "(no text yet)"),
    fields: [
      {
        label: "say",
        placeholder: () => "e.g. my website",
        get: (s) => s.phrase,
        set: (s, v) => ({ ...s, phrase: v }),
      },
      {
        label: "type",
        multiline: true,
        placeholder: (s) => (starter(s.phrase) || {}).placeholder || "text or link to type",
        get: (s) => s.text,
        set: (s, v) => ({ ...s, text: v }),
      },
    ],
    extraActions: (items, commit) => {
      const add = missing(items);
      if (!add.length) return [];
      return [
        GF.el("div", {
          class: "item",
          onclick: () => commit(items.concat(add.map((s) => ({ phrase: s.phrase, text: "", enabled: true })))),
        },
        GF.el("span", { class: "bullet", text: "+" }),
        GF.el("span", { class: "label", text: "add starters: " + add.map((s) => s.phrase).join(", ") })),
      ];
    },
  });
})();
