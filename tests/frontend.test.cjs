const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const crypto = require("node:crypto");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "ui/index.html"), "utf8");
const script = fs.readFileSync(path.join(root, "ui/app.js"), "utf8");
const clone = value => structuredClone(value);
const decode = value => value.replace(/&quot;|&#39;|&amp;|&lt;|&gt;/g,
  entity => ({ "&quot;": '"', "&#39;": "'", "&amp;": "&", "&lt;": "<", "&gt;": ">" }[entity]));

function attributes(source) {
  const values = {};
  for (const match of source.matchAll(/([\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'))?/g)) {
    values[match[1]] = decode(match[2] ?? match[3] ?? "");
  }
  return values;
}

// The adapter models only the DOM surface app.js uses. It loads IDs, types,
// bounds, options and classes from the shipped HTML, so missing controls fail
// at runtime. It deliberately does not claim to test layout or native WebView.
class Element {
  constructor(tag, attrs) {
    this.tagName = tag.toUpperCase();
    this.attrs = attrs;
    this.id = attrs.id || "";
    this.className = attrs.class || "";
    this.dataset = Object.fromEntries(Object.entries(attrs)
      .filter(([key]) => key.startsWith("data-"))
      .map(([key, value]) => [key.slice(5).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase()), value]));
    this.style = {};
    this.title = attrs.title || "";
    this.disabled = "disabled" in attrs;
    this.checked = "checked" in attrs;
    this.listeners = new Map();
    this.options = null;
    this._value = attrs.value || "";
    this._textContent = "";
    this._innerHTML = "";
    this.open = false;
    this.classList = {
      contains: name => this.className.split(/\s+/).includes(name),
      toggle: (name, force) => {
        const names = new Set(this.className.split(/\s+/).filter(Boolean));
        const include = force ?? !names.has(name);
        include ? names.add(name) : names.delete(name);
        this.className = [...names].join(" ");
        return include;
      },
      add: name => this.classList.toggle(name, true),
      remove: name => this.classList.toggle(name, false),
    };
  }
  get value() { return this._value; }
  set value(value) {
    const string = String(value);
    this._value = this.options && !this.options.includes(string) ? "" : string;
  }
  get textContent() { return this._textContent; }
  set textContent(value) {
    this._textContent = String(value);
    this._innerHTML = this._textContent.replace(/[&<>]/g, character => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[character]));
  }
  get innerHTML() { return this._innerHTML; }
  set innerHTML(value) {
    this._innerHTML = String(value);
    this._textContent = decode(this._innerHTML.replace(/<[^>]*>/g, ""));
    if (this.tagName === "SELECT") {
      const options = [...this._innerHTML.matchAll(/<option\b([^>]*)>([\s\S]*?)<\/option>/g)]
        .map(([, attrs, text]) => ({ attrs: attributes(attrs), text: decode(text) }));
      this.options = options.map(option => option.attrs.value ?? option.text);
      const selected = options.find(option => "selected" in option.attrs) || options[0];
      this._value = selected ? selected.attrs.value ?? selected.text : "";
    }
  }
  checkValidity() {
    if (this.attrs.type !== "number" || !this.value) return true;
    const value = Number(this.value);
    const min = this.attrs.min === undefined ? -Infinity : Number(this.attrs.min);
    const max = this.attrs.max === undefined ? Infinity : Number(this.attrs.max);
    if (!Number.isFinite(value) || value < min || value > max) return false;
    if (this.attrs.step === "any") return true;
    const step = Number(this.attrs.step || 1), base = Number.isFinite(min) ? min : 0;
    return Math.abs((value - base) / step - Math.round((value - base) / step)) < 1e-8;
  }
  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(listener);
  }
  async dispatch(type, event = {}) {
    if (this.disabled && type === "click") return;
    for (const listener of this.listeners.get(type) || []) await listener({ target: this, ...event });
  }
  matches(selector) {
    const tag = selector.match(/^[a-z][\w-]*/i)?.[0];
    if (tag && this.tagName !== tag.toUpperCase()) return false;
    for (const [, name] of selector.matchAll(/\.([\w-]+)/g)) if (!this.classList.contains(name)) return false;
    for (const [, name] of selector.matchAll(/#([\w-]+)/g)) if (this.id !== name) return false;
    for (const [, key, quoted, unquoted] of selector.matchAll(/\[([\w-]+)(?:=(?:"([^"]*)"|'([^']*)'))?\]/g)) {
      if (!(key in this.attrs) || ((quoted ?? unquoted) !== undefined && this.attrs[key] !== (quoted ?? unquoted))) return false;
    }
    return true;
  }
  closest(selector) { return this.matches(selector) ? this : null; }
  setAttribute(key, value) { this.attrs[key] = String(value); }
  getAttribute(key) { return this.attrs[key] ?? null; }
  removeAttribute(key) { delete this.attrs[key]; }
  focus() { if (this.ownerDocument) this.ownerDocument.activeElement = this; }
  showModal() { this.open = true; }
  close() { this.open = false; }
}

function makeDocument() {
  const elements = [...html.matchAll(/<([a-z][\w-]*)\b([^>]*)>/gi)]
    .map(([, tag, attrs]) => new Element(tag, attributes(attrs)));
  const ids = new Map(elements.filter(element => element.id).map(element => [element.id, element]));
  for (const [, , attrs, contents] of html.matchAll(/<([a-z][\w-]*)\b([^>]*\bid="[^"]+"[^>]*)>([^<]*)<\/\1>/gi)) {
    const id = attributes(attrs).id;
    if (id) ids.get(id).textContent = decode(contents);
  }
  for (const [, attrs, contents] of html.matchAll(/<select\b([^>]*)>([\s\S]*?)<\/select>/gi)) {
    const id = attributes(attrs).id;
    if (id) ids.get(id).innerHTML = contents;
  }
  const document = {
    ids,
    activeElement: null,
    getElementById: id => ids.get(id) || null,
    querySelectorAll: selector => elements.filter(element => element.matches(selector)),
  };
  for (const element of elements) element.ownerDocument = document;
  return document;
}

function defaultConfig() {
  return {
    log_path: "C:/daylog/activity.log", encryption_enabled: false, exclusion_enabled: false,
    blocklist: [], start_on_login: false, auto_purge_days: null, poll_interval_ms: 1500,
    show_window_on_launch: true, idle_threshold_seconds: 300, app_categories: [],
    time_goals: [], saved_filters: [], auto_check_updates: false,
    text_capture_enabled: false, text_capture_all_apps: false, text_capture_apps: [],
    email: { enabled: false, host: "", port: 587, username: "", sender: "", recipient: "",
      weekly: false, hour: 9, implicit_tls: false, last_sent: null },
  };
}

function defaultStatus() {
  return { paused: false, encrypted: false, locked: false, current_app: "Code", current_title: "main.rs",
    events_total: 2, log_path: "C:/daylog/activity.log", idle: false, idle_detection_available: true,
    excluded: false, storage_error: null, text_capture_enabled: false, text_recording: false, text_capture_supported: true };
}

async function launch(options = {}) {
  const document = makeDocument(), calls = [], events = new Map(), intervals = [], timers = new Map();
  let serverConfig = { ...defaultConfig(), ...clone(options.config || {}) };
  for (const field of options.omitConfigFields || []) delete serverConfig[field];
  let status = { ...defaultStatus(), encrypted: serverConfig.encryption_enabled,
    text_capture_enabled: serverConfig.text_capture_enabled, ...options.status };
  let activityRows = options.rows ? clone(options.rows) : null;
  let timerId = 0;
  const analytics = { total_seconds: 3600, total_events: 2, today_seconds: 1800, week_seconds: 3600,
    per_app: [{ app: "Code", seconds: 3600, events: 2 }], per_category: [{ category: "Work", seconds: 3600 }],
    per_day: [{ date: "2026-11-04", seconds: 3600 }], per_week: [{ week_start: "2026-11-02", seconds: 3600 }], goals: [],
    ...clone(options.analytics || {}) };
  async function invoke(command, args = {}) {
    calls.push({ command, args: clone(args) });
    if (options.handlers?.[command]) return options.handlers[command](args);
    switch (command) {
      case "get_config": return clone(serverConfig);
      case "get_status": return clone(status);
      case "set_config": {
        if (args.config.text_capture_enabled) {
          if (!args.config.encryption_enabled) throw new Error("Text recording requires encryption");
          if (!args.config.text_capture_all_apps && !args.config.text_capture_apps.length) throw new Error("Select at least one app");
        }
        if (!args.config.encryption_enabled && activityRows?.some(row => row.typed_text)) {
          throw new Error("Delete recorded text before disabling encryption");
        }
        const storageChanged = args.config.log_path !== serverConfig.log_path
          || args.config.encryption_enabled !== serverConfig.encryption_enabled;
        if (storageChanged && args.config.encryption_enabled) {
          if (!args.password) throw new Error("password required to enable encryption");
          if (options.destinationPassword && args.password !== options.destinationPassword) {
            throw new Error("could not unlock destination: incorrect password");
          }
        }
        serverConfig = clone(args.config);
        status.encrypted = serverConfig.encryption_enabled;
        status.text_capture_enabled = serverConfig.text_capture_enabled;
        status.text_recording = false;
        status.log_path = serverConfig.log_path;
        return clone(status);
      }
      case "set_encryption": serverConfig.encryption_enabled = status.encrypted = args.enabled; return clone(status);
      case "analytics": return clone(analytics);
      case "email_status": return { ready: false, sending: false, last_error: null, last_success: null };
      case "update_configured": return false;
      case "check_updates": throw new Error("No signed update feed is configured");
      case "query": {
        if (activityRows) return clone(activityRows.slice(args.filter.offset || 0,
          args.filter.limit ? (args.filter.offset || 0) + args.filter.limit : undefined));
        const remaining = Math.max(0, 450 - (args.filter.offset || 0));
        return Array.from({ length: Math.min(remaining, args.filter.limit ?? remaining) }, (_, index) => ({
          id: index, start: "2026-11-04T08:00:00Z", end: "2026-11-04T08:00:30Z", seconds: 30,
          app: "Code", title: `main-${index}.rs`,
        }));
      }
      case "application_details": {
        const filter = args.filter || {};
        let rows = activityRows || [{ id: 1, start: "2026-11-04T08:00:00Z", end: "2026-11-04T09:00:00Z",
          seconds: 3600, app: "Code", title: "main.rs", typed_text: null }];
        rows = rows.filter(row => row.app.toLowerCase() === args.app.trim().toLowerCase());
        if (filter.from) rows = rows.filter(row => new Date(row.end || new Date(new Date(row.start).getTime() + row.seconds * 1000)) > new Date(filter.from));
        if (filter.to) rows = rows.filter(row => new Date(row.start) <= new Date(filter.to));
        if (filter.keyword) rows = rows.filter(row => [row.app, row.title, row.typed_text || ""].join(" ").toLowerCase().includes(filter.keyword.toLowerCase()));
        const appName = rows[0]?.app || args.app;
        const category = serverConfig.app_categories.find(entry => entry.app.toLowerCase() === args.app.toLowerCase())?.category || "Uncategorized";
        if (filter.category && category.toLowerCase() !== filter.category.toLowerCase()) rows = [];
        const paged = args.textOnly ? rows.filter(row => row.typed_text) : rows;
        const limit = Math.min(200, Math.max(1, filter.limit || 50)), offset = filter.offset || 0;
        return clone({ app: appName, category, total_seconds: rows.reduce((sum, row) => sum + row.seconds, 0),
          total_sessions: rows.length, text_sessions: rows.filter(row => row.typed_text).length,
          recorded_characters: rows.reduce((sum, row) => sum + Array.from(row.typed_text || "").length, 0),
          last_seen: rows[0]?.end || rows[0]?.start || null, events: paged.slice(offset, offset + limit), has_more: paged.length > offset + limit });
      }
      case "plugin:dialog|save": return "C:/exports/daylog.csv";
      case "export": case "export_summary": return args.dest;
      case "pause": status.paused = true; return clone(status);
      case "resume": status.paused = false; return clone(status);
      case "unlock": status.locked = false; return clone(status);
      case "clear_recorded_text":
        serverConfig.text_capture_enabled = status.text_capture_enabled = status.text_recording = false;
        if (activityRows) activityRows = activityRows.map(row => ({ ...row, typed_text: null }));
        return clone(status);
      case "set_email_password": case "send_summary_email": case "reveal_log": return null;
      case "summary_preview": return "daylog summary\nCode: 1h\nWork: 1h";
      default: throw new Error(`Unexpected IPC command: ${command}`);
    }
  }
  const context = vm.createContext({
    document, window: { __TAURI__: { core: { invoke }, event: { listen: (name, fn) => { events.set(name, fn); return Promise.resolve(() => {}); } } } },
    structuredClone, crypto, confirm: () => true,
    setInterval: callback => { intervals.push(callback); return intervals.length; },
    setTimeout: callback => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: id => timers.delete(id),
  });
  vm.runInContext(script, context, { filename: "ui/app.js" });
  await new Promise(resolve => setImmediate(resolve));
  return {
    calls, events, document, context,
    element: id => { const element = document.getElementById(id); assert.ok(element, `Missing control ${id}`); return element; },
    click: async id => document.getElementById(id).dispatch("click"),
    getConfig: () => clone(serverConfig),
    latest: command => calls.filter(call => call.command === command).at(-1),
    emit: async (name, payload) => { events.get(name)({ payload }); await new Promise(resolve => setImmediate(resolve)); },
    clickWithin: async (id, selector, matching = () => true) => {
      const parent = document.getElementById(id);
      assert.ok(parent, `Missing control ${id}`);
      const candidates = [...parent.innerHTML.matchAll(/<([a-z][\w-]*)\b([^>]*)>/gi)]
        .map(([, tag, attrs]) => new Element(tag, attributes(attrs)));
      const target = candidates.find(element => element.matches(selector) && matching(element));
      assert.ok(target, `Missing ${selector} in ${id}`);
      await parent.dispatch("click", { target });
    },
    flushTimers: async () => {
      const pending = [...timers.values()]; timers.clear();
      for (const callback of pending) await callback();
    },
  };
}

test("shipped HTML has unique IDs and every literal app.js ID exists", () => {
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(match => match[1]);
  assert.equal(new Set(ids).size, ids.length, "Duplicate IDs in index.html");
  for (const [, id] of script.matchAll(/(?:\$|\bon)\("([^"]+)"/g)) assert.ok(ids.includes(id), `Missing ID: ${id}`);
});

test("app boots, wires controls, and renders analytics with no missing elements", async () => {
  const app = await launch();
  assert.equal(app.element("global-note").textContent, "");
  assert.equal(app.element("stat-total").textContent, "1h 0m");
  assert.equal(app.element("pill-state").textContent, "logging");
  assert.match(app.element("app-bars").innerHTML, /Code/);
  assert.ok(app.events.has("daylog://status"));
  assert.equal(app.calls.filter(call => call.command === "check_updates").length, 0);
});

test("saving encrypted settings without changing the path needs no password", async () => {
  const app = await launch({ config: { encryption_enabled: true } });
  app.element("s-idle").value = "600";
  await app.click("save-settings");
  assert.equal(app.latest("set_config").args.password, null);
  assert.equal(app.getConfig().idle_threshold_seconds, 600);
  assert.equal(app.calls.filter(call => call.command === "set_encryption").length, 0);
  assert.match(app.element("save-note").textContent, /Settings saved/);
  assert.equal(app.element("save-settings").disabled, false);
});

test("enabling encryption without a password leaves saved config untouched", async () => {
  const app = await launch();
  app.element("s-enc").checked = true;
  await app.click("save-settings");
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 0);
  assert.match(app.element("save-note").textContent, /Enter the encryption password/);
  assert.equal(app.getConfig().encryption_enabled, false);
});

test("disabling encryption and moving the log commits together without a password", async () => {
  const app = await launch({ config: { encryption_enabled: true } });
  app.element("s-enc").checked = false;
  app.element("s-path").value = "C:/daylog/moved/activity.log";
  app.element("s-idle").value = "600";
  await app.click("save-settings");
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 1);
  assert.equal(app.calls.filter(call => call.command === "set_encryption").length, 0);
  assert.equal(app.latest("set_config").args.password, null);
  assert.equal(app.getConfig().encryption_enabled, false);
  assert.equal(app.getConfig().log_path, "C:/daylog/moved/activity.log");
  assert.equal(app.getConfig().idle_threshold_seconds, 600);
  assert.match(app.element("save-note").textContent, /Settings saved/);
});

test("enabling encryption and moving the log uses one atomic settings command", async () => {
  const app = await launch();
  app.element("s-enc").checked = true;
  app.element("s-enc-pw").value = "destination password";
  app.element("s-path").value = "C:/daylog/encrypted/activity.log";
  await app.click("save-settings");
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 1);
  assert.equal(app.calls.filter(call => call.command === "set_encryption").length, 0);
  assert.equal(app.getConfig().encryption_enabled, true);
  assert.equal(app.getConfig().log_path, "C:/daylog/encrypted/activity.log");
  assert.equal(app.element("s-enc-pw").value, "");
  assert.match(app.element("save-note").textContent, /Settings saved/);
});

test("wrong destination credentials leave the log path, encryption, and other settings unchanged", async () => {
  const app = await launch({ destinationPassword: "correct password" });
  app.element("s-enc").checked = true;
  app.element("s-enc-pw").value = "incorrect password";
  app.element("s-path").value = "C:/daylog/existing/activity.log";
  app.element("s-idle").value = "600";
  await app.click("save-settings");
  assert.equal(app.getConfig().log_path, "C:/daylog/activity.log");
  assert.equal(app.getConfig().encryption_enabled, false);
  assert.equal(app.getConfig().idle_threshold_seconds, 300);
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 1);
  assert.equal(app.calls.filter(call => call.command === "set_encryption").length, 0);
  assert.match(app.element("save-note").textContent, /incorrect password/);
  assert.equal(app.element("save-settings").disabled, false);
});

test("filters reset pagination and exports include all matches beyond the page cap", async () => {
  const app = await launch();
  await app.click("apply-filter");
  await app.click("next-page");
  assert.equal(app.latest("query").args.filter.offset, 200);
  app.element("f-keyword").value = '"project planning" -meeting';
  app.element("f-app").value = "Code";
  await app.click("apply-filter");
  assert.equal(app.latest("query").args.filter.offset, 0);
  await app.click("next-page");
  await app.click("export-csv");
  const filter = app.latest("export").args.filter;
  assert.equal(filter.keyword, '"project planning" -meeting');
  assert.equal(filter.app, "Code");
  assert.equal("offset" in filter, false);
  assert.equal("limit" in filter, false);
  assert.match(app.element("export-note").textContent, /all matching sessions/);
});

test("saved filters persist, restore combined fields, update by name, and delete", async () => {
  const app = await launch({ config: { app_categories: [{ app: "Code", category: "Work" }] } });
  app.element("f-keyword").value = "project -meeting";
  app.element("f-app").value = "Code";
  app.element("f-category").value = "Work";
  app.element("f-from").value = "2026-11-01";
  app.element("f-to").value = "2026-11-04";
  app.element("filter-name").value = "My work";
  await app.click("save-filter");
  let saved = app.getConfig().saved_filters[0];
  assert.ok(saved.id);
  assert.equal(saved.filter.category, "Work");
  assert.equal("limit" in saved.filter, false);
  await app.click("reset-filter");
  app.element("saved-filter").value = saved.id;
  await app.click("load-saved");
  assert.equal(app.element("f-keyword").value, "project -meeting");
  assert.equal(app.element("f-from").value, "2026-11-01");
  assert.equal(app.element("f-to").value, "2026-11-04");
  assert.equal(app.latest("query").args.filter.offset, 0);
  app.element("f-keyword").value = "changed";
  app.element("filter-name").value = "MY WORK";
  await app.click("save-filter");
  assert.equal(app.getConfig().saved_filters.length, 1);
  assert.equal(app.getConfig().saved_filters[0].filter.keyword, "changed");
  app.element("saved-filter").value = saved.id;
  await app.click("delete-saved");
  assert.equal(app.getConfig().saved_filters.length, 0);
});

test("app category assignments and weekly limits can be edited and persisted", async () => {
  const app = await launch();
  app.element("category-app").value = "Code";
  app.element("category-name").value = "Work";
  await app.click("add-category");
  app.element("category-app").value = "code";
  app.element("category-name").value = "Development";
  await app.click("add-category");
  app.element("goal-name").value = "Weekly focus";
  app.element("goal-type").value = "category";
  app.element("goal-target").value = "Development";
  app.element("goal-period").value = "weekly";
  app.element("goal-kind").value = "limit";
  app.element("goal-minutes").value = "600";
  await app.click("add-goal");
  await app.click("save-settings");
  const config = app.getConfig();
  assert.deepEqual(config.app_categories, [{ app: "Code", category: "Development" }]);
  assert.equal(config.time_goals[0].kind, "limit");
  assert.equal(config.time_goals[0].period, "weekly");
  assert.equal(config.time_goals[0].target_type, "category");
  assert.equal(config.time_goals[0].minutes, 600);
  for (const id of ["category-list", "goal-list"]) {
    await app.element(id).dispatch("click", { target: { closest: () => ({ dataset: { index: "0" } }) } });
  }
  await app.click("save-settings");
  assert.equal(app.getConfig().app_categories.length, 0);
  assert.equal(app.getConfig().time_goals.length, 0);
});

test("status events distinguish idle, exclusion, pause, and the encrypted lock gate", async () => {
  const app = await launch();
  for (const [override, label] of [
    [{ idle: true }, "idle"],
    [{ excluded: true, current_title: null }, "excluded"],
    [{ paused: true }, "paused"],
    [{ locked: true, encrypted: true }, "locked"],
  ]) {
    await app.emit("daylog://status", { ...defaultStatus(), ...override });
    assert.equal(app.element("pill-state").textContent, label);
    assert.equal(app.element("lock-overlay").classList.contains("hidden"), label !== "locked");
    if (label === "excluded") assert.equal(app.element("pill-current").title, "");
  }
  await app.emit("daylog://status", { ...defaultStatus(), idle_detection_available: false });
  assert.match(app.element("idle-support").textContent, /unavailable/);
});

test("locked boot avoids analytics until the log is unlocked", async () => {
  const app = await launch({ config: { encryption_enabled: true }, status: { locked: true } });
  assert.equal(app.calls.filter(call => call.command === "analytics").length, 0);
  assert.equal(app.element("lock-overlay").classList.contains("hidden"), false);
  app.element("unlock-pw").value = "session password";
  await app.click("unlock-btn");
  assert.equal(app.element("unlock-pw").value, "");
  assert.equal(app.element("lock-overlay").classList.contains("hidden"), true);
  assert.equal(app.calls.filter(call => call.command === "analytics").length, 1);
});

test("a source build reports an absent update feed without starting an install", async () => {
  const app = await launch();
  assert.match(app.element("update-status").textContent, /no signed update feed/);
  await app.click("check-updates");
  assert.match(app.element("update-status").textContent, /No signed update feed/);
  assert.equal(app.element("check-updates").disabled, false);
  assert.equal(app.element("update-banner").classList.contains("hidden"), true);
  assert.equal(app.calls.filter(call => call.command === "install_update").length, 0);
});

test("text recording starts off with no app opt-ins or recording indicator", async () => {
  const app = await launch();
  assert.equal(app.element("s-text-capture").checked, false);
  assert.equal(app.element("s-text-all").checked, false);
  assert.equal(app.element("s-text-apps").value, "");
  assert.equal(app.element("pill-text").classList.contains("hidden"), true);
  assert.equal(app.element("text-capture-notice").classList.contains("hidden"), true);
});

test("all supported apps is an explicit encrypted opt-in with no selected app list required", async () => {
  const app = await launch();
  app.element("s-text-capture").checked = true;
  app.element("s-text-all").checked = true;
  await app.element("s-text-all").dispatch("change");
  assert.equal(app.element("s-text-apps").disabled, true);
  app.element("s-enc").checked = true;
  app.element("s-enc-pw").value = "new encryption password";
  await app.click("save-settings");
  const saved = app.getConfig();
  assert.equal(saved.encryption_enabled, true);
  assert.equal(saved.text_capture_enabled, true);
  assert.equal(saved.text_capture_all_apps, true);
  assert.deepEqual(saved.text_capture_apps, []);
  assert.equal(app.latest("set_config").args.password, "new encryption password");
  assert.equal(app.element("s-enc-pw").value, "");
  assert.match(app.element("text-capture-notice").textContent, /all supported apps, including apps you open in the future/);
  assert.match(app.element("text-capture-notice").textContent, /Protected fields and excluded or recognized sensitive apps and contexts are skipped/);
  assert.equal(app.element("pill-text").classList.contains("hidden"), true);
});

test("all-app mode still requires encryption before text recording can be enabled", async () => {
  const app = await launch();
  app.element("s-text-capture").checked = true;
  app.element("s-text-all").checked = true;
  await app.click("save-settings");
  assert.match(app.element("save-note").textContent, /Enable encryption/);
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 0);
  assert.equal(app.getConfig().text_capture_enabled, false);
});

test("all-app preference can be staged while recording remains off", async () => {
  const app = await launch();
  assert.equal(app.element("s-text-all").disabled, false);
  app.element("s-text-all").checked = true;
  await app.element("s-text-all").dispatch("change");
  await app.click("save-settings");
  assert.equal(app.getConfig().text_capture_all_apps, true);
  assert.equal(app.getConfig().text_capture_enabled, false);
  assert.equal(app.getConfig().encryption_enabled, false);
  assert.equal(app.element("text-capture-notice").classList.contains("hidden"), true);
  assert.equal(app.element("pill-text").classList.contains("hidden"), true);
});

test("switching from all apps back to selected apps requires exact app names", async () => {
  const app = await launch({ config: { encryption_enabled: true, text_capture_enabled: true,
    text_capture_all_apps: true, text_capture_apps: [] } });
  app.element("s-text-all").checked = false;
  await app.element("s-text-all").dispatch("change");
  assert.equal(app.element("s-text-apps").disabled, false);
  await app.click("save-settings");
  assert.match(app.element("save-note").textContent, /at least one exact app name/);
  assert.equal(app.getConfig().text_capture_all_apps, true);
  app.element("s-text-apps").value = "Notepad";
  await app.click("save-settings");
  assert.equal(app.getConfig().text_capture_all_apps, false);
  assert.deepEqual(app.getConfig().text_capture_apps, ["Notepad"]);
  assert.match(app.element("text-capture-notice").textContent, /selected apps: Notepad/);
});

test("switching capture scope preserves the selected-app list and disabling keeps the preference", async () => {
  const app = await launch({ config: { encryption_enabled: true, text_capture_enabled: true,
    text_capture_apps: ["Notepad", "Code"] } });
  app.element("s-text-all").checked = true;
  await app.element("s-text-all").dispatch("change");
  await app.click("save-settings");
  assert.deepEqual(app.getConfig().text_capture_apps, ["Notepad", "Code"]);
  assert.equal(app.element("s-text-apps").value, "Notepad\nCode");
  assert.equal(app.element("s-text-apps").disabled, true);
  app.element("s-text-all").checked = false;
  await app.element("s-text-all").dispatch("change");
  assert.equal(app.element("s-text-apps").disabled, false);
  await app.click("save-settings");
  assert.deepEqual(app.getConfig().text_capture_apps, ["Notepad", "Code"]);
  app.element("s-text-all").checked = true;
  app.element("s-text-capture").checked = false;
  await app.click("save-settings");
  assert.equal(app.getConfig().text_capture_all_apps, true);
  assert.equal(app.getConfig().text_capture_enabled, false);
  assert.equal(app.element("text-capture-notice").classList.contains("hidden"), true);
});

test("legacy settings without the all-app flag retain selected-app mode", async () => {
  const app = await launch({ omitConfigFields: ["text_capture_all_apps"], config: {
    encryption_enabled: true, text_capture_enabled: true, text_capture_apps: ["Notepad"],
  } });
  assert.equal(app.element("s-text-all").checked, false);
  assert.equal(app.element("s-text-apps").disabled, false);
  assert.match(app.element("text-capture-notice").textContent, /selected apps: Notepad/);
  await app.click("save-settings");
  assert.equal(app.getConfig().text_capture_all_apps, false);
});

test("unsupported platforms cannot opt into all apps and can turn an existing preference off", async () => {
  const app = await launch({ status: { text_capture_supported: false } });
  assert.equal(app.element("s-text-all").disabled, true);
  const existing = await launch({ status: { text_capture_supported: false }, config: {
    text_capture_all_apps: true, text_capture_enabled: false,
  } });
  assert.equal(existing.element("s-text-all").disabled, false);
  existing.element("s-text-all").checked = false;
  await existing.element("s-text-all").dispatch("change");
  await existing.click("save-settings");
  assert.equal(existing.getConfig().text_capture_all_apps, false);
});

test("recording text requires encryption and explicit app selections", async () => {
  const app = await launch();
  app.element("s-text-capture").checked = true;
  app.element("s-text-apps").value = "Notepad";
  await app.click("save-settings");
  assert.match(app.element("save-note").textContent, /Enable encryption/);
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 0);
  app.element("s-enc").checked = true;
  app.element("s-enc-pw").value = "new encryption password";
  app.element("s-text-apps").value = " \n ";
  await app.click("save-settings");
  assert.match(app.element("save-note").textContent, /at least one exact app name/);
  assert.equal(app.calls.filter(call => call.command === "set_config").length, 0);
});

