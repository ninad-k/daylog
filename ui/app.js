// daylog frontend. Talks to the Rust backend purely over Tauri IPC.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
let latestStatus = null;
let savedConfig = null;
let refreshBusy = false;
let uiError = null;

function showError(error) {
  uiError = String(error);
  $("error-banner").textContent = uiError;
  $("error-banner").classList.remove("hidden");
  setTimeout(() => { uiError = null; if (latestStatus) renderStatus(latestStatus); }, 10000);
}
window.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  showError(event.reason);
});

// ---------- helpers ----------
function fmtDur(secs) {
  secs = Math.max(0, Math.round(secs));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

function startOfToday() {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d;
}

function rangeFilter(range) {
  const now = new Date();
  let from = null;
  if (range === "today") from = startOfToday();
  else if (range === "7d") from = new Date(now.getTime() - 7 * 864e5);
  else if (range === "30d") from = new Date(now.getTime() - 30 * 864e5);
  const f = {};
  if (from) f.from = from.toISOString();
  return f;
}

function dateInputToISO(val, endOfDay) {
  if (!val) return null;
  const d = new Date(val + (endOfDay ? "T23:59:59.999" : "T00:00:00"));
  return d.toISOString();
}

function toast(el, msg, isErr) {
  el.textContent = msg;
  el.style.color = isErr ? "var(--danger)" : "var(--accent-2)";
  if (msg) setTimeout(() => { if (el.textContent === msg) el.textContent = ""; }, 4000);
}

// ---------- status ----------
function renderStatus(st) {
  latestStatus = st;
  $("error-banner").textContent = st.error || uiError || "";
  $("error-banner").classList.toggle("hidden", !st.error && !uiError);
  $("retry-storage").classList.toggle("hidden", !st.error);
  $("capture-note").textContent = !st.capture_available
    ? "No active window available. Check OS permissions if this persists."
    : !st.idle_available && savedConfig?.idle_threshold_seconds !== null ? "OS idle detection is unavailable on this desktop." : "";
  const state = $("pill-state");
  if (st.locked) {
    state.textContent = "locked";
    state.className = "pill paused";
  } else if (st.paused) {
    state.textContent = "paused";
    state.className = "pill paused";
  } else if (st.error || st.excluded || st.idle || !st.capture_available) {
    state.textContent = st.error ? "storage error" : st.excluded ? "excluded" : st.idle ? "idle" : "waiting";
    state.className = "pill paused";
  } else {
    state.textContent = "logging";
    state.className = "pill running";
  }
  $("toggle-pause").textContent = st.paused ? "Resume" : "Pause";

  const enc = $("pill-enc");
  enc.textContent = st.encrypted ? "encrypted" : "plaintext";
  enc.className = "pill " + (st.encrypted ? "enc-on" : "pill-muted");

  const cur = $("pill-current");
  cur.textContent = st.current_app ? st.current_app : "idle";
  cur.title = st.current_title || "";

  $("lock-overlay").classList.toggle("hidden", !st.locked);
}

async function refreshStatus() {
  try { renderStatus(await invoke("get_status")); } catch (e) { console.error(e); }
}

// ---------- dashboard ----------
async function loadDashboard() {
  const filter = rangeFilter($("range-select").value);
  let a;
  try { a = await invoke("analytics", { filter }); }
  catch (e) { showError(e); return; }

  $("stat-total").textContent = fmtDur(a.total_seconds);
  $("stat-events").textContent = a.total_events.toLocaleString();
  $("stat-top").textContent = a.per_app.length ? a.per_app[0].app : "—";

  const bars = $("app-bars");
  if (!a.per_app.length) { bars.innerHTML = '<div class="empty">No data yet.</div>'; }
  else {
    const max = a.per_app[0].seconds || 1;
    bars.innerHTML = a.per_app.slice(0, 12).map((r) => `
      <div class="bar-row">
        <div class="bar-app" title="${esc(r.app)}">${esc(r.app)}</div>
        <div class="bar-track"><div class="bar-fill" style="width:${(r.seconds / max) * 100}%"></div></div>
        <div class="bar-val">${fmtDur(r.seconds)}</div>
      </div>`).join("");
  }

  const days = $("day-bars");
  if (!a.per_day.length) { days.innerHTML = '<div class="empty">No data yet.</div>'; }
  else {
    const max = Math.max(...a.per_day.map((d) => d.seconds), 1);
    days.innerHTML = a.per_day.slice(-30).map((d) => `
      <div class="day-col" title="${d.date}: ${fmtDur(d.seconds)}">
        <div class="day-fill" style="height:${(d.seconds / max) * 100}%"></div>
        <div class="day-label">${d.date.slice(5)}</div>
      </div>`).join("");
  }
}

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

// ---------- log ----------
function currentLogFilter() {
  const limit = parseInt($("f-limit").value, 10);
  const f = {};
  const kw = $("f-keyword").value.trim();
  const app = $("f-app").value.trim();
  if (kw) f.keyword = kw;
  if (app) f.app = app;
  const from = dateInputToISO($("f-from").value, false);
  const to = dateInputToISO($("f-to").value, true);
  if (from) f.from = from;
  if (to) f.to = to;
  if (limit > 0) f.limit = limit;
  return f;
}

async function loadLog() {
  let rows;
  try { rows = await invoke("query", { filter: currentLogFilter() }); }
  catch (e) { showError(e); return; }
  const body = $("log-body");
  $("log-empty").classList.toggle("hidden", rows.length > 0);
  body.innerHTML = rows.map((r) => {
    const start = new Date(r.start).toLocaleString();
    return `<tr>
      <td>${esc(start)}</td>
      <td>${fmtDur(r.seconds)}</td>
      <td>${esc(r.app)}</td>
      <td class="title" title="${esc(r.title)}">${esc(r.title)}</td>
    </tr>`;
  }).join("");
}

async function doExport(fmt) {
  try {
    const dest = await window.__TAURI__.dialog.save({
      defaultPath: `daylog-export.${fmt}`,
      filters: [{ name: fmt === "csv" ? "CSV" : "Markdown", extensions: [fmt] }],
    });
    if (!dest) return;
    const path = await invoke("export", { format: fmt, filter: currentLogFilter(), dest });
    const note = $("export-note");
    note.classList.remove("hidden");
    note.textContent = `Exported to ${path}`;
  } catch (e) { showError("Export failed: " + e); }
}

// ---------- settings ----------
async function loadSettings() {
  let c;
  try { c = await invoke("get_config"); } catch (e) { showError(e); return; }
  savedConfig = c;
  $("s-path").value = c.log_path;
  $("s-purge").value = c.auto_purge_days ?? "";
  $("s-enc").checked = c.encryption_enabled;
  $("s-excl").checked = c.exclusion_enabled;
  $("s-blocklist").value = (c.blocklist || []).join("\n");
  $("s-autostart").checked = c.start_on_login;
  $("s-show").checked = c.show_window_on_launch;
  $("s-poll").value = c.poll_interval_ms;
  $("s-idle").value = c.idle_threshold_seconds ?? "";
}

async function saveSettings() {
  const note = $("save-note");
  const encWanted = $("s-enc").checked;
  const encPw = $("s-enc-pw").value;

  // Non-crypto config first.
  const purgeRaw = $("s-purge").value.trim();
  const cfg = {
    log_path: $("s-path").value.trim(),
    encryption_enabled: encWanted,
    exclusion_enabled: $("s-excl").checked,
    blocklist: $("s-blocklist").value.split("\n").map((s) => s.trim()).filter(Boolean),
    start_on_login: $("s-autostart").checked,
    auto_purge_days: purgeRaw ? parseInt(purgeRaw, 10) : null,
    poll_interval_ms: Math.max(250, parseInt($("s-poll").value, 10) || 1500),
    show_window_on_launch: $("s-show").checked,
    idle_threshold_seconds: $("s-idle").value.trim() ? Number($("s-idle").value) : null,
  };

  if (!savedConfig) { toast(note, "Reload settings before saving.", true); return; }
  const enabling = encWanted && !savedConfig.encryption_enabled;
  if (enabling && !encPw) { toast(note, "Enter a password to enable encryption.", true); return; }
  if (encWanted !== savedConfig.encryption_enabled) {
    const message = encWanted
      ? "Encrypt and migrate your history? The old plaintext log will be removed after migration. Backups and exports remain unchanged."
      : "Decrypt your history into a readable plaintext log?";
    if (!await window.__TAURI__.dialog.confirm(message, { title: "Change encryption", kind: "warning" })) return;
  }
  $("save-settings").disabled = true;
  try {
    const st = await invoke("save_settings", { config: cfg, password: encPw || null });
    renderStatus(st);
    toast(note, "Saved.");
    $("s-enc-pw").value = "";
    await loadSettings();
    await loadDashboard();
  } catch (e) { toast(note, "Save failed: " + e, true); }
  finally { $("save-settings").disabled = false; }
}

// ---------- unlock ----------
async function doUnlock() {
  const pw = $("unlock-pw").value;
  try {
    const st = await invoke("unlock", { password: pw });
    renderStatus(st);
    $("lock-overlay").classList.add("hidden");
    $("unlock-pw").value = "";
    $("unlock-err").textContent = "";
    initAfterUnlock();
  } catch (e) { $("unlock-err").textContent = String(e); }
}

// ---------- wiring ----------
function switchTab(name) {
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.dataset.tab === name));
  document.querySelectorAll(".tab-panel").forEach((p) => p.classList.toggle("active", p.id === "tab-" + name));
  if (name === "dashboard") loadDashboard();
  if (name === "log") loadLog();
  if (name === "settings") loadSettings();
}

