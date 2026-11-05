const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');

// Exercise the IPC boundary and actual application handlers without a browser
// dependency or a second frontend toolchain.
function fixture() {
  const elements = new Map();
  const html = readFileSync(path.join(__dirname, '../ui/index.html'), 'utf8');
  for (const match of html.matchAll(/id="([^"]+)"/g)) {
    const classes = new Set();
    elements.set(match[1], {
      value: '', textContent: '', innerHTML: '', checked: false, disabled: false,
      style: {}, listeners: {}, className: '',
      classList: {
        add: (s) => classes.add(s), remove: (s) => classes.delete(s),
        contains: (s) => classes.has(s),
        toggle: (s, on) => { if (on ?? !classes.has(s)) classes.add(s); else classes.delete(s); },
      },
      addEventListener(name, handler) { this.listeners[name] = handler; },
    });
  }
  const status = { paused: false, encrypted: false, locked: false, excluded: false,
    idle: false, idle_available: true, capture_available: true, error: null, current_app: 'Code' };
  const config = { log_path: '/tmp/daylog-test.log', encryption_enabled: false,
    exclusion_enabled: false, blocklist: [], start_on_login: false,
    auto_purge_days: null, poll_interval_ms: 1500, idle_threshold_seconds: 300, show_window_on_launch: true };
  const calls = [];
  const responses = {
    get_status: () => ({ ...status }), get_config: () => ({ ...config }),
    analytics: () => ({ total_seconds: 30, total_events: 1, per_app: [], per_day: [] }),
    query: () => [], save_settings: () => ({ ...status }), export: () => '/tmp/export.csv',
  };
  const context = vm.createContext({
    window: { __TAURI__: {
      core: { invoke: async (command, args) => { calls.push({ command, args }); return responses[command](args); } },
      event: { listen: async () => () => {} },
      dialog: { confirm: async () => true, save: async () => '/tmp/export.csv' },
    }, addEventListener() {} },
    document: { getElementById: (id) => { assert.ok(elements.has(id), `Missing DOM id: ${id}`); return elements.get(id); },
      querySelectorAll: () => [], addEventListener() {}, hidden: false },
    console, setTimeout: () => {}, setInterval: () => {}, Date,
  });
  vm.runInContext(readFileSync(path.join(__dirname, '../ui/app.js'), 'utf8'), context);
  return { context, elements, calls, status, config, responses,
    run: (code) => vm.runInContext(code, context) };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test('all DOM references exist and boot loads dashboard/settings', async () => {
  const f = fixture(); await settle();
  assert.equal(f.elements.get('stat-total').textContent, '30s');
  assert.equal(f.elements.get('s-idle').value, 300);
});

test('status distinguishes exclusions, idle, errors and lock recovery', async () => {
  const f = fixture(); await settle();
  f.context.st = { ...f.status, excluded: true };
  f.run('renderStatus(st)'); assert.equal(f.elements.get('pill-state').textContent, 'excluded');
  f.context.st = { ...f.status, idle: true };
  f.run('renderStatus(st)'); assert.equal(f.elements.get('pill-state').textContent, 'idle');
  f.context.st = { ...f.status, locked: true };
  f.run('renderStatus(st)'); assert.equal(f.elements.get('lock-overlay').classList.contains('hidden'), false);
  f.context.st = { ...f.status, error: 'Disk full' };
  f.run('renderStatus(st)'); assert.equal(f.elements.get('lock-overlay').classList.contains('hidden'), true);
  assert.equal(f.elements.get('error-banner').textContent, 'Disk full');
});

test('saving encrypted settings does not require password again', async () => {
  const f = fixture(); f.config.encryption_enabled = true; await settle();
  // The minimal DOM uses string values as browser inputs do.
  for (const id of ['s-purge', 's-idle', 's-poll', 's-enc-pw']) f.elements.get(id).value = '';
  f.elements.get('s-enc').checked = true;
  await f.run('saveSettings()');
  const saved = f.calls.find((c) => c.command === 'save_settings');
  assert.ok(saved); assert.equal(saved.args.password, null);
  assert.equal(f.elements.get('save-settings').disabled, false);
});

test('failed settings save restores the button and displays the error', async () => {
  const f = fixture(); await settle();
  for (const id of ['s-purge', 's-idle', 's-poll', 's-enc-pw']) f.elements.get(id).value = '';
  f.responses.save_settings = () => { throw new Error('Read only'); };
  await f.run('saveSettings()');
  assert.match(f.elements.get('save-note').textContent, /Read only/);
  assert.equal(f.elements.get('save-settings').disabled, false);
});

test('log empty state follows returned rows and escapes window titles', async () => {
  const f = fixture(); await settle(); await f.run('loadLog()');
  assert.equal(f.elements.get('log-empty').classList.contains('hidden'), false);
  f.responses.query = () => [{ start: '2026-01-01T00:00:00Z', seconds: 10, app: 'Code', title: '<script>"' }];
  await f.run('loadLog()');
  assert.equal(f.elements.get('log-empty').classList.contains('hidden'), true);
  assert.match(f.elements.get('log-body').innerHTML, /&lt;script&gt;&quot;/);
});

test('export uses the selected destination and current filters', async () => {
  const f = fixture(); await settle(); f.elements.get('f-keyword').value = 'Document';
  await f.run('doExport("csv")');
  const exported = f.calls.find((c) => c.command === 'export');
  assert.equal(exported.args.dest, '/tmp/export.csv');
  assert.equal(exported.args.filter.keyword, 'Document');
});