test("explicit text app opt-ins are normalized and saved in an already encrypted log", async () => {
  const app = await launch({ config: { encryption_enabled: true } });
  app.element("s-text-capture").checked = true;
  app.element("s-text-apps").value = "Notepad\nnotepad\n Code \n";
  await app.click("save-settings");
  const config = app.getConfig();
  assert.equal(config.text_capture_enabled, true);
  assert.deepEqual(config.text_capture_apps.map(name => name.toLowerCase()), ["notepad", "code"]);
  assert.equal(app.latest("set_config").args.password, null);
  assert.equal(app.element("text-capture-notice").classList.contains("hidden"), false);
  assert.equal(app.element("pill-text").classList.contains("hidden"), true);
  app.element("s-idle").value = "600";
  await app.click("save-settings");
  assert.equal(app.getConfig().text_capture_enabled, true);
  assert.equal(app.getConfig().idle_threshold_seconds, 600);
});

test("unsupported platforms cannot enable text recording but can turn a saved opt-in off", async () => {
  const app = await launch({ status: { text_capture_supported: false } });
  assert.equal(app.element("s-text-capture").disabled, true);
  assert.equal(app.element("s-text-apps").disabled, true);
  assert.match(app.element("text-capture-support").textContent, /Windows only/);
  const existing = await launch({ config: { encryption_enabled: true, text_capture_enabled: true,
    text_capture_apps: ["Notepad"] }, status: { text_capture_supported: false } });
  assert.equal(existing.element("s-text-capture").disabled, false);
  existing.element("s-text-capture").checked = false;
  existing.element("s-enc").checked = false;
  await existing.click("save-settings");
  assert.equal(existing.getConfig().text_capture_enabled, false);
  assert.equal(existing.getConfig().encryption_enabled, false);
});

