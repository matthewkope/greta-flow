const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

const STEP_KEY = "setup-step";
const SECTIONS = [...document.querySelectorAll("section[data-step]")];
const STEPS = SECTIONS.map((s) => s.dataset.step);

// Storage can be unavailable; setup then just starts from the first step.
function savedStep() {
  try { return localStorage.getItem(STEP_KEY); } catch { return null; }
}
function saveStep(step) {
  try {
    if (step) localStorage.setItem(STEP_KEY, step);
    else localStorage.removeItem(STEP_KEY);
  } catch { /* resume is a convenience */ }
}

function setStatus(el, text, kind) {
  el.textContent = text;
  el.className = "status" + (kind ? " " + kind : "");
}

let current = 0;
let resumed = false;
let menu = { has_engine: false, models: [], shortcuts: [], recent: [] };

const refresh = async () => {
  menu = await invoke("menu_state");
  const accel = menu.shortcuts[0];
  const key = accel === "Fn" ? "the 🌐 Globe/Fn key" : accel || "your hotkey";
  $("practice-key").textContent = key;
  $("finish-key").textContent = key;
  renderModel();
  return menu;
};

function updateNav() {
  const step = STEPS[current];
  $("back").disabled = current === 0;
  $("next").disabled = step === "model" && !menu.has_engine;
  $("next").textContent = step === "finish" ? "Done" : "Next";
}

async function show(index) {
  leaveStep();
  current = index;
  SECTIONS.forEach((s, i) => { s.hidden = i !== index; });
  saveStep(STEPS[index]);
  const place = `Step ${index + 1} of ${STEPS.length}`;
  $("progress").textContent = resumed ? `${place} · picking up where you left off` : place;
  resumed = false;
  updateNav();
  if (STEPS[index] === "practice") await enterPractice();
}

function leaveStep() {
  stopMicPoll();
  stopMeter();
  clearInterval(practiceTimer);
}

$("back").onclick = () => show(current - 1);
$("next").onclick = () => {
  if (STEPS[current] !== "finish") { show(current + 1); return; }
  saveStep(null);
  invoke("close_setup");
};

// Permission buttons open the matching Privacy pane.
for (const b of document.querySelectorAll(".perm")) {
  b.onclick = () => invoke("open_privacy", { pane: b.dataset.pane });
}
$("quit").onclick = () => invoke("menu_quit");

// --- Model -----------------------------------------------------------------
// set_model refreshes the menu once when it starts and once when it finishes.
// A failure leaves no other signal, so the second update without an engine is
// treated as failed. Retrying is safe: verified files aren't downloaded again.
let modelWork = null;

function renderModel() {
  const ready = menu.has_engine;
  if (ready && modelWork) stopModelWork();
  const working = Boolean(modelWork) && !modelWork.failed;
  $("model-busy").hidden = !working;
  $("download").hidden = ready || working;
  $("close").hidden = ready;
  if (ready) setStatus($("model-status"), "✓ Model ready", "ok");
  else if (modelWork?.failed) {
    setStatus($("model-status"),
      "The download stopped. Check your connection and free disk space, then retry. Finished files are kept.",
      "bad");
    $("download").textContent = "Retry Download";
  } else setStatus($("model-status"), "");
  updateNav();
}

function stopModelWork() {
  clearInterval(modelWork.timer);
  modelWork = null;
}

