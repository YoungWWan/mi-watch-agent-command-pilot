import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';

const bundledApp = JSON.parse(readFileSync(new URL('../src-tauri/resources/watch-app.json', import.meta.url), 'utf8'));
function compile(path, dependencies) {
  const source = readFileSync(new URL(path, import.meta.url), 'utf8');
  const script = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true },
  }).outputText;
  const exports = {};
  runInNewContext(script, { exports, ...dependencies });
  return exports;
}
const watch = compile('../src/watchApp.ts', { require: () => bundledApp });
const commandModel = compile('../src/commandModel.ts', { require: () => ({}) });
const appItem = (version = bundledApp.versionCode) => ({ package_name: bundledApp.package, version_code: version });
const otherApp = { package_name: 'com.baidu.BaiduMap', app_name: '百度地图', version_code: 1 };
const flush = () => new Promise((resolve) => setImmediate(resolve));
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('only the command assistant is selected from the watch app list', () => {
  const installed = appItem();
  assert.equal(watch.findWatchApp([otherApp, installed]), installed);
  assert.equal(watch.findWatchApp([otherApp]), null);
});

test('upgrade uses the firmware version code and never downgrades a newer install', () => {
  assert.equal(watch.needsWatchAppUpgrade(appItem(bundledApp.versionCode - 1)), true);
  for (const app of [null, appItem(), appItem(bundledApp.versionCode + 1), { package_name: bundledApp.package }]) {
    assert.equal(watch.needsWatchAppUpgrade(app), false);
  }
});

test('older and unknown versions are not labelled with the bundled version name', () => {
  assert.equal(watch.watchAppVersion(appItem()), `v${bundledApp.versionName}`);
  assert.equal(watch.watchAppVersion(appItem(0)), '版本号 0');
  assert.equal(watch.watchAppVersion({ package_name: bundledApp.package }), '版本未知');
});

// Exercise the component's async handlers with a lightweight hook host and
// mocked Tauri transport, so no test connects to or changes a real watch.
function appHarness({ queries = [], install, server = { status: 'running', watch_paired: true } } = {}) {
  const slots = [];
  const calls = [];
  const listeners = new Map();
  let cursor = 0;
  let effects = [];
  const react = {
    useState: (initial) => {
      const index = cursor++;
      if (!slots[index]) slots[index] = { value: typeof initial === 'function' ? initial() : initial };
      return [slots[index].value, (value) => { slots[index].value = typeof value === 'function' ? value(slots[index].value) : value; }];
    },
    useRef: (initial) => {
      const index = cursor++;
      if (!slots[index]) slots[index] = { current: initial };
      return slots[index];
    },
    useEffect: (effect, deps) => {
      const index = cursor++;
      const previous = slots[index];
      if (!previous || deps.some((value, i) => !Object.is(value, previous.deps[i]))) {
        previous?.cleanup?.();
        slots[index] = { deps };
        effects.push(() => { slots[index].cleanup = effect(); });
      }
    },
  };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    switch (command) {
      case 'account_xiaomi_get_status': return { logged_in: true, device_count: 1 };
      case 'account_xiaomi_get_devices': return [{ name: 'Redmi Watch 5', mac: 'AA:BB', has_authkey: true, is_verified: true }];
      case 'service_get_info': return server;
      case 'command_list_all':
      case 'integration_get_statuses': return [];
      case 'connect_and_auth':
      case 'disconnect_device': return { success: true };
      case 'query_device_apps': {
        const result = queries.shift();
        return typeof result === 'function' ? result() : result ?? [];
      }
      case 'install_bundled_rpk': return install ? install() : { success: true };
      default: throw new Error(`Unexpected invoke: ${command}`);
    }
  };
  const jsx = (type, props) => ({ type, props });
  const App = compile('../src/App.tsx', {
    require: (name) => {
      switch (name) {
        case 'react': return react;
        case 'react/jsx-runtime': return { jsx, jsxs: jsx };
        case '@tauri-apps/api/core': return { invoke };
        case '@tauri-apps/api/event': return { listen: async (name, callback) => { listeners.set(name, callback); return () => listeners.delete(name); } };
        case './deviceConnection': return { monitorDeviceConnection: () => () => {} };
        case './watchApp': return watch;
        case './commandModel': return commandModel;
        case './desktopUpdateModel': return { updateIsBusy: (phase) => ['checking', 'downloading', 'verifying', 'installing', 'restarting'].includes(phase) };
        case './DesktopUpdates': return {
          useDesktopUpdates: () => ({ state: { version: '1.1.0', enabled: false, phase: 'idle', available: null }, check: () => {}, install: () => {} }),
          DesktopUpdateNotice: 'desktop-update-notice', DesktopUpdatePanel: 'desktop-update-panel',
        };
        case './CommandCenter': return { CommandCenter: 'command-center' };
        case './IntegrationManagement': return { IntegrationManagement: 'integration-management' };
        case './AgentTabs': return { AgentTabs: 'agent-tabs' };
        case './WatchIllustration': return { WatchIllustration: 'watch-illustration' };
        case 'lucide-react': return new Proxy({}, { get: (_, key) => String(key) });
        default: throw new Error(`Unexpected import: ${name}`);
      }
    },
    localStorage: { getItem: () => null, setItem: () => {} },
    window: { matchMedia: () => ({ matches: true }) },
    document: { documentElement: { setAttribute: () => {} } },
    setInterval: () => 1,
    clearInterval: () => {},
    alert: () => {},
    console: { error: () => {}, warn: () => {} },
  }).default;
  function render() {
    cursor = 0;
    effects = [];
    const tree = App();
    effects.forEach((effect) => effect());
    return tree;
  }
  function nodes(tree) {
    if (tree === null || tree === undefined || typeof tree === 'boolean') return [];
    if (Array.isArray(tree)) return tree.flatMap(nodes);
    if (typeof tree !== 'object') return [tree];
    return [tree, ...nodes(tree.props?.children)];
  }
  const content = (tree) => nodes(tree).filter((node) => typeof node === 'string').join(' ');
  function button(text) {
    const tree = render();
    const found = nodes(tree).find((node) => node.type === 'button' && (content(node).includes(text) || node.props['aria-label'] === text));
    assert.ok(found, `Missing button: ${text}`);
    return found.props;
  }
  async function ready() {
    render();
    await flush();
    render();
  }
  async function connect() {
    await button('连接设备').onClick();
    await flush();
    render();
  }
  function pairingPanel() {
    button('指令').onClick();
    const center = nodes(render()).find(node => node.type === 'command-center');
    assert.ok(center, 'Missing command center');
    return center.props.pairingPanel;
  }
  return { calls, queries, ready, connect, render, button, pairingPanel, panelText: () => content(pairingPanel()), text: () => content(render()), progress: (payload) => listeners.get('install-progress')({ payload }) };
}

