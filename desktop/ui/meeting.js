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

  // Source labels say where audio came from, not who spoke.
  const SOURCE_LABEL = { microphone: "mic", system: "system" };

  function line(ts, text, gap, source) {
    const li = el("li", "gm-line" + (gap ? " gap" : ""));
    li.append(el("span", "gm-ts", ts));
    if (source) li.append(el("span", "gm-src " + source, SOURCE_LABEL[source] || source));
    li.append(el("span", "gm-text", text));
    return li;
  }

  // Segments and gaps interleaved by start time.
  function transcript(segments, gaps) {
    const ul = el("ul", "gm-lines");
    const rows = segments.map((s) => ({ at: s.start_ms, node: line(clock(s.start_ms), s.text, false, s.source || "microphone") }))
      .concat((gaps || []).map((g) => ({
        at: g.start_ms,
        node: line(`${clock(g.start_ms)}–${clock(g.end_ms)}`, `no transcript: ${g.reason}`, true, g.source || "microphone"),
      })))
      .sort((a, b) => a.at - b.at);
    if (!rows.length) ul.append(el("li", "gm-empty", "Nothing transcribed yet."));
    for (const r of rows) ul.append(r.node);
    return ul;
  }


  const STATUS_LABEL = {
    unavailable: "Not summarized",
    generating: "Generating…",
    generated: "Model summary",
    failed: "Summary failed",
  };

  // One grounded item: timestamp, model text, and the verbatim quote it cites.
  function groundedLine(item) {
    const li = el("li", "gm-line");
    const body = el("span", "gm-text");
    body.append(el("span", null, "Model wording: " + item.text + (item.owner ? ` (${item.owner})` : "")));
    body.append(el("span", "gm-quote", `Transcript: “${item.quote}”`));
    li.append(el("span", "gm-ts", clock(item.start_ms)), body);
    return li;
  }

  function groundedList(items, empty) {
    const ul = el("ul", "gm-lines");
    if (!items.length) ul.append(el("li", "gm-empty", empty));
    for (const i of items) ul.append(groundedLine(i));
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
    let source = "microphone"; // "microphone" | "system" | "both"
    let timer = null;

    async function act(fn) {
      if (busy) return;
      busy = true;
      try { await fn(); } catch (e) { flash = String(e); }
      busy = false;
      render();
    }
    let flash = null;

    let summarizing = null; // meeting id while a summary runs
    let downloadPoll = null;

    function pollDownload() {
      if (downloadPoll) return;
      downloadPoll = setInterval(async () => {
        let d = null;
        try { d = await invoke("summary_model_download_state"); } catch (_) {}
        if (!d || !d.active) { clearInterval(downloadPoll); downloadPoll = null; }
        if (view !== "live") render();
      }, 1000);
    }

    async function summarySection(id, summary) {
      const box = el("div", "gm-card");
      const status = summarizing === id ? "generating" : summary.status;
      box.append(el("div", "gm-label", STATUS_LABEL[status] || status));
      box.append(el("div", null, summarizing === id ? "Generating a local summary…" : summary.reason));

      const s = summary.summary;
      if (status === "generated" && s) {
        box.append(el("p", "gm-note", `Model summary, grounded in quoted transcript · ${s.model}` +
          (s.dropped_ungrounded ? ` · ${s.dropped_ungrounded} ungrounded item(s) dropped` : "")));
        const failed = s.failed_chunks || [];
        if (failed.length) {
          box.append(el("p", "gm-note", `Partial summary — ${failed.length} of ${s.sections} sections failed: ` +
            failed.map((f) => `section ${f.index + 1}: ${f.reason}`).join("; ")));
        }
        box.append(groundedList(s.bullets, "No grounded points."));
        box.append(el("div", "gm-label", "Action items (model, grounded)"));
        box.append(groundedList(s.action_items, "None."));
      }

      let rt = null;
      try { rt = await invoke("summary_status"); } catch (e) { box.append(el("p", "gm-error", String(e))); }
      if (!rt) return box;
      const actions = el("div", "gm-actions");
      if (rt.state === "ready" || rt.state === "failed") {
        const go = el("button", "gm-btn small", status === "generated" ? "summarize again" : "summarize");
        go.disabled = busy || summarizing !== null;
        go.onclick = () => {
          summarizing = id;
          flash = null;
          render();
          invoke("meeting_summarize", { id })
            .catch((e) => { flash = String(e); })
            .finally(() => { summarizing = null; render(); });
        };
        actions.append(go);
        if (rt.state === "failed") box.append(el("p", "gm-error", rt.detail));
        else box.append(el("p", "gm-note", rt.detail));
      } else if (rt.state === "loading") {
        box.append(el("p", "gm-note", rt.detail));
      } else {
        box.append(el("p", "gm-note warn", rt.detail));
        const pathRow = el("div", "gm-actions");
        const input = el("input", "gm-input");
        input.type = "text";
        input.placeholder = "/path/to/mlx-model-folder";
        if (rt.path) input.value = rt.path;
        const choose = el("button", "gm-btn small", "Choose model folder…");
        choose.onclick = () => act(async () => {
          if (!input.value.trim()) throw new Error("Type the model folder path first.");
          await invoke("summary_model_set_path", { path: input.value });
          flash = null;
        });
        pathRow.append(input, choose);
        box.append(pathRow);

        let d = null;
        try { d = await invoke("summary_model_download_state"); } catch (_) {}
        const dl = el("button", "gm-btn small", "Download (2.26 GB)");
        if (d && d.active) {
          const pct = d.total ? Math.floor((d.bytes / d.total) * 100) : 0;
          dl.textContent = `Downloading… ${pct}%`;
          dl.disabled = true;
          pollDownload();
        } else {
          dl.onclick = () => {
            invoke("summary_model_download").catch((e) => { flash = String(e); }).finally(render);
            setTimeout(pollDownload, 200);
          };
        }
        actions.append(dl);
        if (d && d.error && !d.active) box.append(el("p", "gm-error", `Download failed: ${d.error}`));
        box.append(el("p", "gm-note",
          "Downloads mlx-community/Qwen3-4B-Instruct-2507-4bit (pinned revision, hash-checked) from Hugging Face. Summaries then run entirely on this Mac."));
      }
      box.append(actions);
      box.append(el("p", "gm-note",
        "Note: this summarizer has not yet been run against the real model on this machine; treat results as unverified."));
      return box;
    }

    function liveSection(s) {
      const box = el("div");
      const status = el("div", "gm-status" + (s.active && !s.finishing ? " rec" : s.finishing ? " busy" : ""));
      status.setAttribute("role", "status");
      status.append(el("span", "gm-dot"));
      status.append(el("span", "gm-state",
        s.finishing ? "FINISHING…" : s.active ? "REC" : "READY"));
      if (s.active && !s.finishing) {
        status.append(el("span", "gm-clock", `${clock(s.audio_ms)} captured · ${s.source === "microphone" ? "mic" : s.source === "both" ? "mic + system" : "system"}`));
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
        // Toggle buttons rather than a <select>: the panel re-renders every second.
        const picker = el("div", "gm-sources");
        picker.setAttribute("role", "radiogroup");
        picker.setAttribute("aria-label", "Audio source");
        picker.append(el("span", "gm-label", "SOURCE"));
        for (const [value, label] of [["microphone", "microphone"], ["system", "system audio"], ["both", "both"]]) {
          const b = el("button", "gm-btn small" + (source === value ? " on" : ""), label);
          b.setAttribute("role", "radio");
          b.setAttribute("aria-checked", String(source === value));
          b.disabled = busy;
          b.onclick = () => { source = value; render(); };
          picker.append(b);
        }
        box.append(picker);
        const start = el("button", "gm-btn", "start meeting");
        start.disabled = busy;
        start.onclick = () => act(async () => {
          await invoke("meeting_start", { source });
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
      box.append(await summarySection(id, r.summary));

      box.append(el("h2", null, "ACTION ITEM CANDIDATES"));
      box.append(el("p", "gm-note", "Keyword-detected sentences quoted from the transcript. Check before acting."));
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
