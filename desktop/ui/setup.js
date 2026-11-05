const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

const LEGACY_STEP_KEY = "setup-step";
const SECTIONS = [...document.querySelectorAll("section[data-step]")];
const STEPS = SECTIONS.map((s) => s.dataset.step);

// Saves run strictly in navigation order. Resume is a convenience, so a failed
// save never blocks setup.
let saveQueue = Promise.resolve();
function saveStep(step) {
  saveQueue = saveQueue
    .then(() => invoke("setup_save_step", { step }))
    .catch((e) => console.warn("couldn't save the setup step", e));
  return saveQueue;
}

// Earlier builds kept the step in localStorage, which Rust can't read. Move it
// across once, and keep it until the backend has it.
async function migrateLegacyStep(saved) {
  let legacy = null;
  try { legacy = localStorage.getItem(LEGACY_STEP_KEY); } catch { return saved; }
  if (legacy === null) return saved;
  let step = saved;
  if (!saved && STEPS.includes(legacy)) {
    try {
      await invoke("setup_save_step", { step: legacy });
      step = legacy;
    } catch (e) {
      console.warn("couldn't migrate the setup step", e);
      return legacy;
    }
  }
  try { localStorage.removeItem(LEGACY_STEP_KEY); } catch { /* retried next open */ }
  return step;
}

async function restoreStep() {
  let saved = null;
  try {
    saved = await invoke("setup_saved_step");
  } catch (e) {
    console.warn("ignoring the saved setup step", e);
  }
  return migrateLegacyStep(saved);
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
  $("next").disabled = step === "model" && !modelReady();
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
  await enterStep(STEPS[index]);
}

async function enterStep(step) {
  if (step === "model") await checkDisk();
  else if (step === "microphone") await enterMic();
  else if (step in PERMISSIONS) startPermissionPoll();
  else if (step === "practice") await enterPractice();
}

function leaveStep() {
  setRelaunchStatus("");
  stopMicPoll();
  stopPermissionPoll();
  stopMeter();
  clearInterval(practiceTimer);
}

$("back").onclick = () => show(current - 1);
$("next").onclick = async () => {
  if (STEPS[current] !== "finish") { show(current + 1); return; }
  $("next").disabled = true;
  await saveStep(null);
  invoke("close_setup");
};

// Permission buttons open the matching Privacy pane.
for (const b of document.querySelectorAll(".perm")) {
  b.onclick = () => invoke("open_privacy", { pane: b.dataset.pane });
}

// A new grant can need a new process. The step is saved first, so setup reopens
// here. The reply can be lost as the process exits, so success shows nothing.
function setRelaunchStatus(text, kind) {
  setStatus($("relaunch-status"), text, kind);
  $("relaunch-status").hidden = !text;
}

for (const b of document.querySelectorAll(".relaunch")) {
  b.onclick = async () => {
    b.disabled = true;
    setRelaunchStatus("Relaunching Greta…");
    try {
      await saveQueue;
      await invoke("setup_relaunch", { step: STEPS[current] });
    } catch (e) {
      b.disabled = false;
      setRelaunchStatus(`Greta couldn’t relaunch: ${e}. Quit and reopen it to continue from this step.`, "bad");
    }
  };
}

// --- Model -----------------------------------------------------------------
// setup-model-status drives this step. Byte counts are per file, so a full bar
// only means that file finished. Only the ready phase means the model works.
const MODEL_WORK = {
  queued: "Getting ready",
  checking: "Checking model files",
  downloading: "Downloading",
  verifying: "Verifying files",
  preparing: "Preparing",
};
let modelStatus = null;
let modelRevision = 0;
let modelPhaseStarted = 0;
let modelTimer = null;
let modelRequested = false;
let disk = null;
let diskFailed = false;

const modelPhase = () => modelStatus?.phase ?? null;
const modelWorking = () => modelPhase() in MODEL_WORK;
const modelReady = () =>
  !modelWorking() && (menu.has_engine || modelPhase() === "ready");