test("the text recording indicator follows native status while the enabled notice persists", async () => {
  const app = await launch({ config: { encryption_enabled: true, text_capture_enabled: true,
    text_capture_apps: ["Notepad"] } });
  await app.emit("daylog://status", { ...defaultStatus(), encrypted: true, text_capture_enabled: true, text_recording: true });
  assert.equal(app.element("pill-text").classList.contains("hidden"), false);
  await app.emit("daylog://status", { ...defaultStatus(), encrypted: true, text_capture_enabled: true,
    text_recording: false, idle: true });
  assert.equal(app.element("pill-text").classList.contains("hidden"), true);
  assert.equal(app.element("text-capture-notice").classList.contains("hidden"), false);
  assert.match(app.element("text-capture-notice").textContent, /Notepad/);
});

test("recorded text renders as escaped plain text inside a collapsed disclosure", async () => {
  const app = await launch({ rows: [{ start: "2026-11-04T08:00:00Z", seconds: 30, app: "Notepad",
    title: "Notes", typed_text: '<img src=x onerror="alert(1)">\n<script>secret</script>&' }] });
  await app.click("apply-filter");
  const rendered = app.element("log-body").innerHTML;
  assert.match(rendered, /<details class="recorded-text">/);
  assert.doesNotMatch(rendered, /<details[^>]*\bopen\b/);
  assert.match(rendered, /&lt;img src=x onerror=&quot;alert\(1\)&quot;&gt;/);
  assert.match(rendered, /&lt;script&gt;secret&lt;\/script&gt;&amp;/);
  assert.doesNotMatch(rendered, /<img\b|<script\b/);
});

