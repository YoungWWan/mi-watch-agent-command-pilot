import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';
function compile(path, require) {
  const source = readFileSync(new URL(path, import.meta.url), 'utf8');
  const script = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020, jsx: ts.JsxEmit.ReactJSX } }).outputText;
  const exports = {};
  runInNewContext(script, { exports, require });
  return exports;
}
const model = compile('../src/commandModel.ts', () => ({}));
function harness({ status = 'running', port = 8000, loaded = true, paired = true, watchStatus = 'online', commands = [], failure, refreshFailure } = {}) {
  const slots = [], calls = [], effects = [];
  let cursor = 0, refreshes = 0, dirty = false;
  const serverInfo = () => loaded ? { status, port, lan_ip: '192.168.1.20', watch_paired: paired, watch_connection_status: watchStatus } : null;
  const react = {
    useState(initial) {
      const i = cursor++;
      if (!slots[i]) slots[i] = { value: typeof initial === 'function' ? initial() : initial };
      return [slots[i].value, value => {
        const next = typeof value === 'function' ? value(slots[i].value) : value;
        if (next !== slots[i].value) dirty = true;
        slots[i].value = next;
      }];
    },
    useRef(initial) {
      const i = cursor++;
      if (!slots[i]) slots[i] = { current: initial };
      return slots[i];
    },
    useEffect(effect, deps) {
      const i = cursor++;
      if (!slots[i] || deps.some((value, j) => value !== slots[i].deps[j])) {
        slots[i] = { deps };
        effects.push(effect);
      }
    },
  };
  const jsx = (type, props) => ({ type, props });
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (failure) throw new Error(failure);
    if (command === 'command_create') {
      const item = { ...args.payload, id: `new-${calls.length}`, status: 'pending', created_at: Date.now()/1000, expires_at: Date.now()/1000+args.payload.timeout_seconds };
      commands.unshift(item); return item;
    }
    if (command === 'service_save_port') { port = args.port; return serverInfo(); }
    return {};
  };
  const { CommandCenter } = compile('../src/CommandCenter.tsx', name => {
    if (name === 'react') return react;
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx };
    if (name === '@tauri-apps/api/core') return { invoke };
    if (name === './commandModel') return model;
    if (name === 'lucide-react') return new Proxy({}, { get: (_, key) => key });
    throw new Error(name);
  });
  function render() {
    let tree;
    do {
      cursor = 0; dirty = false;
      tree = CommandCenter({ server: serverInfo(), commands, pairingPanel: null, refresh: async () => { refreshes++; if (refreshFailure) throw new Error(refreshFailure); } });
      effects.splice(0).forEach(effect => effect());
    } while (dirty);
    return tree;
  }
  function nodes(tree) {
    if (Array.isArray(tree)) return tree.flatMap(nodes);
    if (!tree || typeof tree === 'boolean') return [];
    if (typeof tree !== 'object') return [tree];
    return [tree, ...nodes(tree.props?.children)];
  }
  const text = tree => nodes(tree).filter(node => typeof node === 'string').join(' ');
  function find(type, label) {
    const node = nodes(render()).find(node => node.type === type && (node.props['aria-label'] === label || text(node).includes(label)));
    assert.ok(node, `Missing ${type}: ${label}`); return node.props;
  }
  render();
  return { calls, commands, render, find, setFailure(value) { failure = value; }, setServer(changes) { if ('port' in changes) port = changes.port; if ('loaded' in changes) loaded = changes.loaded; }, text: () => text(render()), get refreshes() { return refreshes; } };
}
const flush = () => new Promise(resolve => setImmediate(resolve));
const submitPort = app => app.find('form', '服务端口设置').onSubmit({ preventDefault() {} });
const editPort = (app, value) => app.find('input', '服务端口').onChange({ target: { value } });

test('service port validation rejects invalid and out-of-range inputs without dispatch', () => {
  for (const value of ['', '0', '-1', '65536', '3.5', '1e3', 'abc', 'Infinity']) {
    const app = harness();
    editPort(app, value); submitPort(app);
    assert.equal(app.calls.length, 0);
    assert.match(app.text(), /服务端口必须是 1–65535 的整数/);
  }
  for (const [input, port] of [['1', 1], ['65535', 65535], [' 8123 ', 8123]]) {
    assert.equal(model.parseServicePort(input), port);
  }
});