$("download").onclick = async () => {
  if (modelWork) stopModelWork();
  const started = Date.now();
  const tick = () => {
    const s = Math.floor((Date.now() - started) / 1000);
    $("model-elapsed").textContent =
      `Working… ${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  tick();
  modelWork = { updates: 0, failed: false, timer: setInterval(tick, 1000) };
  renderModel();
  await invoke("menu_choose_model", { name: menu.models[0].id });
};

$("close").onclick = () => invoke("close_setup");

// --- Microphone ------------------------------------------------------------
const hasMicApi = () => Boolean(navigator.mediaDevices?.getUserMedia);
const openMic = () => navigator.mediaDevices.getUserMedia({ audio: true });
let micPoll = null;

function stopMicPoll() {
  clearInterval(micPoll);
  micPoll = null;
}

async function checkMic() {
  try {
    const stream = await openMic();
    stream.getTracks().forEach((t) => t.stop());
    stopMicPoll();
    setStatus($("mic-status"), "✓ Microphone is on", "ok");
    return "on";
  } catch (e) {
    if (e.name === "NotFoundError") {
      stopMicPoll();
      setStatus($("mic-status"), "No microphone found. Connect one, then try again.", "bad");
      return "missing";
    }
    setStatus($("mic-status"), "Microphone is off. Waiting for you to turn it on…", "bad");
    return "off";
  }
}

$("mic-allow").onclick = async () => {
  if (!hasMicApi()) {
    // No live status here: ask macOS directly and let practice confirm it.
    invoke("prime_microphone");
    invoke("open_privacy", { pane: "microphone" });
    setStatus($("mic-status"), "Turn on Greta Flow in System Settings. The practice step confirms it.");
    return;
  }
  stopMicPoll();
  if ((await checkMic()) !== "off" || STEPS[current] !== "microphone") return;
  invoke("open_privacy", { pane: "microphone" });
  micPoll = setInterval(checkMic, 2000);
};

// --- Mic level -------------------------------------------------------------
const SPEECH_RMS = 0.02; // Clearly above room noise.
const HEARD_TICKS = 10; // About half a second of speech at 50 ms per tick.
const CHECK_MS = 20000;
let meter = null;

function stopMeter() {
  if (!meter) return;
  clearInterval(meter.timer);
  meter.stream.getTracks().forEach((t) => t.stop());
  meter.context.close();
  meter = null;
  $("level-start").disabled = false;
  $("level-start").textContent = "Test Again";
}

$("level-start").onclick = async () => {
  const status = $("level-status");
  if (!hasMicApi()) {
    setStatus(status, "A live level isn’t available here. The practice step tests your mic instead.");
    return;
  }
  $("level-start").disabled = true;
  let stream;
  try {
    stream = await openMic();
  } catch {
    $("level-start").disabled = false;
    setStatus(status, "Greta can’t reach the microphone. Go back and allow it first.", "bad");
    return;
  }
  // Never leave the mic open if the user moved on while it was starting.
  if (STEPS[current] !== "level") {
    stream.getTracks().forEach((t) => t.stop());
    $("level-start").disabled = false;
    return;
  }
  const context = new AudioContext();
  await context.resume();
  const analyser = context.createAnalyser();
  analyser.fftSize = 1024;
  context.createMediaStreamSource(stream).connect(analyser);
  const samples = new Float32Array(analyser.fftSize);
  const started = Date.now();
  let heard = 0;
  setStatus(status, "Listening… say a few words.");
  const timer = setInterval(() => {
    analyser.getFloatTimeDomainData(samples);
    const rms = Math.sqrt(samples.reduce((sum, v) => sum + v * v, 0) / samples.length);
    $("level-fill").style.width = `${Math.min(100, rms * 400)}%`;
    if (rms >= SPEECH_RMS) heard += 1;
    if (heard >= HEARD_TICKS) {
      stopMeter();
      setStatus(status, "✓ Greta can hear you", "ok");
    } else if (Date.now() - started > CHECK_MS) {
      stopMeter();
      $("level-fill").style.width = "0";
      setStatus(status,
        "Greta didn’t hear speech. Check the input device in System Settings ▸ Sound.", "bad");
    }
  }, 50);
  meter = { stream, context, timer };
};

// --- Practice --------------------------------------------------------------
let practiceTimer = null;
let recentAtStart = null;

async function enterPractice() {
  const s = await refresh();
  if (STEPS[current] !== "practice") return;
  recentAtStart = s.recent[0] ?? null;
  $("practice-settings").hidden = true;
  setStatus($("practice-status"), s.has_engine ? "" :
    "The model isn’t ready yet. Go back to the model step first.", s.has_engine ? "" : "bad");
  $("practice").focus();
  practiceTimer = setInterval(() => {
    if (!$("practice").value.trim()) return;
    $("practice-settings").hidden = true;
    setStatus($("practice-status"), "✓ It works", "ok");
    clearInterval(practiceTimer);
  }, 300);
}

// Text in Recent with nothing in the box means recognition worked but typing
// didn't, which is what a missing Accessibility grant looks like.
function checkPracticeDelivery() {
  if (STEPS[current] !== "practice" || $("practice").value.trim()) return;
  if ((menu.recent[0] ?? null) === recentAtStart) return;
  setTimeout(() => {
    if ($("practice").value.trim()) return;
    setStatus($("practice-status"),
      `Greta heard “${menu.recent[0]}” but couldn’t type it. Click in the box and try again. If that fails, check Accessibility.`,
      "bad");
    $("practice-settings").hidden = false;
  }, 500);
}
$("practice-settings").onclick = () => invoke("open_privacy", { pane: "accessibility" });

// --- Start -----------------------------------------------------------------
async function start() {
  await listen("menu-updated", async () => {
    if (modelWork) modelWork.updates += 1;
    await refresh();
    if (modelWork && modelWork.updates >= 2) {
      clearInterval(modelWork.timer);
      modelWork.failed = true;
      renderModel();
    }
    checkPracticeDelivery();
  });
  await refresh();
  const saved = STEPS.indexOf(savedStep());
  resumed = saved > 0;
  await show(Math.max(saved, 0));
}

start();