test("deleting recorded text preserves activity and permits turning encryption off", async () => {
  const app = await launch({ config: { encryption_enabled: true, text_capture_enabled: true,
    text_capture_apps: ["Notepad"] }, rows: [{ id: 7, start: "2026-11-04T08:00:00Z", seconds: 30,
      app: "Notepad", title: "Notes", typed_text: "private draft" }] });
  app.element("s-text-capture").checked = false;
  app.element("s-enc").checked = false;
  await app.click("save-settings");
  assert.match(app.element("save-note").textContent, /Delete recorded text/);
  assert.equal(app.getConfig().encryption_enabled, true);
  await app.click("clear-recorded-text");
  assert.equal(app.calls.filter(call => call.command === "clear_recorded_text").length, 1);
  assert.equal(app.getConfig().text_capture_enabled, false);
  assert.doesNotMatch(app.element("log-body").innerHTML, /private draft|recorded-text-row/);
  assert.match(app.element("log-body").innerHTML, /Notes|30s/);
  app.element("s-enc").checked = false;
  await app.click("save-settings");
  assert.equal(app.getConfig().encryption_enabled, false);
  assert.equal(app.getConfig().text_capture_enabled, false);
});

test("an unsaved SMTP server cannot receive the entered session password", async () => {
  const app = await launch();
  app.element("email-host").value = "unsaved.example.com";
  app.element("email-password").value = "session-only-secret";
  await app.click("send-email");
  assert.equal(app.calls.filter(call => call.command === "set_email_password").length, 0);
  assert.equal(app.calls.filter(call => call.command === "send_summary_email").length, 0);
  assert.match(app.element("global-note").textContent, /Save the email settings before sending/);
});

