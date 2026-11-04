// daylog frontend. Talks to the Rust backend purely over Tauri IPC.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

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
  const d = new Date(val + (endOfDay ? "T23:59:59" : "T00:00:00"));
  return d.toISOString();
}

function toast(el, msg, isErr) {
  el.textContent = msg;
  el.style.color = isErr ? "var(--danger)" : "var(--accent-2)";
  if (msg) setTimeout(() => { if (el.textContent === msg) el.textContent = ""; }, 4000);
}

// ---------- status ----------
function renderStatus(st) {
  const state = $("pill-state");
  if (st.locked) {
    state.textContent = "locked";
    state.className = "pill paused";
  } else if (st.paused) {
    state.textContent = "paused";
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

  if (st.locked) $("lock-overlay").classList.remove("hidden");
}

async function refreshStatus() {
  try { renderStatus(await invoke("get_status")); } catch (e) { console.error(e); }
}

// ---------- dashboard ----------
async function loadDashboard() {
  const filter = rangeFilter($("range-select").value);
  let a;
  try { a = await invoke("analytics", { filter }); }
  catch (e) { console.error(e); return; }

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
  catch (e) { console.error(e); return; }
  const body = $("log-body");
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
    const path = await invoke("export", { format: fmt, filter: currentLogFilter(), dest: null });
    const note = $("export-note");
    note.classList.remove("hidden");
    note.textContent = `Exported to ${path}`;
  } catch (e) { alert("Export failed: " + e); }
}

// ---------- settings ----------
async function loadSettings() {
  let c;
  try { c = await invoke("get_config"); } catch (e) { console.error(e); return; }
  $("s-path").value = c.log_path;
  $("s-purge").value = c.auto_purge_days ?? "";
  $("s-enc").checked = c.encryption_enabled;
  $("s-excl").checked = c.exclusion_enabled;
  $("s-blocklist").value = (c.blocklist || []).join("\n");
  $("s-autostart").checked = c.start_on_login;
  $("s-show").checked = c.show_window_on_launch;
  $("s-poll").value = c.poll_interval_ms;
}

async function saveSettings() {
  const note = $("save-note");
  const encWanted = $("s-enc").checked;
  const encPw = $("s-enc-pw").value;

  // Non-crypto config first.
  const purgeRaw = $("s-purge").value.trim();
  const cfg = {
    log_path: $("s-path").value.trim(),
    encryption_enabled: encWanted, // backend reopen handled via set_encryption below
    exclusion_enabled: $("s-excl").checked,
    blocklist: $("s-blocklist").value.split("\n").map((s) => s.trim()).filter(Boolean),
    start_on_login: $("s-autostart").checked,
    auto_purge_days: purgeRaw ? parseInt(purgeRaw, 10) : null,
    poll_interval_ms: Math.max(250, parseInt($("s-poll").value, 10) || 1500),
    show_window_on_launch: $("s-show").checked,
  };

  try {
    // set_config persists everything and reopens the store when NOT switching to encrypted.
    let st = await invoke("set_config", { config: cfg });
    // Encryption on/off needs the password, so drive it explicitly.
    if (encWanted) {
      if (!encPw) { toast(note, "Enter a password to enable encryption.", true); return; }
      st = await invoke("set_encryption", { enabled: true, password: encPw });
    } else if (st.encrypted) {
      st = await invoke("set_encryption", { enabled: false, password: null });
    }
    renderStatus(st);
    toast(note, "Saved.");
    $("s-enc-pw").value = "";
  } catch (e) { toast(note, "Save failed: " + e, true); }
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
    if (!confirm("Delete ALL logged entries? This cannot be undone.")) return;
    await invoke("clear_logs");
    loadLog(); loadDashboard(); refreshStatus();
  });
  $("open-folder").addEventListener("click", () => invoke("reveal_log").catch((e) => alert(e)));
  $("save-settings").addEventListener("click", saveSettings);
  $("unlock-btn").addEventListener("click", doUnlock);
  $("unlock-pw").addEventListener("keydown", (e) => { if (e.key === "Enter") doUnlock(); });

  // Live status from the capture loop.
  listen("daylog://status", (ev) => renderStatus(ev.payload));
}

async function boot() {
  wire();
  let st;
  try { st = await invoke("get_status"); } catch (e) { console.error(e); return; }
  renderStatus(st);
  if (!st.locked) { loadDashboard(); loadSettings(); }
}

boot();