test('saving the running port dispatches once, updates the form and explains watch changes', async () => {
  const app = harness({ port: 8123 });
  assert.equal(app.find('input', '服务端口').value, '8123');
  assert.equal(app.find('button', '保存并应用').disabled, true);
  editPort(app, '9000');
  assert.equal(app.find('button', '保存并应用').disabled, false);
  submitPort(app); submitPort(app);
  assert.equal(app.find('input', '服务端口').disabled, true);
  await flush();
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].command, 'service_save_port');
  assert.equal(app.calls[0].args.port, 9000);
  assert.equal(app.refreshes, 1);
  assert.equal(app.find('input', '服务端口').value, '9000');
  assert.match(app.text(), /服务已切换至 9000/);
  assert.match(app.text(), /192\.168\.1\.20:9000/);
  assert.match(app.text(), /未完成指令已过期/);
});

test('stopped service saves the port without claiming that it started', async () => {
  const app = harness({ status: 'stopped' });
  editPort(app, '8123'); submitPort(app); await flush();
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].command, 'service_save_port');
  assert.match(app.text(), /端口 8123 已保存，下次启动服务时使用/);
  assert.doesNotMatch(app.text(), /服务已切换|服务已启动/);
});

test('port errors preserve the draft for retry and never claim success', async () => {
  const app = harness({ failure: '端口被占用' });
  editPort(app, '8123'); submitPort(app); await flush();
  assert.match(app.text(), /端口被占用/);
  assert.doesNotMatch(app.text(), /端口已保存|服务已切换/);
  assert.equal(app.find('input', '服务端口').value, '8123');
  assert.equal(app.find('button', '保存并应用').disabled, false);
  app.setFailure(null); submitPort(app); await flush();
  assert.equal(app.calls.length, 2);
  assert.match(app.text(), /服务已切换至 8123/);
});

test('service polling preserves a draft and loads a changed saved port', () => {
  const app = harness({ loaded: false });
  assert.equal(app.find('input', '服务端口').disabled, true);
  submitPort(app);
  assert.equal(app.calls.length, 0);
  app.setServer({ loaded: true, port: 8123 });
  assert.equal(app.find('input', '服务端口').value, '8123');
  editPort(app, '9000');
  assert.equal(app.find('input', '服务端口').value, '9000');
  assert.equal(app.find('input', '服务端口').value, '9000');
  app.setServer({ port: 8888 });
  assert.equal(app.find('input', '服务端口').value, '8888');
  submitPort(app);
  assert.equal(app.calls.length, 0);
});