function detailRow(app, typed_text, index = 0, title = "Editor notes") {
  const start = new Date(Date.now() - 60_000 * (index + 2));
  return { id: index, app, title, start: start.toISOString(), end: new Date(start.getTime() + 30_000).toISOString(), seconds: 30, typed_text };
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((success, failure) => { resolve = success; reject = failure; });
  return { promise, resolve, reject };
}

function detailResponse(app, text) {
  const events = [detailRow(app, text)];
  return { app, category: "Work", total_seconds: 30, total_sessions: 1, text_sessions: text ? 1 : 0,
    recorded_characters: Array.from(text || "").length, last_seen: events[0].end, events, has_more: false };
}

test("dashboard app buttons and top app open an exact app detail screen", async () => {
  const app = await launch({ rows: [detailRow("Code", "selected app text"), detailRow("Code Insiders", "other app text", 1)] });
  app.element("range-select").value = "all";
  await app.clickWithin("app-bars", "button[data-open-app]", button => button.dataset.openApp === "Code");
  const request = app.latest("application_details").args;
  assert.equal(request.app, "Code");
  assert.equal(request.textOnly, true);
  assert.equal(request.filter.limit, 50);
  assert.equal(request.filter.offset, 0);
  assert.equal(request.filter.app, undefined);
  assert.equal(request.filter.app_exact, undefined);
  assert.equal(app.element("tab-app-details").classList.contains("active"), true);
  assert.equal(app.element("app-details-title").textContent, "Code");
  assert.equal(app.document.activeElement, app.element("app-details-title"));
  assert.match(app.element("app-detail-text-list").innerHTML, /selected app text/);
  assert.doesNotMatch(app.element("app-detail-text-list").innerHTML, /other app text/);
  await app.click("app-detail-back");
  assert.equal(app.element("tab-dashboard").classList.contains("active"), true);
  assert.equal(app.element("range-select").value, "all");
  assert.equal(app.element("app-detail-text-list").innerHTML, "");
  await app.click("stat-top");
  assert.equal(app.latest("application_details").args.app, "Code");
});