function formatBytes(bytes) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.round(bytes / 1e3)} KB`;
}

function elapsed() {
  const s = Math.floor((Date.now() - modelPhaseStarted) / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function renderModelProgress() {
  const phase = modelPhase();
  const total = modelStatus?.total_bytes;
  const known = phase === "downloading" && total > 0;
  const fill = $("model-fill");
  $("model-track").classList.toggle("busy", !known);
  fill.style.width = known
    ? `${Math.min(100, (modelStatus.downloaded_bytes / total) * 100)}%` : "";
  if (phase === "downloading" && modelStatus.file) {
    const done = formatBytes(modelStatus.downloaded_bytes);
    const size = known ? `${done} of ${formatBytes(total)}` : `${done} so far`;
    $("model-detail").textContent = `${modelStatus.file} · ${size}`;
  } else {
    $("model-detail").textContent = `${MODEL_WORK[phase]}… ${elapsed()}`;
  }
}

function renderDisk() {
  const el = $("model-disk");
  if (modelWorking() || (modelReady() && modelPhase() !== "failed")) setStatus(el, "");
  else if (diskFailed) {
    setStatus(el, "Greta couldn’t check free disk space. You can still try the download.");
  } else if (disk) {
    const free = formatBytes(disk.available_bytes);
    const need = formatBytes(disk.required_bytes);
    // Advisory: cached files can need less, and the download checks again itself.
    if (disk.sufficient) setStatus(el, `${free} free. A new install needs about ${need}.`);
    else setStatus(el, `Only ${free} free. A new install needs about ${need}, so free up space first.`, "bad");
  }
}

function renderModel() {
  const phase = modelPhase();
  const working = modelWorking();
  const failed = phase === "failed";
  const ready = modelReady();
  const preparing = phase === "preparing";

  $("model-title").textContent = preparing ? "Preparing the model" : "Download a speech model";
  $("model-sub").textContent = preparing
    ? "The model is downloaded. Greta is getting it ready for this Mac."
    : "Greta ships without one so you can choose.";
  $("model-intro").hidden = working;
  $("model-busy").hidden = !working;
  $("model-note").textContent = preparing
    ? "The first time can take about half a minute, and the bar won’t fill up. That’s normal. Next turns on when the model is ready."
    : "Each file has its own progress. The model isn’t ready until Greta says so here.";
  if (working) renderModelProgress();

  $("download").hidden = (ready && !failed) || working;
  $("download").disabled = modelRequested;
  $("download").textContent = failed ? "Retry Download" : "Download Recommended Model (464\u00a0MB)";
  $("close").hidden = ready;

  if (failed) {
    const reason = modelStatus.failure_reason || "Greta didn’t get a reason.";
    const keep = menu.has_engine ? " Greta is still using the model that was working." : "";
    setStatus($("model-status"), `The model didn’t finish. Finished files are kept.${keep} Reason: ${reason}`, "bad");
  } else if (ready) setStatus($("model-status"), "✓ Model ready", "ok");
  else setStatus($("model-status"), "");
  renderDisk();
  updateNav();
}

function syncModelTimer() {
  const working = modelWorking();
  if (working && !modelTimer) {
    modelTimer = setInterval(() => {
      if (modelPhase() !== "downloading") renderModelProgress();
    }, 1000);
  } else if (!working && modelTimer) {
    clearInterval(modelTimer);
    modelTimer = null;
  }
}

// Snapshots and events can arrive in either order; the higher revision wins.
function applyModelStatus(status) {
  if (!status || status.revision <= modelRevision) return;
  const moved = status.phase !== modelStatus?.phase
    || status.generation !== modelStatus?.generation;
  modelRevision = status.revision;
  modelStatus = status;
  modelRequested = false;
  if (moved) modelPhaseStarted = Date.now();
  syncModelTimer();
  renderModel();
  if (!moved) return;
  if (status.phase === "ready" || status.phase === "failed") refresh();
  if (status.phase === "failed") checkDisk();
}

async function checkDisk() {
  try {
    disk = await invoke("setup_disk_space");
    diskFailed = false;
  } catch (e) {
    console.warn("couldn't read free disk space", e);
    disk = null;
    diskFailed = true;
  }
  renderDisk();
}

$("download").onclick = async () => {
  const name = modelPhase() === "failed" ? modelStatus.model : menu.models[0]?.id;
  if (!name) return;
  modelRequested = true;
  renderModel();
  try {
    await invoke("menu_choose_model", { name });
  } catch (e) {
    setStatus($("model-status"), `Greta couldn’t start the download: ${e}`, "bad");
  }
  modelRequested = false;
  $("download").disabled = false;
};

$("close").onclick = () => invoke("close_setup");

// --- Microphone ------------------------------------------------------------
const hasMicApi = () => Boolean(navigator.mediaDevices?.getUserMedia);
const openMic = () => navigator.mediaDevices.getUserMedia({ audio: true });
let micPoll = null;
let micCheck = null;
let micOn = false;

function stopMicPoll() {
  clearInterval(micPoll);
  micPoll = null;
}

// The status check can wait on the macOS prompt, so overlapping checks share one.
function checkMic() {
  micCheck ??= readMic().finally(() => { micCheck = null; });
  return micCheck;
}

async function readMic() {
  let result = "off";
  try {
    const stream = await openMic();
    stream.getTracks().forEach((t) => t.stop());
    result = "on";
  } catch (e) {
    if (e.name === "NotFoundError") result = "missing";
  }
  micOn = result === "on";
  $("mic-relaunch").hidden = result !== "off";
  if (result === "on") setStatus($("mic-status"), "✓ Microphone is on", "ok");
  else if (result === "missing") {
    setStatus($("mic-status"), "No microphone found. Connect one, then try again.", "bad");
  } else setStatus($("mic-status"), "Microphone is off. Waiting for you to turn it on…", "bad");
  if (result !== "off") stopMicPoll();
  return result;
}

function pollMic() {
  if (STEPS[current] === "microphone" && !micPoll) micPoll = setInterval(checkMic, 2000);
}

async function enterMic() {
  if (!hasMicApi()) {
    $("mic-relaunch").hidden = false;
    return;
  }
  if ((await checkMic()) === "off") pollMic();
}

$("mic-allow").onclick = async () => {
  if (!hasMicApi()) {
    // No live status here: ask macOS directly and let practice confirm it.
    invoke("prime_microphone");
    invoke("open_privacy", { pane: "microphone" });
    setStatus($("mic-status"), "Turn on Greta Flow in System Settings. The practice step confirms it.");
    return;
  }
  if ((await checkMic()) !== "off" || STEPS[current] !== "microphone") return;
  invoke("open_privacy", { pane: "microphone" });
  pollMic();
};

// --- Accessibility and Input Monitoring ------------------------------------
// A false result can also mean a new grant waiting on a relaunch, and a true
// result doesn't prove the Fn event tap recovered. Turning on while this page
// watches is the one case known to need a relaunch.
const PERMISSIONS = {
  accessibility: { field: "accessibility", label: "Accessibility", relaunchWhenOn: false },
  input: { field: "input_monitoring", label: "Input Monitoring", relaunchWhenOn: true },
};
const PERMISSION_POLL_MS = 1000;
let permissionPoll = null;
let permissionRead = null;
const seenOff = new Set();

function stopPermissionPoll() {
  clearInterval(permissionPoll);
  permissionPoll = null;
}

function readPermissions() {
  permissionRead ??= invoke("setup_permissions").finally(() => { permissionRead = null; });
  return permissionRead;
}

function renderPermission(step, on) {
  const { label, relaunchWhenOn } = PERMISSIONS[step];
  if (!on) seenOff.add(step);
  const needsRelaunch = on && relaunchWhenOn && seenOff.has(step);
  const relaunch = $(`${step}-relaunch`);
  relaunch.hidden = on && !relaunchWhenOn;
  relaunch.classList.toggle("primary", needsRelaunch);
  $(`${step}-settings`).classList.toggle("primary", !on);
  if (needsRelaunch) {
    setStatus($(`${step}-status`), `✓ ${label} is on. Relaunch Greta so the hotkey can use it.`, "ok");
  } else if (on) setStatus($(`${step}-status`), `✓ ${label} is on`, "ok");
  else setStatus($(`${step}-status`), `${label} is off. Waiting for you to turn it on…`, "bad");
}

async function checkPermission() {
  const step = STEPS[current];
  if (!(step in PERMISSIONS)) return;
  try {
    const permissions = await readPermissions();
    if (STEPS[current] !== step) return;
    renderPermission(step, permissions[PERMISSIONS[step].field]);
  } catch (e) {
    if (STEPS[current] !== step) return;
    setStatus($(`${step}-status`), `Greta couldn’t read this permission: ${e}`, "bad");
  }
}

function startPermissionPoll() {
  stopPermissionPoll();
  checkPermission();
  permissionPoll = setInterval(checkPermission, PERMISSION_POLL_MS);
}

// Returning from System Settings shouldn't wait for the next poll.
window.addEventListener("focus", () => {
  const step = STEPS[current];
  if (step === "microphone" && hasMicApi() && !micOn) checkMic();
  else if (step in PERMISSIONS) checkPermission();
  else if (step === "model") checkDisk();
});

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

async function practiceBlocker(s) {
  if (!s.has_engine) return { text: "The model isn’t ready yet. Go back to the model step first." };
  let permissions;
  try {
    permissions = await readPermissions();
  } catch {
    return null; // The practice run itself shows whether typing works.
  }
  if (!permissions.accessibility) {
    return {
      text: "Accessibility looks off, so Greta may not be able to type here.",
      pane: "accessibility", button: "Open Accessibility Settings",
    };
  }
  if (s.shortcuts[0] !== "Fn") return null;
  if (!permissions.input_monitoring) {
    return {
      text: "Input Monitoring looks off, so the Fn key may not work yet.",
      pane: "input", button: "Open Input Monitoring Settings",
    };
  }
  if (seenOff.has("input")) {
    return { text: "If the Fn key doesn’t respond, go back and relaunch Greta." };
  }
  return null;
}

function showPracticeHelp(pane, button) {
  $("practice-settings").dataset.pane = pane;
  $("practice-settings").textContent = button;
  $("practice-settings").hidden = false;
}

async function enterPractice() {
  const s = await refresh();
  const blocker = await practiceBlocker(s);
  if (STEPS[current] !== "practice") return;
  recentAtStart = s.recent[0] ?? null;
  $("practice-settings").hidden = true;
  setStatus($("practice-status"), blocker?.text ?? "", blocker ? "bad" : "");
  if (blocker?.pane) showPracticeHelp(blocker.pane, blocker.button);
  $("practice").focus();
  clearInterval(practiceTimer);
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
    showPracticeHelp("accessibility", "Open Accessibility Settings");
  }, 500);
}
$("practice-settings").onclick = () =>
  invoke("open_privacy", { pane: $("practice-settings").dataset.pane });

// --- Start -----------------------------------------------------------------
async function start() {
  await listen("menu-updated", async () => {
    await refresh();
    checkPracticeDelivery();
  });
  // Subscribe before the snapshot so nothing falls between the two.
  await listen("setup-model-status", (event) => applyModelStatus(event.payload));
  applyModelStatus(await invoke("setup_model_status"));
  await refresh();
  const saved = STEPS.indexOf(await restoreStep());
  resumed = saved > 0;
  await show(Math.max(saved, 0));
}

start();
