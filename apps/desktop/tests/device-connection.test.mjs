import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';

const source = readFileSync(new URL('../src/deviceConnection.ts', import.meta.url), 'utf8');
const script = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const flush = () => new Promise((resolve) => setImmediate(resolve));
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function harness({ getStatus, listenerReady, poll } = {}) {
  const timers = new Set();
  const exports = {};
  runInNewContext(script, {
    exports,
    setInterval: (callback, milliseconds) => {
      assert.equal(milliseconds, 2000);
      timers.add(callback);
      return callback;
    },
    clearInterval: (timer) => timers.delete(timer),
  });
  const state = { connected: true, queries: 0, disconnects: 0, errors: [], unlistens: 0 };
  let listener;
  const stop = exports.monitorDeviceConnection('AA:BB', {
    poll,
    getStatus: () => {
      state.queries += 1;
      return getStatus ? getStatus() : Promise.resolve(state.connected ? 'connected' : 'disconnected');
    },
    listen: (callback) => {
      listener = callback;
      const unlisten = () => { state.unlistens += 1; };
      return listenerReady ? listenerReady.then(() => unlisten) : Promise.resolve(unlisten);
    },
    onDisconnected: () => { state.disconnects += 1; },
    onError: (error) => { state.errors.push(error); },
  });
  return { state, stop, timers, event: (mac = 'AA:BB') => listener({ mac }), tick: () => [...timers].forEach((callback) => callback()) };
}

test('transport disconnect event updates once and stops polling', async () => {
  const monitor = harness();
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  monitor.state.connected = false;
  monitor.event('aa:bb');
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  assert.equal(monitor.timers.size, 0);
  monitor.event();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
  assert.equal(monitor.state.unlistens, 1);
});

test('polling detects disconnects when the transport event is missed', async () => {
  const monitor = harness();
  await flush();
  monitor.state.connected = false;
  monitor.tick();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
});

test('initial check catches a disconnect before the listener was registered', async () => {
  const ready = deferred();
  const monitor = harness({ listenerReady: ready.promise });
  monitor.state.connected = false;
  ready.resolve();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
});

test('late events from another device or a previous session do not disconnect a live watch', async () => {
  const monitor = harness();
  await flush();
  monitor.event('CC:DD');
  await flush();
  assert.equal(monitor.state.queries, 1);
  monitor.event();
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  assert.equal(monitor.state.queries, 2);
  monitor.stop();
});

test('disposing a monitor ignores pending responses and unregisters a late listener', async () => {
  const ready = deferred();
  const status = deferred();
  const monitor = harness({ listenerReady: ready.promise, getStatus: () => status.promise });
  monitor.tick();
  monitor.stop();
  ready.resolve();
  status.resolve('disconnected');
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  assert.equal(monitor.state.unlistens, 1);
  assert.equal(monitor.timers.size, 0);
});

test('a failed status query keeps polling without inventing a disconnect', async () => {
  let fail = true;
  const monitor = harness({ getStatus: () => fail ? Promise.reject(new Error('IPC failed')) : Promise.resolve('disconnected') });
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  assert.equal(monitor.state.errors.length, 1);
  fail = false;
  monitor.tick();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
});

test('a disconnect during an in-flight check immediately requests another check', async () => {
  const initial = deferred();
  let first = true;
  const monitor = harness({ getStatus: () => {
    if (first) { first = false; return initial.promise; }
    return Promise.resolve('disconnected');
  } });
  await flush();
  monitor.event();
  initial.resolve('connected');
  await flush();
  assert.equal(monitor.state.queries, 2);
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
});

test('a pending authentication stays connecting until an actual disconnect', async () => {
  let status = 'connecting';
  const monitor = harness({ getStatus: () => Promise.resolve(status) });
  await flush();
  monitor.tick();
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  status = 'disconnected';
  monitor.event();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  assert.equal(monitor.timers.size, 0);
  monitor.stop();
});

test('polling catches a missed disconnect event during authentication', async () => {
  let status = 'connecting';
  const monitor = harness({ getStatus: () => Promise.resolve(status) });
  await flush();
  status = 'disconnected';
  monitor.tick();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
});

test('authentication success does not trigger a disconnect', async () => {
  let status = 'connecting';
  const monitor = harness({ getStatus: () => Promise.resolve(status) });
  await flush();
  status = 'connected';
  monitor.tick();
  monitor.event();
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  monitor.stop();
});

test('an attempt waits for disconnect events without checking before session registration', async () => {
  let status = 'disconnected';
  const monitor = harness({ poll: false, getStatus: () => Promise.resolve(status) });
  await flush();
  assert.equal(monitor.state.queries, 0);
  assert.equal(monitor.state.disconnects, 0);
  assert.equal(monitor.timers.size, 0);
  status = 'connecting';
  monitor.event();
  await flush();
  assert.equal(monitor.state.disconnects, 0);
  status = 'disconnected';
  monitor.event();
  await flush();
  assert.equal(monitor.state.disconnects, 1);
  monitor.stop();
});
