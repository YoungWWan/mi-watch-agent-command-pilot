import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';

const exports = {};
runInNewContext(ts.transpileModule(readFileSync(new URL('../src/desktopUpdateModel.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, { exports, Date, Set });
const { DesktopUpdateController } = exports;
const newer = { version: '1.1.1', notes: '改进与修复', date: null };
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function harness(overrides = {}) {
  const calls = [];
  let progress;
  const client = {
    info: async () => ({ version: '1.1.0', enabled: true, message: '自动检查' }),
    check: async () => { calls.push('check'); return newer; },
    listen: async (callback) => { calls.push('listen'); progress = callback; return () => calls.push('unlisten'); },
    install: async () => { calls.push('install'); },
    ...overrides,
  };
  return { client, calls, controller: new DesktopUpdateController(client, '1.1.0', () => 123), progress: (event) => progress(event) };
}

test('startup announces a newer version but never installs without a click', async () => {
  const h = harness();
  await h.controller.initialize();
  assert.equal(h.controller.snapshot().available, newer);
  assert.equal(h.controller.snapshot().phase, 'available');
  assert.deepEqual(h.calls, ['check']);
});

test('unconfigured builds do not contact a placeholder update server', async () => {
  const h = harness({ info: async () => ({ version: '1.1.0', enabled: false, message: '未开通' }) });
  await h.controller.initialize();
  await h.controller.check();
  await h.controller.install();
  assert.deepEqual(h.calls, []);
  assert.equal(h.controller.snapshot().enabled, false);
});

test('offline checks remain retryable and never report a successful latest-version check', async () => {
  const h = harness({ check: async () => { throw new Error('connection timeout'); } });
  await h.controller.initialize();
  assert.equal(h.controller.snapshot().phase, 'error');
  assert.equal(h.controller.snapshot().lastChecked, null);
  assert.match(h.controller.snapshot().error, /无法连接/);
  h.client.check = async () => null;
  await h.controller.check();
  assert.equal(h.controller.snapshot().phase, 'idle');
  assert.equal(h.controller.snapshot().error, '');
  assert.equal(h.controller.snapshot().lastChecked, 123);
});

test('a failed recheck preserves the discovered update for retry', async () => {
  const h = harness();
  await h.controller.initialize();
  h.client.check = async () => { throw new Error('DNS failed'); };
  await h.controller.check();
  assert.equal(h.controller.snapshot().available, newer);
  await h.controller.install();
  assert.equal(h.controller.snapshot().phase, 'restarting');
});

test('concurrent checks and install clicks cannot launch duplicate operations', async () => {
  const pending = deferred();
  const h = harness({ check: () => { h.calls.push('check'); return pending.promise; } });
  const initialize = h.controller.initialize();
  await Promise.resolve();
  await Promise.all([h.controller.check(), h.controller.install()]);
  assert.deepEqual(h.calls, ['check']);
  pending.resolve(newer);
  await initialize;
  const installing = deferred();
  h.client.install = () => { h.calls.push('install'); return installing.promise; };
  const operation = h.controller.install();
  await Promise.resolve();
  await Promise.all([h.controller.check(), h.controller.install()]);
  assert.equal(h.calls.filter((call) => call === 'install').length, 1);
  installing.resolve();
  await operation;
  assert.deepEqual(h.calls, ['check', 'listen', 'install', 'unlisten']);
});

test('progress is registered before download and supports absent content length', async () => {
  const h = harness();
  await h.controller.initialize();
  h.client.install = async () => {
    assert.deepEqual(h.calls, ['check', 'listen']);
    h.progress({ phase: 'downloading', downloaded: 2048, total: null });
    assert.equal(h.controller.snapshot().downloaded, 2048);
    assert.equal(h.controller.snapshot().total, null);
    h.progress({ phase: 'verifying', downloaded: 2048, total: 2048 });
    h.progress({ phase: 'installing', downloaded: 2048, total: 2048 });
  };
  await h.controller.install();
  assert.equal(h.controller.snapshot().phase, 'restarting');
  assert.equal(h.calls.at(-1), 'unlisten');
});

test('signature failure never reports installed or restarting and releases its listener', async () => {
  const h = harness({ install: async () => { throw new Error('minisign signature verification failed'); } });
  await h.controller.initialize();
  await h.controller.install();
  assert.equal(h.controller.snapshot().phase, 'error');
  assert.match(h.controller.snapshot().error, /签名校验失败，未安装/);
  assert.equal(h.controller.snapshot().available, newer);
  assert.equal(h.calls.at(-1), 'unlisten');
});

test('unmount during listener setup cancels installation and cleans up the listener', async () => {
  const pending = deferred();
  const h = harness({ listen: () => pending.promise });
  await h.controller.initialize();
  const installing = h.controller.install();
  h.controller.dispose();
  pending.resolve(() => h.calls.push('unlisten'));
  await installing;
  assert.deepEqual(h.calls, ['check', 'unlisten']);
});