test("log app links inherit date and keyword filters and Back preserves the overview page", async () => {
  const app = await launch();
  app.element("f-app").value = "cod";
  app.element("f-keyword").value = "main";
  app.element("f-from").value = "2026-11-01";
  app.element("f-to").value = "2026-11-04";
  await app.click("apply-filter");
  await app.click("next-page");
  assert.equal(app.latest("query").args.filter.offset, 200);
  await app.clickWithin("log-body", "button[data-open-app]");
  assert.equal(app.latest("application_details").args.app, "Code");
  assert.equal(app.latest("application_details").args.filter.keyword, "main");
  assert.equal(app.latest("application_details").args.filter.app, undefined);
  assert.equal(app.element("app-detail-from").value, "2026-11-01");
  assert.equal(app.element("app-detail-to").value, "2026-11-04");
  assert.match(app.element("app-detail-back").textContent, /Back to log/);
  app.element("app-detail-keyword").value = "local detail search";
  await app.click("app-detail-back");
  const restored = app.latest("query").args.filter;
  assert.equal(restored.offset, 200);
  assert.equal(restored.app, "cod");
  assert.equal(restored.keyword, "main");
  assert.equal(app.element("f-from").value, "2026-11-01");
  assert.equal(app.element("f-to").value, "2026-11-04");
  assert.equal(app.element("tab-log").classList.contains("active"), true);
});