function initAfterUnlock() {
  loadDashboard();
  loadSettings();
}

function wire() {
  document.querySelectorAll(".tab").forEach((t) => t.addEventListener("click", () => switchTab(t.dataset.tab)));
  $("toggle-pause").addEventListener("click", async () => {
    const st = await invoke("get_status");
    renderStatus(await invoke(st.paused ? "resume" : "pause"));
  });
  $("refresh-dash").addEventListener("click", loadDashboard);
  $("range-select").addEventListener("change", loadDashboard);
  $("apply-filter").addEventListener("click", loadLog);
  $("export-csv").addEventListener("click", () => doExport("csv"));
  $("export-md").addEventListener("click", () => doExport("md"));
  $("clear-logs").addEventListener("click", async () => {
    if (!await window.__TAURI__.dialog.confirm("Delete all sessions from the active log? This cannot be undone.", { title: "Clear history", kind: "warning" })) return;
    await invoke("clear_logs");
    loadLog(); loadDashboard(); refreshStatus();
  });
  $("open-folder").addEventListener("click", () => invoke("reveal_log").catch(showError));
  $("save-settings").addEventListener("click", saveSettings);
  $("unlock-btn").addEventListener("click", doUnlock);
  $("unlock-pw").addEventListener("keydown", (e) => { if (e.key === "Enter") doUnlock(); });

  // Live status from the capture loop.
  listen("daylog://status", (ev) => renderStatus(ev.payload)).catch(showError);
  $("retry-storage").addEventListener("click", async () => {
    renderStatus(await invoke("retry_storage"));
    await refreshVisible();
  });
  document.addEventListener("visibilitychange", () => { if (!document.hidden) refreshVisible(); });
  setInterval(refreshVisible, 5000);
}

async function refreshVisible() {
  if (document.hidden || refreshBusy || !latestStatus || latestStatus.locked) return;
  refreshBusy = true;
  try {
    if ($("tab-dashboard").classList.contains("active")) await loadDashboard();
    if ($("tab-log").classList.contains("active")) await loadLog();
  } finally { refreshBusy = false; }
}

async function boot() {
  wire();
  let st;
  try { st = await invoke("get_status"); } catch (e) { showError(e); return; }
  renderStatus(st);
  if (!st.locked) { loadDashboard(); loadSettings(); }
}

boot();