test('stopped service exposes start and disables dispatch and restart', async () => {
  const app = harness({ status: 'stopped' });
  const button = app.find('button','测试手表连接');
  assert.equal(button.disabled,true);
  button.onClick();
  assert.equal(app.calls.length,0);
  assert.equal(app.find('button','重启').disabled,true);
  app.find('button','启动服务').onClick(); await flush();
  assert.equal(app.calls[0].command,'service_start');
  assert.equal(app.refreshes,1);
});
test('unpaired watch cannot receive a connection test', () => {
  const app = harness({ paired: false });
  const button = app.find('button','测试手表连接');
  assert.equal(button.disabled,true);
  button.onClick();
  assert.equal(app.calls.length,0);
  assert.match(app.text(),/请先完成上方的手表配对/);
});
test('one click dispatches a fixed test and blocks repeats while awaiting wrist confirmation', async () => {
  const app = harness();
  const button = app.find('button','测试手表连接');
  button.onClick(); button.onClick(); await flush();
  assert.equal(app.calls.length,1);
  assert.equal(app.calls[0].command,'command_create');
  assert.equal(app.calls[0].args.payload.title,'手表连接测试');
  assert.equal(app.calls[0].args.payload.actions.length,1);
  assert.equal(app.calls[0].args.payload.actions[0].id,'confirm_connection');
  assert.equal(app.calls[0].args.payload.timeout_seconds,60);
  assert.equal(app.find('button','等待手表确认').disabled,true);
  app.find('button','等待手表确认').onClick(); await flush();
  assert.equal(app.calls.length,1);
  assert.doesNotMatch(app.text(),/连接正常/);
});
test('live wrist reply completes the test and permits another test', async () => {
  const app = harness();
  app.find('button','测试手表连接').onClick(); await flush();
  app.commands[0] = { ...app.commands[0], status:'replied', reply:{ action_id:'confirm_connection', device_id:'redmi-watch-5' } };
  assert.match(app.text(),/连接正常，已收到手表确认/);
  assert.equal(app.find('button','测试手表连接').disabled,false);
  app.find('button','测试手表连接').onClick(); await flush();
  assert.equal(app.calls.length,2);
  assert.match(app.text(),/等待手表确认/);
  assert.doesNotMatch(app.text(),/连接正常/);
});
test('desktop replies and unidentified replies cannot report wrist connectivity', () => {
  for (const device_id of ['desktop-simulator', undefined]) {
    const item = { ...model.createConnectionTest(), id:'test', status:'replied', created_at:1, expires_at:Date.now()/1000+60, reply:{ action_id:'confirm_connection', device_id } };
    const app = harness({ commands:[item] });
    assert.match(app.text(),/未收到有效的手表确认/);
    assert.doesNotMatch(app.text(),/连接正常/);
    assert.equal(app.find('button','测试手表连接').disabled,false);
  }
});
test('a failed retest replaces the previous success message', async () => {
  const item = { ...model.createConnectionTest(), id:'test', status:'replied', created_at:1, expires_at:Date.now()/1000+60, reply:{ action_id:'confirm_connection', device_id:'redmi-watch-5' } };
  const app = harness({ commands:[item] });
  assert.match(app.text(),/连接正常/);
  app.setFailure('连接已断开');
  app.find('button','测试手表连接').onClick(); await flush();
  assert.match(app.text(),/测试发送失败/);
  assert.doesNotMatch(app.text(),/连接正常/);
  assert.equal(app.find('button','测试手表连接').disabled,false);
});
test('dispatch and refresh errors are visible and release the busy state', async () => {
  const app = harness({ failure:'端口不可用' });
  app.find('button','测试手表连接').onClick(); await flush();
  assert.match(app.text(),/端口不可用/);
  assert.equal(app.find('button','测试手表连接').disabled,false);
  app.find('button','测试手表连接').onClick(); await flush();
  assert.equal(app.calls.length,2);
  const refreshError = harness({ status:'stopped', refreshFailure:'刷新失败' });
  refreshError.find('button','启动服务').onClick(); await flush();
  assert.match(refreshError.text(),/刷新失败/);
  assert.equal(refreshError.find('button','启动服务').disabled,false);
});
test('expired tests permit retry even before the server updates their status', async () => {
  for (const status of ['pending', 'expired']) {
    const item = { ...model.createConnectionTest(), id:'old', status, created_at:1, expires_at:Date.now()/1000-1 };
    const app = harness({ commands:[item] });
    assert.match(app.text(),/未收到手表回复/);
    assert.equal(app.find('button','测试手表连接').disabled,false);
    app.find('button','测试手表连接').onClick(); await flush();
    assert.equal(app.calls.length,1);
    assert.match(app.text(),/等待手表确认/);
  }
});
test('returning to the page recovers an outstanding connection test from history', () => {
  const item = { ...model.createConnectionTest(), id:'existing', status:'pending', created_at:1, expires_at:Date.now()/1000+60 };
  const app = harness({ commands:[item] });
  assert.equal(app.find('button','等待手表确认').disabled,true);
  app.find('button','等待手表确认').onClick();
  assert.equal(app.calls.length,0);
});

test('authentication failures replace old test success and prevent futile dispatch', () => {
  const item = { ...model.createConnectionTest(), id:'old', status:'replied', created_at:1, expires_at:2, reply:{ action_id:'confirm_connection', device_id:'redmi-watch-5' } };
  const app = harness({ watchStatus:'auth_failed', commands:[item] });
  assert.match(app.text(), /配对验证失败.*重新配对/);
  assert.doesNotMatch(app.text(), /连接正常/);
  const button = app.find('button', '测试手表连接');
  assert.equal(button.disabled, true);
  button.onClick();
  assert.equal(app.calls.length, 0);
});

test('old wrist confirmations never claim that an offline or unverified watch is online', () => {
  const item = { ...model.createConnectionTest(), id:'old', status:'replied', created_at:1, expires_at:2, reply:{ action_id:'confirm_connection', device_id:'redmi-watch-5' } };
  for (const watchStatus of ['offline', 'waiting', null]) {
    const app = harness({ watchStatus, commands:[item] });
    assert.match(app.text(), /上次测试已收到手表确认/);
    assert.doesNotMatch(app.text(), /连接正常/);
    assert.equal(app.find('button', '测试手表连接').disabled, false);
  }
  const stopped = harness({ status:'stopped', commands:[item] });
  assert.match(stopped.text(), /请先启动指令服务/);
  assert.doesNotMatch(stopped.text(), /连接正常/);
});