test("recorded text and activity views use separate pagination and reset together on filters", async () => {
  const app = await launch({ rows: Array.from({ length: 125 }, (_, index) => detailRow("Code", index < 75 ? `text-${index}` : null, index)) });
  app.element("range-select").value = "all";
  await app.click("stat-top");
  assert.match(app.element("app-detail-page").textContent, /^1–50 of 75 text sessions$/);
  assert.equal(app.element("app-detail-next").disabled, false);
  await app.click("app-detail-next");
  assert.equal(app.latest("application_details").args.filter.offset, 50);
  assert.match(app.element("app-detail-page").textContent, /^51–75 of 75 text sessions$/);
  assert.equal(app.element("app-detail-next").disabled, true);
  await app.click("app-detail-log-tab");
  assert.equal(app.latest("application_details").args.textOnly, false);
  assert.equal(app.latest("application_details").args.filter.offset, 0);
  assert.equal(app.element("app-detail-log-view").classList.contains("hidden"), false);
  assert.equal(app.element("app-detail-text-view").classList.contains("hidden"), true);
  assert.equal(app.element("app-detail-log-tab").getAttribute("aria-pressed"), "true");
  await app.click("app-detail-next");
  assert.equal(app.latest("application_details").args.filter.offset, 50);
  await app.click("app-detail-text-tab");
  assert.equal(app.latest("application_details").args.filter.offset, 50);
  app.element("app-detail-keyword").value = "text-1";
  await app.element("app-detail-keyword").dispatch("keydown", { key: "Enter" });
  assert.equal(app.latest("application_details").args.filter.keyword, "text-1");
  assert.equal(app.latest("application_details").args.filter.offset, 0);
  await app.click("app-detail-log-tab");
  assert.equal(app.latest("application_details").args.filter.offset, 0);
  await app.click("app-detail-reset");
  const cleared = app.latest("application_details").args.filter;
  assert.equal(cleared.from, undefined);
  assert.equal(cleared.to, undefined);
  assert.equal(cleared.keyword, undefined);
  assert.equal(cleared.offset, 0);
});