test('saved pairing alone is never presented as a verified watch connection', async () => {
  const app = appHarness();
  await app.ready();
  assert.match(app.panelText(), /配对已保存 · 等待手表连接/);
  assert.doesNotMatch(app.panelText(), /已配对|手表已连接/);
  const connected = appHarness({ server: { status:'running', watch_paired:true, watch_connection_status:'online' } });
  await connected.ready();
  assert.match(connected.panelText(), /手表已连接/);
});

test('pairing errors automatically expand recovery guidance while offline watches retain pairing', async () => {
  const failed = appHarness({ server: { status:'running', watch_paired:true, watch_connection_status:'auth_failed' } });
  await failed.ready();
  assert.equal(failed.pairingPanel().props.open, true);
  assert.match(failed.panelText(), /配对异常.*确认手表电脑地址.*新配对码/);
  const offline = appHarness({ server: { status:'running', watch_paired:true, watch_connection_status:'offline' } });
  await offline.ready();
  assert.match(offline.panelText(), /配对已保存 · 手表离线/);
  assert.equal(offline.pairingPanel().props.open, false);
});

test('connecting automatically checks versions and shows only the assistant with an upgrade reminder', async () => {
  const app = appHarness({ queries: [[otherApp, appItem(bundledApp.versionCode - 1)]] });
  await app.ready();
  await app.connect();
  assert.equal(app.calls.filter((call) => call.command === 'query_device_apps').length, 1);
  assert.match(app.text(), /版本较旧，请升级/);
  assert.equal(app.button('升级到新版本').disabled, false);
  assert.doesNotMatch(app.text(), /百度地图|com\.baidu|已安装应用/);
});

test('a pending automatic check disables installation and a missing app is shown explicitly', async () => {
  const pending = deferred();
  const app = appHarness({ queries: [() => pending.promise] });
  await app.ready();
  await app.connect();
  assert.match(app.text(), /正在检查安装状态与版本/);
  assert.equal(app.button('安装到设备').disabled, true);
  pending.resolve([otherApp]);
  await flush();
  assert.match(app.text(), /尚未安装/);
  assert.equal(app.button('安装到设备').disabled, false);
});

test('failed version checks show an error and can be retried', async () => {
  const app = appHarness({ queries: [() => Promise.reject('timeout'), [appItem()]] });
  await app.ready();
  await app.connect();
  assert.match(app.text(), /无法检查指令助手/);
  assert.doesNotMatch(app.text(), /尚未安装/);
  app.button('检查指令助手版本').onClick();
  await flush();
  assert.match(app.text(), /当前版本/);
  assert.doesNotMatch(app.text(), /无法检查指令助手/);
});

test('a stale query after disconnect cannot replace a reconnected watch state', async () => {
  const pending = deferred();
  const app = appHarness({ queries: [() => pending.promise, [appItem()]] });
  await app.ready();
  await app.connect();
  await app.button('断开连接').onClick();
  await app.connect();
  pending.resolve([appItem(bundledApp.versionCode - 1)]);
  await flush();
  assert.match(app.text(), /当前版本/);
  assert.doesNotMatch(app.text(), /版本较旧/);
});

test('installation remains busy through confirmation, then refreshes the installed version', async () => {
  const pending = deferred();
  const app = appHarness({ queries: [[appItem(bundledApp.versionCode - 1)], [appItem()]], install: () => pending.promise });
  await app.ready();
  await app.connect();
  const installing = app.button('升级到新版本').onClick();
  app.progress({ mac: 'AA:BB', stage: 'UNINSTALLING', percent: 0 });
  assert.match(app.text(), /正在卸载旧版指令助手/);
  app.progress({ mac: 'OTHER', stage: 'COMPLETE', percent: 100 });
  assert.match(app.text(), /正在卸载旧版指令助手/);
  app.progress({ mac: 'AA:BB', stage: 'COMPLETE', percent: 100 });
  assert.equal(app.button('安装中…').disabled, true);
  pending.resolve({ success: true });
  await installing;
  assert.match(app.text(), /当前版本/);
  assert.equal(app.button('重新安装').disabled, false);
});

test('a failure after removal refreshes the actual missing state', async () => {
  const app = appHarness({ queries: [[appItem()], []], install: () => Promise.reject('transfer failed') });
  await app.ready();
  await app.connect();
  await app.button('重新安装').onClick();
  assert.match(app.text(), /尚未安装/);
  assert.match(app.text(), /安装异常/);
  assert.equal(app.button('安装到设备').disabled, false);
});
