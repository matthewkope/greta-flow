// Meeting notes panel. Self-contained: a window shell can call
// GretaMeetingPanel.mount(element) after loading meeting.css and this file.
(function () {
  "use strict";

  function clock(ms) {
    const s = Math.floor((ms || 0) / 1000);
    const pad = (n) => String(n).padStart(2, "0");
    return `${pad(Math.floor(s / 3600))}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}`;
  }

  function when(ms) {
    const d = new Date(ms);
    return d.toLocaleString(undefined, {
      month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
    });
  }

  function el(tag, cls, text) {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function line(ts, text, gap) {
    const li = el("li", "gm-line" + (gap ? " gap" : ""));
    li.append(el("span", "gm-ts", ts), el("span", "gm-text", text));
    return li;
  }

  // Segments and gaps interleaved by start time.
  function transcript(segments, gaps) {
    const ul = el("ul", "gm-lines");
    const rows = segments.map((s) => ({ at: s.start_ms, node: line(clock(s.start_ms), s.text) }))
      .concat((gaps || []).map((g) => ({
        at: g.start_ms,
        node: line(`${clock(g.start_ms)}–${clock(g.end_ms)}`, `no transcript: ${g.reason}`, true),
      })))
      .sort((a, b) => a.at - b.at);
    if (!rows.length) ul.append(el("li", "gm-empty", "Nothing transcribed yet."));
    for (const r of rows) ul.append(r.node);
    return ul;
  }

  function mount(root) {
    const invoke = window.__TAURI__.core.invoke;
    root.classList.add("gm");
    root.innerHTML = "";
    const scroll = el("div", "gm-scroll");
    root.append(scroll);

    let view = "live"; // "live" | meeting id
    let busy = false;
    let lastCount = -1;
    let timer = null;

    async function act(fn) {
      if (busy) return;
      busy = true;
      try { await fn(); } catch (e) { flash = String(e); }
      busy = false;
      render();
    }
    let flash = null;

    function liveSection(s) {
      const box = el("div");
      const status = el("div", "gm-status" + (s.active && !s.finishing ? " rec" : s.finishing ? " busy" : ""));
      status.setAttribute("role", "status");
      status.append(el("span", "gm-dot"));
      status.append(el("span", "gm-state",
        s.finishing ? "FINISHING…" : s.active ? "REC" : "READY"));
      if (s.active && !s.finishing) {
        status.append(el("span", "gm-clock", `${clock(s.audio_ms)} captured · mic`));
      }
      box.append(status);

      const actions = el("div", "gm-actions");
      if (s.active) {
        const stop = el("button", "gm-btn rec", s.finishing ? "saving…" : "stop & save");
        stop.disabled = s.finishing || busy;
        stop.onclick = () => act(async () => {
          const r = await invoke("meeting_stop");
          flash = null;
          view = r.id;
        });
        actions.append(stop);
      } else {
        const start = el("button", "gm-btn", "start meeting");
        start.disabled = busy;
        start.onclick = () => act(async () => {
          await invoke("meeting_start");
          flash = null;
          lastCount = -1;
        });
        actions.append(start);
      }
      box.append(actions);

      box.append(el("p", "gm-note warn", s.disclosure));
      if (s.active) box.append(el("p", "gm-note", "Dictation is paused until the meeting stops."));
      if (s.limit_reached) box.append(el("p", "gm-note warn", "Maximum meeting length reached — stop to save."));
      const error = flash || s.error;
      if (error) box.append(el("p", "gm-error", error));

      if (s.active) {
        box.append(el("h2", null, `LIVE TRANSCRIPT (${s.segment_count})`));
        if (s.segment_count > s.segments.length) {
          box.append(el("p", "gm-note", `Showing the latest ${s.segments.length}; the full transcript is saved.`));
        }
        box.append(transcript(s.segments, s.gaps));
      }
      return box;
    }

    async function listSection() {
      const box = el("div");
      box.append(el("h2", null, "SAVED MEETINGS"));
      let items = [];
      try { items = await invoke("meeting_list"); } catch (e) { box.append(el("p", "gm-error", String(e))); }
      if (!items.length) box.append(el("p", "gm-empty", "No meetings yet."));
      for (const m of items) {
        const b = el("button", "gm-item");
        const title = el("span", "gm-title", m.title);
        b.append(title);
        if (!m.complete) title.append(el("span", "gm-badge", "interrupted"));
        b.append(el("span", "gm-sub",
          `${when(m.started_ms)} · ${clock(m.audio_ms)} · ${m.words} words · ${m.action_items} action items`));
        if (m.preview) b.append(el("span", "gm-sub", m.preview));
        b.onclick = () => { view = m.id; render(); };
        box.append(b);
      }
      return box;
    }

    async function detail(id) {
      const box = el("div");
      const back = el("button", "gm-btn small gm-back", "◂ back");
      back.onclick = () => { view = "live"; flash = null; render(); };
      box.append(back);
      let r = null;
      try { r = await invoke("meeting_get", { id }); } catch (e) { flash = String(e); }
      if (!r) {
        box.append(el("p", "gm-error", flash || "Meeting not found."));
        return box;
      }
      box.append(el("h2", null, r.title.toUpperCase()));
      box.append(el("p", "gm-note",
        `${when(r.started_ms)} · ${clock(r.audio_ms)} · ${r.words} words · source: ${r.source}`));
      if (!r.complete) box.append(el("p", "gm-note warn", "Interrupted: saved from a checkpoint; the end may be missing."));
      if (r.limit_reached) box.append(el("p", "gm-note warn", "Stopped at the maximum meeting length."));

      const actions = el("div", "gm-actions");
      const copy = el("button", "gm-btn small", "copy notes");
      copy.onclick = () => act(async () => { await invoke("meeting_copy", { id }); flash = "Copied as Markdown."; });
      const del = el("button", "gm-btn small rec", "delete");
      del.onclick = () => {
        if (!window.confirm("Delete this meeting transcript? This can't be undone.")) return;
        act(async () => { await invoke("meeting_delete", { id }); view = "live"; flash = null; });
      };
      actions.append(copy, del);
      box.append(actions);
      if (flash) box.append(el("p", flash.startsWith("Copied") ? "gm-note" : "gm-error", flash));

      box.append(el("h2", null, "SUMMARY"));
      const sum = el("div", "gm-card");
      sum.append(el("div", "gm-label", r.summary.status === "unavailable" ? "Not available" : r.summary.status));
      sum.append(el("div", null, r.summary.reason));
      box.append(sum);

      box.append(el("h2", null, "ACTION ITEM CANDIDATES"));
      box.append(el("p", "gm-note", "Keyword-detected sentences quoted from the transcript — not AI. Check before acting."));
      const items = el("ul", "gm-lines");
      if (!r.action_items.length) items.append(el("li", "gm-empty", "None detected."));
      for (const a of r.action_items) items.append(line(clock(a.start_ms), a.quote));
      box.append(items);

      box.append(el("h2", null, "TRANSCRIPT"));
      box.append(transcript(r.segments, r.gaps));
      return box;
    }

    let rendering = false;
    async function render() {
      if (rendering) return;
      rendering = true;
      try {
        const nodes = [];
        if (view === "live") {
          const s = await invoke("meeting_state");
          const atBottom = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 40;
          nodes.push(liveSection(s));
          if (!s.active) nodes.push(await listSection());
          scroll.replaceChildren(...nodes);
          // Follow the live transcript only if the reader was already at the end.
          if (s.active && s.segment_count !== lastCount && atBottom) scroll.scrollTop = scroll.scrollHeight;
          lastCount = s.segment_count;
        } else {
          scroll.replaceChildren(await detail(view));
          scroll.scrollTop = 0;
        }
      } catch (e) {
        scroll.replaceChildren(el("p", "gm-error", `Meeting notes are not available in this build (${e}).`));
      }
      rendering = false;
    }

    function tick() {
      if (view === "live" && !document.hidden) render();
    }
    render();
    timer = setInterval(tick, 1000);
    return { refresh: render, unmount() { clearInterval(timer); root.innerHTML = ""; } };
  }

  window.GretaMeetingPanel = { mount };
})();
