// Plain JavaScript; all tracking and persistence stays behind the Tauri bridge.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
let config = null;
let categories = [];
let goals = [];
let page = 0;
let logRequest = 0;
let dashboardRequest = 0;
let activeTab = "dashboard";
let lastStatus = null;
let logDebounce;
let detailApp = null;
let detailReturnTab = "dashboard";
let detailReturnScroll = 0;
let detailMode = "text";
let detailPages = { text: 0, log: 0 };
let detailRequest = 0;
const DETAIL_PAGE_SIZE = 50;

function esc(s) { return String(s).replace(/[&<>"']/g, c => ({"&":"&amp;", "<":"&lt;", ">":"&gt;", '"':"&quot;", "'":"&#39;"}[c])); }
function fmtDur(secs) {
  secs = Math.max(0, Math.round(secs));
  const h = Math.floor(secs / 3600), m = Math.floor(secs % 3600 / 60), s = secs % 60;
  return h ? `${h}h ${m}m` : m ? `${m}m ${s}s` : `${s}s`;
}
function localDate(d) { return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,"0")}-${String(d.getDate()).padStart(2,"0")}`; }
function startOfToday() { const d = new Date(); d.setHours(0,0,0,0); return d; }
function rangeFilter(range) {
  const from = startOfToday();
  if (range === "week") from.setDate(from.getDate() - (from.getDay()+6)%7);
  else if (range === "7d") from.setDate(from.getDate()-6);
  else if (range === "30d") from.setDate(from.getDate()-29);
  return { ...(range === "all" ? {} : {from: from.toISOString()}), to: new Date().toISOString() };
}
function dateInputToISO(value, end) {
  if (!value) return null;
  const d = new Date(`${value}T00:00:00`);
  if (end) { d.setDate(d.getDate()+1); d.setMilliseconds(-1); }
  return d.toISOString();
}
function toast(el, message, error=false) { el.textContent = message; el.style.color = error ? "var(--danger)" : "var(--accent-2)"; el.classList.toggle("hidden", !message); }
function notify(message, error=false) { toast($("global-note"), message, error); }
function action(fn) { return async (...args) => { try { await fn(...args); } catch(e) { notify(String(e), true); } }; }
function categoryFor(app) { return config?.app_categories.find(c => c.app.toLowerCase() === app.toLowerCase())?.category || "Uncategorized"; }

function renderStatus(st) {
  lastStatus = st;
  if (st.locked && detailApp) {
    detailRequest++;
    clearAppDetails();
    toast($("app-detail-error"), "Unlock the encrypted log to view this application's history.", true);
  }
  const label = st.locked ? "locked" : st.storage_error ? "storage warning" : st.paused ? "paused" : st.excluded ? "excluded" : st.idle ? "idle" : "logging";
  $("pill-state").textContent = label;
  $("pill-state").className = "pill " + (label === "logging" ? "running" : "paused");
  $("toggle-pause").textContent = st.paused ? "Resume" : "Pause";
  $("pill-enc").textContent = st.encrypted ? "encrypted" : "plaintext";
  $("pill-enc").className = "pill " + (st.encrypted ? "enc-on" : "pill-muted");
  $("pill-current").textContent = st.current_app || "No active window";
  $("pill-current").title = st.current_title || "";
  $("pill-text").classList.toggle("hidden", !st.text_recording);
  $("text-capture-notice").classList.toggle("hidden", !st.text_capture_enabled);
  const textScope = config?.text_capture_all_apps
    ? "all supported apps, including apps you open in the future"
    : `selected apps${config?.text_capture_apps?.length ? ": " + config.text_capture_apps.join(", ") : ""}`;
  $("text-capture-notice").textContent = st.text_capture_enabled
    ? `Text recording is enabled for ${textScope}. Protected fields and excluded or recognized sensitive apps and contexts are skipped. Ordinary text fields may contain sensitive information. ${st.locked ? "Unlock the encrypted log to record." : "Pause logging or turn off text recording in Settings to stop."}`
    : "";
  $("lock-overlay").classList.toggle("hidden", !st.locked);
  $("idle-support").textContent = st.idle_detection_available ? "OS idle detection is available." : "OS idle time is unavailable in this session. Tracking uses window focus only.";
  if (st.storage_error) notify(st.storage_error, true);
  updateTextCaptureControls();
}
function updateTextCaptureControls() {
  const supported = lastStatus?.text_capture_supported === true;
  const enabled = $("s-text-capture").checked;
  const allApps = $("s-text-all").checked;
  // A profile carried to another OS must still let the user turn this off.
  $("s-text-capture").disabled = !supported && !enabled;
  $("s-text-all").disabled = !supported && !allApps;
  $("s-text-apps").disabled = allApps || (!supported && !enabled);
  $("text-selected-apps-field").classList.toggle("inactive-field", allApps);
  $("text-capture-support").textContent = supported
    ? "Available on Windows for supported editable fields. Some apps expose no usable text field."
    : "Text recording is unavailable on this platform. It is currently supported on Windows only.";
  $("text-encryption-warning").classList.toggle("hidden", !enabled || $("s-enc").checked);
}
async function refreshStatus() { renderStatus(await invoke("get_status")); }
function renderBars(element, rows, labelKey) {
  const max = Math.max(1, ...rows.map(r => r.seconds));
  element.innerHTML = rows.length ? rows.map(r => {
    const clickable = labelKey === "app";
    const tag = clickable ? "button" : "div";
    return `<${tag} class="bar-row${clickable ? " app-row" : ""}"${clickable ? ` type="button" data-open-app="${esc(r.app)}" aria-label="View ${esc(r.app)} details"` : ""}><span class="bar-app" title="${esc(r[labelKey])}">${esc(r[labelKey])}</span><span class="bar-track"><span class="bar-fill" style="width:${Math.max(0,Math.min(100,r.seconds/max*100))}%"></span></span><span class="bar-val">${fmtDur(r.seconds)}</span></${tag}>`;
  }).join("") : '<div class="empty">No activity in this range.</div>';
}
async function loadDashboard() {
  const request = ++dashboardRequest;
  const a = await invoke("analytics", {filter: rangeFilter($("range-select").value)});
  if (request !== dashboardRequest) return;
  $("stat-total").textContent = fmtDur(a.total_seconds);
  $("stat-events").textContent = a.total_events.toLocaleString();
  $("stat-top").textContent = a.per_app[0]?.app || "—";
  $("stat-top").dataset.openApp = a.per_app[0]?.app || "";
  $("stat-top").disabled = !a.per_app.length;
  $("stat-today").textContent = fmtDur(a.today_seconds);
  $("stat-week").textContent = fmtDur(a.week_seconds);
  renderBars($("app-bars"), a.per_app.slice(0,15), "app");
  renderBars($("category-bars"), a.per_category, "category");
  renderBars($("week-bars"), a.per_week.slice(-12), "week_start");
  const max = Math.max(1, ...a.per_day.map(d => d.seconds));
  $("day-bars").innerHTML = a.per_day.length ? a.per_day.slice(-30).map(d => `<div class="day-col" title="${esc(d.date)}: ${fmtDur(d.seconds)}"><div class="day-fill" style="height:${d.seconds/max*100}%"></div><div class="day-label">${esc(d.date.slice(5))}</div></div>`).join("") : '<div class="empty">No activity in this range.</div>';
  $("goal-progress").innerHTML = a.goals.length ? a.goals.map(g => `<div class="goal-item ${g.reached ? (g.kind === "limit" ? "over-limit" : "goal-reached") : ""}"><div class="inline-row"><b>${esc(g.name)}</b><span>${esc(g.target)} · ${g.period} ${g.kind}</span><span class="spacer"></span><b>${g.reached ? (g.kind === "limit" ? "Limit reached" : "Goal reached") : `${Math.round(g.progress)}%`}</b></div><progress max="100" value="${Math.min(100,g.progress)}"></progress><span>${fmtDur(g.seconds)} / ${fmtDur(g.minutes*60)}</span></div>`).join("") : '<div class="empty">Add daily or weekly goals and limits in Settings.</div>';
  $("known-apps").innerHTML = a.per_app.map(r => `<option value="${esc(r.app)}"></option>`).join("");
}
function currentLogFilter(paginated=true) {
  const f = {};
  for (const [id,key] of [["f-keyword","keyword"],["f-app","app"],["f-category","category"]]) { const v = $(id).value.trim(); if (v) f[key]=v; }
  const from = dateInputToISO($("f-from").value, false), to = dateInputToISO($("f-to").value, true);
  if (from) f.from = from; if (to) f.to = to;
  if (from && to && from > to) throw new Error("The start date must precede the end date.");
  const size = Number($("f-limit").value);
  if (paginated && size) { f.offset = page*size; f.limit = size+1; }
  return f;
}
async function loadLog() {
  const request = ++logRequest, filter = currentLogFilter();
  const rows = await invoke("query", {filter});
  if (request !== logRequest) return;
  const size = Number($("f-limit").value), hasNext = size > 0 && rows.length > size;
  if (hasNext) rows.pop();
  $("log-body").innerHTML = rows.map(r => {
    const text = typeof r.typed_text === "string" ? r.typed_text : "";
    const recording = text ? `<tr class="recorded-text-row"><td colspan="4"><details class="recorded-text"><summary>Recorded text <span class="muted">${text.length.toLocaleString()} characters</span></summary><pre>${esc(text)}</pre></details></td></tr>` : "";
    return `<tr><td>${esc(new Date(r.start).toLocaleString())}</td><td>${fmtDur(r.seconds)}</td><td><button class="app-name-link" type="button" data-open-app="${esc(r.app)}" aria-label="View ${esc(r.app)} details">${esc(r.app)} <span aria-hidden="true">↗</span></button><small class="category-tag">${esc(categoryFor(r.app))}</small></td><td class="title" title="${esc(r.title)}">${esc(r.title)}</td></tr>${recording}`;
  }).join("");
  $("log-empty").classList.toggle("hidden", rows.length > 0);
  $("prev-page").disabled = page === 0;
  $("next-page").disabled = !hasNext;
  $("page-info").textContent = rows.length ? `${size ? page*size+1 : 1}–${size ? page*size+rows.length : rows.length} matching sessions` : "No matching sessions";
}
async function applyFilter() { page=0; await loadLog(); }
async function saveDialog(name, extensions) { return invoke("plugin:dialog|save", {options:{defaultPath:name, filters:[{name:extensions.join(" / ").toUpperCase(),extensions}]}}); }
async function doExport(format) {
  const filter = currentLogFilter(false);
  const dest = await saveDialog(`daylog-${localDate(new Date())}.${format}`, [format]);
  if (!dest) return;
  const path = await invoke("export", {format,filter,dest});
  toast($("export-note"), `Exported all matching sessions to ${path}. Recorded text was omitted.`);
}
function renderSavedFilters() {
  const selected = $("saved-filter").value;
  $("saved-filter").innerHTML = '<option value="">Saved filters…</option>' + (config?.saved_filters || []).map(f => `<option value="${esc(f.id)}">${esc(f.name)}</option>`).join("");
  $("saved-filter").value = selected;
}
async function saveFilter() {
  const name = $("filter-name").value.trim(); if (!name) throw new Error("Enter a name for the saved filter.");
  const filter = currentLogFilter(false), c = await invoke("get_config");
  const existing = c.saved_filters.find(f => f.name.toLowerCase() === name.toLowerCase());
  if (existing) existing.filter = filter; else c.saved_filters.push({id:crypto.randomUUID(),name,filter});
  renderStatus(await invoke("set_config", {config:c,password:null})); config=c;
  renderSavedFilters(); $("filter-name").value=""; notify("Filter saved.");
}
async function deleteFilter() {
  const id = $("saved-filter").value; if (!id) return;
  const c = await invoke("get_config"); c.saved_filters = c.saved_filters.filter(f => f.id !== id);
  renderStatus(await invoke("set_config", {config:c,password:null})); config=c; renderSavedFilters();
}
async function loadSavedFilter() {
  const saved = config?.saved_filters.find(f => f.id === $("saved-filter").value); if (!saved) return;
  const f=saved.filter;
  $("f-keyword").value=f.keyword || ""; $("f-app").value=f.app || ""; $("f-category").value=f.category || "";
  $("f-from").value=f.from ? localDate(new Date(f.from)) : ""; $("f-to").value=f.to ? localDate(new Date(f.to)) : "";
  await applyFilter();
}
function renderCategoryOptions() {
  const selected = $("f-category").value;
  const names = [...new Set(["Uncategorized",...(config?.app_categories || []).map(c=>c.category)])].sort();
  $("f-category").innerHTML = '<option value="">All categories</option>' + names.map(n=>`<option value="${esc(n)}">${esc(n)}</option>`).join("");
  $("f-category").value = selected;
  $("known-categories").innerHTML = names.map(n=>`<option value="${esc(n)}"></option>`).join("");
  updateTargetOptions();
}
function updateTargetOptions() { $("known-targets").innerHTML = $("goal-type").value === "app" ? $("known-apps").innerHTML : $("known-categories").innerHTML; }
function renderEditors() {
  $("category-list").innerHTML = categories.map((c,i)=>`<div class="editor-row"><b>${esc(c.app)}</b><span>${esc(c.category)}</span><button class="ghost remove-category" data-index="${i}" aria-label="Remove ${esc(c.app)} category">Remove</button></div>`).join("") || '<p class="muted">No app assignments yet.</p>';
  $("goal-list").innerHTML = goals.map((g,i)=>`<div class="editor-row"><b>${esc(g.name)}</b><span>${esc(g.target)} · ${g.period} ${g.kind} · ${g.minutes} min</span><button class="ghost remove-goal" data-index="${i}" aria-label="Remove ${esc(g.name)}">Remove</button></div>`).join("") || '<p class="muted">No goals yet.</p>';
}
async function loadSettings() {
  config = await invoke("get_config"); const c=config;
  categories = structuredClone(c.app_categories); goals = structuredClone(c.time_goals);
  $("s-path").value=c.log_path; $("s-purge").value=c.auto_purge_days ?? ""; $("s-enc").checked=c.encryption_enabled;
  $("s-text-capture").checked=!!c.text_capture_enabled; $("s-text-all").checked=!!c.text_capture_all_apps; $("s-text-apps").value=(c.text_capture_apps || []).join("\n");
  $("s-excl").checked=c.exclusion_enabled; $("s-blocklist").value=c.blocklist.join("\n");
  $("s-autostart").checked=c.start_on_login; $("s-show").checked=c.show_window_on_launch; $("s-poll").value=c.poll_interval_ms;
  $("s-idle").value=c.idle_threshold_seconds ?? ""; $("s-updates").checked=c.auto_check_updates;
  const e=c.email;
  $("s-email").checked=e.enabled; $("email-host").value=e.host; $("email-port").value=e.port; $("email-user").value=e.username;
  $("email-from").value=e.sender; $("email-to").value=e.recipient; $("email-period").value=e.weekly ? "weekly" : "daily";
  $("email-hour").value=e.hour; $("email-tls").checked=e.implicit_tls;
  renderEditors(); renderCategoryOptions(); renderSavedFilters(); updateTextCaptureControls();
  if (lastStatus) renderStatus(lastStatus);
  await refreshEmailStatus();
  const enabled = await invoke("update_configured");
  $("update-status").textContent = enabled ? "Signed update feed is configured." : "This source build has no signed update feed. See docs/BUILD.md to configure release signing.";
}
function numberInput(id, nullable=false) {
  const raw=$(id).value.trim(); if (nullable && !raw) return null;
  const value=Number(raw); if (!raw || !Number.isSafeInteger(value)) throw new Error("Enter a whole number for " + id.replaceAll("-"," "));
  if (!$(id).checkValidity()) throw new Error("Value is outside the allowed range for " + id.replaceAll("-"," "));
  return value;
}
async function saveSettings() {
  const button=$("save-settings"); button.disabled=true;
  try {
    const c=await invoke("get_config"), encWanted=$("s-enc").checked, password=$("s-enc-pw").value;
    const textWanted=$("s-text-capture").checked;
    const textAllApps=$("s-text-all").checked;
    const textApps=[...new Map($("s-text-apps").value.split("\n").map(app=>app.trim()).filter(Boolean).map(app=>[app.toLowerCase(),app])).values()];
    if (textWanted && !lastStatus?.text_capture_supported) throw new Error("Text recording is currently supported on Windows only.");
    if (textWanted && !encWanted) throw new Error("Enable encryption before saving text recording, or turn text recording off.");
    if (textWanted && !textAllApps && !textApps.length) throw new Error("Select at least one exact app name for text recording, or choose All supported apps.");
    if (encWanted && (!c.encryption_enabled || c.log_path !== $("s-path").value.trim()) && !password) throw new Error("Enter the encryption password to enable encryption or migrate to a new encrypted path.");
    Object.assign(c, {log_path:$("s-path").value.trim(), encryption_enabled:encWanted, text_capture_enabled:textWanted, text_capture_all_apps:textAllApps, text_capture_apps:textApps, auto_purge_days:numberInput("s-purge",true),
      exclusion_enabled:$("s-excl").checked, blocklist:$("s-blocklist").value.split("\n").map(s=>s.trim()).filter(Boolean),
      start_on_login:$("s-autostart").checked, show_window_on_launch:$("s-show").checked, poll_interval_ms:numberInput("s-poll"),
      idle_threshold_seconds:numberInput("s-idle",true),app_categories:categories,time_goals:goals,auto_check_updates:$("s-updates").checked});
    Object.assign(c.email,{enabled:$("s-email").checked,host:$("email-host").value.trim(),port:numberInput("email-port"),username:$("email-user").value.trim(),
      sender:$("email-from").value.trim(),recipient:$("email-to").value.trim(),weekly:$("email-period").value === "weekly",hour:numberInput("email-hour"),implicit_tls:$("email-tls").checked});
    const st=await invoke("set_config", {config:c,password:password || null});
    if ($("email-password").value) await invoke("set_email_password",{password:$("email-password").value});
    $("s-enc-pw").value=""; $("email-password").value=""; renderStatus(st); await loadSettings();
    toast($("save-note"),st.storage_error || "Settings saved. Existing history was preserved.",!!st.storage_error);
  } catch(e) { toast($("save-note"),"Save failed: " + e,true); await refreshStatus(); }
  finally { button.disabled=false; }
}
async function refreshEmailStatus() {
  const s=await invoke("email_status");
  $("email-status").textContent=s.last_error || (s.sending ? "Sending…" : s.last_success ? `Last sent ${new Date(s.last_success).toLocaleString()}` : s.ready ? "SMTP password is available for this session." : "SMTP password has not been entered this session.");
  $("send-email").disabled=s.sending;
}
async function sendEmail() {
  const saved = (await invoke("get_config")).email;
  const shown = {enabled:$("s-email").checked,host:$("email-host").value.trim(),port:numberInput("email-port"),
    username:$("email-user").value.trim(),sender:$("email-from").value.trim(),recipient:$("email-to").value.trim(),
    weekly:$("email-period").value === "weekly",hour:numberInput("email-hour"),implicit_tls:$("email-tls").checked};
  if (Object.keys(shown).some(key => shown[key] !== saved[key])) throw new Error("Save the email settings before sending a summary.");
  if ($("email-password").value) { await invoke("set_email_password",{password:$("email-password").value}); $("email-password").value=""; }
  $("send-email").disabled=true;
  try { await invoke("send_summary_email"); } finally { await refreshEmailStatus(); }
}
async function previewSummary() {
  const text=await invoke("summary_preview",{weekly:$("summary-period").value === "weekly"});
  $("summary-text").textContent=text; $("summary-dialog").showModal();
}
async function checkUpdates() {
  $("check-updates").disabled=true; toast($("update-status"),"Checking…");
  try { const update=await invoke("check_updates"); if(update) showUpdate(update); else toast($("update-status"),"You are up to date."); }
  catch(e) { toast($("update-status"),String(e),true); } finally { $("check-updates").disabled=false; }
}
function showUpdate(update) { $("update-banner").classList.remove("hidden"); $("update-message").textContent=`daylog ${update.version} is available. ${update.notes || ""}`; toast($("update-status"),`Version ${update.version} is available.`); }
async function doUnlock() {
  try { renderStatus(await invoke("unlock",{password:$("unlock-pw").value})); $("unlock-pw").value=""; $("unlock-err").textContent=""; await loadSettings(); if(activeTab === "app-details")await loadAppDetails();else await loadDashboard(); }
  catch(e) { $("unlock-err").textContent=String(e); }
}
function selectPanel(name) {
  activeTab=name;
  document.querySelectorAll(".tab").forEach(t=>t.classList.toggle("active",t.dataset.tab === name));
  document.querySelectorAll(".tab-panel").forEach(p=>p.classList.toggle("active",p.id === `tab-${name}`));
}
async function switchTab(name) {
  detailRequest++;
  clearAppDetails();
  selectPanel(name);
  if(name === "dashboard") await loadDashboard(); if(name === "log") await loadLog(); if(name === "settings") await loadSettings();
}
function clearAppDetails() {
  $("app-detail-text-list").textContent="";
  $("app-detail-log-body").textContent="";
  ["app-detail-time","app-detail-sessions","app-detail-characters"].forEach(id=>$(id).textContent="—");
  $("app-detail-page").textContent="";
  $("app-detail-text-tab").textContent="Recorded text";
  $("app-detail-log-tab").textContent="Activity log";
  $("app-detail-prev").disabled=true;
  $("app-detail-next").disabled=true;
  $("app-detail-text-empty").classList.add("hidden");
  $("app-detail-log-empty").classList.add("hidden");
  $("app-detail-loading").classList.add("hidden");
}
function renderAppDetailMode() {
  const text = detailMode === "text";
  $("app-detail-text-view").classList.toggle("hidden", !text);
  $("app-detail-log-view").classList.toggle("hidden", text);
  $("app-detail-text-tab").classList.toggle("active", text);
  $("app-detail-log-tab").classList.toggle("active", !text);
  $("app-detail-text-tab").setAttribute?.("aria-pressed", String(text));
  $("app-detail-log-tab").setAttribute?.("aria-pressed", String(!text));
}
async function openAppDetails(app, source = activeTab) {
  if (!app || !app.trim()) return;
  const inherited = source === "log" ? currentLogFilter(false) : rangeFilter($("range-select").value);
  detailReturnTab = source === "log" ? "log" : "dashboard";
  detailReturnScroll = window.scrollY || 0;
  detailApp = app;
  detailMode = "text";
  detailPages = { text: 0, log: 0 };
  $("app-details-title").textContent = app;
  $("app-detail-icon").textContent = Array.from(app.trim()).slice(0,2).join("").toUpperCase();
  $("app-detail-meta").textContent = categoryFor(app);
  $("app-detail-back").textContent = detailReturnTab === "log" ? "← Back to log" : "← Back to dashboard";
  $("app-detail-from").value = inherited.from ? localDate(new Date(inherited.from)) : "";
  $("app-detail-to").value = inherited.to ? localDate(new Date(inherited.to)) : "";
  $("app-detail-keyword").value = inherited.keyword || "";
  selectPanel("app-details");
  renderAppDetailMode();
  window.scrollTo?.(0,0);
  $("app-details-title").focus?.({preventScroll:true});
  await loadAppDetails();
}
function appDetailsFilter() {
  const filter = {limit:DETAIL_PAGE_SIZE,offset:detailPages[detailMode]*DETAIL_PAGE_SIZE};
  const from=dateInputToISO($("app-detail-from").value,false), to=dateInputToISO($("app-detail-to").value,true);
  if(from)filter.from=from;if(to)filter.to=to;
  if(from && to && from>to)throw new Error("The start date must precede the end date.");
  const keyword=$("app-detail-keyword").value.trim();if(keyword)filter.keyword=keyword;
  return filter;
}
async function loadAppDetails() {
  const request=++detailRequest, app=detailApp, mode=detailMode;
  clearAppDetails(); toast($("app-detail-error"),"");
  if(!app || activeTab !== "app-details")return;
  if(lastStatus?.locked){toast($("app-detail-error"),"Unlock the encrypted log to view this application's history.",true);return;}
  $("app-detail-loading").classList.remove("hidden");
  try {
    const filter=appDetailsFilter();
    const result=await invoke("application_details",{app,filter,textOnly:mode === "text"});
    if(request !== detailRequest || activeTab !== "app-details" || lastStatus?.locked)return;
    $("app-detail-time").textContent=fmtDur(result.total_seconds);
    $("app-detail-sessions").textContent=result.total_sessions.toLocaleString();
    $("app-detail-characters").textContent=result.recorded_characters.toLocaleString();
    $("app-detail-meta").textContent=`${result.category} · ${result.last_seen ? "Last activity " + new Date(result.last_seen).toLocaleString() : "No activity in this range"}`;
    $("app-detail-text-tab").textContent=`Recorded text (${result.text_sessions.toLocaleString()})`;
    $("app-detail-log-tab").textContent=`Activity log (${result.total_sessions.toLocaleString()})`;
    const rows=result.events;
    if(mode === "text") {
      $("app-detail-text-list").innerHTML=rows.map(row=>`<article class="card app-text-entry"><div class="app-session-meta"><time>${esc(new Date(row.start).toLocaleString())}</time><span>${fmtDur(row.seconds)}</span><span>${Array.from(row.typed_text || "").length.toLocaleString()} characters</span></div><h3>${esc(row.title || "Untitled window")}</h3><pre>${esc(row.typed_text || "")}</pre></article>`).join("");
      $("app-detail-text-empty").classList.toggle("hidden",rows.length>0);
    } else {
      $("app-detail-log-body").innerHTML=rows.map(row=>`<tr><td>${esc(new Date(row.start).toLocaleString())}</td><td>${esc(new Date(row.end).toLocaleString())}</td><td>${fmtDur(row.seconds)}</td><td class="app-session-title">${esc(row.title || "Untitled window")}</td><td>${row.typed_text ? `${Array.from(row.typed_text).length.toLocaleString()} characters` : "—"}</td></tr>`).join("");
      $("app-detail-log-empty").classList.toggle("hidden",rows.length>0);
    }
    const offset=filter.offset,total=mode === "text" ? result.text_sessions : result.total_sessions;
    $("app-detail-page").textContent=rows.length ? `${offset+1}–${offset+rows.length} of ${total.toLocaleString()} ${mode === "text" ? "text sessions" : "activity logs"}` : "No matching sessions";
    $("app-detail-prev").disabled=detailPages[mode]===0;
    $("app-detail-next").disabled=!result.has_more;
  } catch(error) {
    if(request===detailRequest && activeTab === "app-details")toast($("app-detail-error"),String(error),true);
  } finally {
    if(request===detailRequest)$("app-detail-loading").classList.add("hidden");
  }
}
async function applyAppDetailsFilter() { detailPages={text:0,log:0}; await loadAppDetails(); }
async function changeAppDetailMode(mode) { detailMode=mode; renderAppDetailMode(); await loadAppDetails(); }
async function openAppFromClick(event, source) {
  const button=event.target.closest("[data-open-app]");
  if(button?.dataset.openApp)await openAppDetails(button.dataset.openApp,source);
}

function wire() {
  document.querySelectorAll(".tab").forEach(t=>t.addEventListener("click",action(()=>switchTab(t.dataset.tab))));
  const on=(id,event,fn)=>$(id).addEventListener(event,action(fn));
  on("app-bars","click",event=>openAppFromClick(event,"dashboard"));
  on("log-body","click",event=>openAppFromClick(event,"log"));
  on("stat-top","click",()=>openAppDetails($("stat-top").dataset.openApp,"dashboard"));
  on("app-detail-back","click",async()=>{
    await switchTab(detailReturnTab);
    window.scrollTo?.(0,detailReturnScroll);
    const buttons=Array.from($(`tab-${detailReturnTab}`).querySelectorAll?.("[data-open-app]") || []);
    const target=buttons.find(button=>button.dataset.openApp === detailApp)
      || Array.from(document.querySelectorAll(".tab")).find(tab=>tab.dataset.tab === detailReturnTab);
    target?.focus?.({preventScroll:true});
  });
  on("app-detail-refresh","click",loadAppDetails);
  on("app-detail-text-tab","click",()=>changeAppDetailMode("text"));
  on("app-detail-log-tab","click",()=>changeAppDetailMode("log"));
  on("app-detail-prev","click",async()=>{detailPages[detailMode]=Math.max(0,detailPages[detailMode]-1);await loadAppDetails();});
  on("app-detail-next","click",async()=>{detailPages[detailMode]++;await loadAppDetails();});
  on("app-detail-apply","click",applyAppDetailsFilter);
  ["app-detail-from","app-detail-to"].forEach(id=>on(id,"change",applyAppDetailsFilter));
  on("app-detail-keyword","keydown",event=>{if(event.key === "Enter")return applyAppDetailsFilter();});
  on("app-detail-reset","click",async()=>{["app-detail-from","app-detail-to","app-detail-keyword"].forEach(id=>$(id).value="");await applyAppDetailsFilter();});
  on("toggle-pause","click",async()=>{const s=await invoke("get_status"); renderStatus(await invoke(s.paused?"resume":"pause"));});
  on("refresh-dash","click",loadDashboard); on("range-select","change",loadDashboard); on("apply-filter","click",applyFilter); on("f-limit","change",applyFilter); on("f-category","change",applyFilter);
  ["f-keyword","f-app"].forEach(id=>on(id,"input",()=>{clearTimeout(logDebounce);logDebounce=setTimeout(action(applyFilter),250);}));
  ["f-from","f-to"].forEach(id=>on(id,"change",applyFilter));
  on("prev-page","click",async()=>{page=Math.max(0,page-1);await loadLog();}); on("next-page","click",async()=>{page++;await loadLog();});
  on("export-csv","click",()=>doExport("csv")); on("export-md","click",()=>doExport("md"));
  on("save-filter","click",saveFilter); on("delete-saved","click",deleteFilter); on("load-saved","click",loadSavedFilter); on("saved-filter","change",loadSavedFilter);
  on("reset-filter","click",async()=>{["f-keyword","f-app","f-from","f-to","f-category"].forEach(id=>$(id).value="");await applyFilter();});
  on("clear-logs","click",async()=>{if(!confirm("Delete ALL logged entries? This cannot be undone."))return;renderStatus(await invoke("clear_logs"));await loadLog();await loadDashboard();});
  on("open-folder","click",()=>invoke("reveal_log")); on("save-settings","click",saveSettings);
  on("s-text-capture","change",updateTextCaptureControls); on("s-text-all","change",updateTextCaptureControls); on("s-enc","change",updateTextCaptureControls);
  on("clear-recorded-text","click",async()=>{
    if(!confirm("Delete all recorded text and turn off text recording? Activity times remain."))return;
    renderStatus(await invoke("clear_recorded_text")); await loadSettings(); await loadLog();
    notify("Recorded text deleted. App names, window titles, and activity times remain.");
  });
  on("add-category","click",()=>{
    const app=$("category-app").value.trim(),category=$("category-name").value.trim();if(!app || !category)throw new Error("Enter an app name and category.");
    const existing=categories.find(c=>c.app.toLowerCase() === app.toLowerCase()); if(existing)existing.category=category;else categories.push({app,category});
    $("category-app").value="";$("category-name").value="";renderEditors();toast($("save-note"),"Category added. Save settings to apply.");
  });
  on("add-goal","click",()=>{
    const name=$("goal-name").value.trim(),target=$("goal-target").value.trim();if(!name || !target)throw new Error("Enter a goal name and target.");
    const minutes=numberInput("goal-minutes");goals.push({id:crypto.randomUUID(),name,target,target_type:$("goal-type").value,period:$("goal-period").value,kind:$("goal-kind").value,minutes});
    ["goal-name","goal-target","goal-minutes"].forEach(id=>$(id).value="");renderEditors();toast($("save-note"),"Goal added. Save settings to apply.");
  });
  on("category-list","click",e=>{const b=e.target.closest(".remove-category");if(b){categories.splice(Number(b.dataset.index),1);renderEditors();}});
  on("goal-list","click",e=>{const b=e.target.closest(".remove-goal");if(b){goals.splice(Number(b.dataset.index),1);renderEditors();}});
  on("goal-type","change",updateTargetOptions); on("preview-summary","click",previewSummary);on("close-summary","click",()=>$("summary-dialog").close());
  on("save-summary","click",async()=>{const dest=await saveDialog(`daylog-summary-${localDate(new Date())}.txt`,["txt"]);if(dest){await invoke("export_summary",{weekly:$("summary-period").value === "weekly",dest});notify(`Summary saved to ${dest}`);}});
  on("send-email","click",sendEmail);
  on("check-updates","click",checkUpdates);
  on("install-update","click",async()=>{if(!confirm("Install the signed update and restart daylog?"))return;$("install-update").disabled=true;try{await invoke("install_update");}catch(e){$("install-update").disabled=false;throw e;}});
  on("unlock-btn","click",doUnlock);on("unlock-pw","keydown",e=>{if(e.key === "Enter")return doUnlock();});
  listen("daylog://status",e=>renderStatus(e.payload)); listen("daylog://update-available",e=>showUpdate(e.payload)); listen("daylog://update-error",e=>toast($("update-status"),String(e.payload),true));
  listen("daylog://update-progress",()=>{$("install-update").textContent="Downloading and verifying…";});
}
async function boot() {
  wire();await refreshStatus();if(!lastStatus.locked){await loadSettings();await loadDashboard();}
  setInterval(action(async()=>{if(!lastStatus?.locked && activeTab === "dashboard")await loadDashboard();if(activeTab === "settings")await refreshEmailStatus();}),15000);
}
boot().catch(e=>notify(String(e),true));