test("app details escape app names, window titles and Unicode text without embedding text in activity rows", async () => {
  const name = 'Code <img src=x onerror="app()">';
  const title = '<script>title()</script>&"';
  const text = 'é🙂\n<img src=x onerror="text()">&<script>secret</script>';
  const app = await launch({ rows: [detailRow(name, text, 0, title)] });
  await app.click("apply-filter");
  assert.doesNotMatch(app.element("log-body").innerHTML, /data-open-app="Code <img/);
  await app.clickWithin("log-body", "button[data-open-app]", button => button.dataset.openApp === name);
  assert.equal(app.latest("application_details").args.app, name);
  assert.equal(app.element("app-details-title").textContent, name);
  const rendered = app.element("app-detail-text-list").innerHTML;
  assert.match(rendered, /é🙂\n&lt;img src=x onerror=&quot;text\(\)&quot;&gt;/);
  assert.match(rendered, /&lt;script&gt;title\(\)&lt;\/script&gt;&amp;&quot;/);
  assert.match(rendered, new RegExp(`${Array.from(text).length} characters`));
  assert.doesNotMatch(rendered, /<img\b|<script\b/);
  assert.equal(app.element("app-detail-characters").textContent, String(Array.from(text).length));
  await app.click("app-detail-log-tab");
  assert.equal(app.element("app-detail-text-list").innerHTML, "");
  assert.doesNotMatch(app.element("app-detail-log-body").innerHTML, /text\(\)|secret/);
  assert.match(app.element("app-detail-log-body").innerHTML, /&lt;script&gt;title\(\)&lt;\/script&gt;/);
});

test("empty recorded text explains opt-in and skipped fields while activity history remains available", async () => {
  const app = await launch({ rows: [detailRow("Code", null)] });
  app.element("range-select").value = "all";
  await app.click("stat-top");
  assert.equal(app.element("app-detail-text-empty").classList.contains("hidden"), false);
  assert.match(app.element("app-detail-text-empty").textContent, /after you enabled text recording/);
  assert.match(app.element("app-detail-text-empty").textContent, /Protected or unsupported fields and pasted text/);
  assert.equal(app.element("app-detail-sessions").textContent, "1");
  assert.equal(app.element("app-detail-characters").textContent, "0");
  await app.click("app-detail-log-tab");
  assert.match(app.element("app-detail-log-body").innerHTML, /Editor notes/);
  assert.equal(app.element("app-detail-log-empty").classList.contains("hidden"), true);
});

test("a late app response cannot overwrite another app after Back navigation", async () => {
  const first = deferred(), second = deferred();
  const app = await launch({ analytics: { per_app: [{ app: "Code", seconds: 60 }, { app: "Browser", seconds: 30 }] },
    handlers: { application_details: args => args.app === "Code" ? first.promise : second.promise } });
  app.element("range-select").value = "all";
  const openFirst = app.clickWithin("app-bars", "button[data-open-app]", button => button.dataset.openApp === "Code");
  assert.equal(app.element("app-detail-loading").classList.contains("hidden"), false);
  await app.click("app-detail-back");
  const openSecond = app.clickWithin("app-bars", "button[data-open-app]", button => button.dataset.openApp === "Browser");
  second.resolve(detailResponse("Browser", "new app result"));
  await openSecond;
  first.resolve(detailResponse("Code", "STALE APP PRIVATE TEXT"));
  await openFirst;
  assert.equal(app.element("app-details-title").textContent, "Browser");
  assert.match(app.element("app-detail-text-list").innerHTML, /new app result/);
  assert.doesNotMatch(app.element("app-detail-text-list").innerHTML, /STALE APP PRIVATE TEXT/);
});

test("switching details views cancels an older text response", async () => {
  const text = deferred(), activity = deferred();
  const app = await launch({ handlers: { application_details: args => args.textOnly ? text.promise : activity.promise } });
  const opening = app.click("stat-top");
  const changing = app.click("app-detail-log-tab");
  activity.resolve(detailResponse("Code", null));
  await changing;
  text.resolve(detailResponse("Code", "STALE TEXT VIEW"));
  await opening;
  assert.equal(app.element("app-detail-log-view").classList.contains("hidden"), false);
  assert.match(app.element("app-detail-log-body").innerHTML, /Editor notes/);
  assert.equal(app.element("app-detail-text-list").innerHTML, "");
});

test("locking the log clears rendered text and rejects a pending detail response", async () => {
  const pending = deferred();
  let requests = 0;
  const app = await launch({ config: { encryption_enabled: true }, handlers: {
    application_details: () => ++requests === 1 ? detailResponse("Code", "PRIVATE VISIBLE DRAFT") : pending.promise,
  } });
  await app.click("stat-top");
  assert.match(app.element("app-detail-text-list").innerHTML, /PRIVATE VISIBLE DRAFT/);
  const refreshing = app.click("app-detail-refresh");
  assert.equal(app.element("app-detail-text-list").innerHTML, "");
  await app.emit("daylog://status", { ...defaultStatus(), encrypted: true, locked: true });
  pending.resolve(detailResponse("Code", "STALE LOCKED SECRET"));
  await refreshing;
  assert.equal(app.element("lock-overlay").classList.contains("hidden"), false);
  assert.equal(app.element("app-detail-text-list").innerHTML, "");
  assert.equal(app.element("app-detail-log-body").innerHTML, "");
  assert.match(app.element("app-detail-error").textContent, /Unlock the encrypted log/);
  assert.equal(app.element("app-detail-time").textContent, "—");
  assert.equal(app.element("app-detail-next").disabled, true);
  await app.click("app-detail-refresh");
  assert.equal(requests, 2);
});

test("detail errors clear previous data and invalid date filters never invoke IPC", async () => {
  let requests = 0;
  const app = await launch({ handlers: { application_details: () => {
    if (++requests === 1) return detailResponse("Code", "OLD PRIVATE DRAFT");
    throw new Error("log is locked or storage unavailable");
  } } });
  await app.click("stat-top");
  await app.click("app-detail-refresh");
  assert.equal(app.element("app-detail-text-list").innerHTML, "");
  assert.match(app.element("app-detail-error").textContent, /log is locked or storage unavailable/);
  assert.equal(app.element("app-detail-error").classList.contains("hidden"), false);
  assert.equal(app.element("app-detail-loading").classList.contains("hidden"), true);
  assert.equal(app.element("app-detail-next").disabled, true);
  app.element("app-detail-from").value = "2026-11-05";
  app.element("app-detail-to").value = "2026-11-01";
  await app.click("app-detail-apply");
  assert.equal(requests, 2);
  assert.match(app.element("app-detail-error").textContent, /start date must precede/);
});
