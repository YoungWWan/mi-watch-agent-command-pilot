import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import * as jsx from 'react/jsx-runtime';
import * as icons from 'lucide-react';
import ts from 'typescript';

const model = {};
runInNewContext(ts.transpileModule(readFileSync(new URL('../src/desktopUpdateModel.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, { exports: model, Date, Set });
const view = {};
runInNewContext(ts.transpileModule(readFileSync(new URL('../src/DesktopUpdates.tsx', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true },
}).outputText, { exports: view, require: (name) => {
  switch (name) {
    case 'react': return React;
    case 'react/jsx-runtime': return jsx;
    case 'lucide-react': return icons;
    case './desktopUpdateModel': return model;
    case '../package.json': return { version: '1.1.0' };
    case '@tauri-apps/api/core': case '@tauri-apps/api/event': return {};
    default: throw new Error(`Unexpected import ${name}`);
  }
} });
const initial = { version: '1.1.0', enabled: true, message: '', phase: 'idle', available: null, error: '', lastChecked: null, downloaded: 0, total: null };
function render(changes, blocked = false, name = 'DesktopUpdatePanel') {
  return renderToStaticMarkup(React.createElement(view[name], { state: { ...initial, ...changes }, blocked, check: () => {}, install: () => {}, onDetails: () => {}, dismissedVersion: null, dismiss: () => {} }));
}

test('a failed check never displays latest version and retains a retry button', () => {
  const html = render({ phase: 'error', error: '无法连接更新服务', lastChecked: 123 });
  assert.doesNotMatch(html, /已是最新版本/);
  assert.match(html, /无法连接更新服务/);
  assert.match(html, /检查更新/);
  assert.doesNotMatch(html, /disabled=""/);
  assert.match(render({ lastChecked: 123 }), /已是最新版本/);
});

test('release notes render as escaped text and pending commands block update', () => {
  const html = render({ phase: 'available', available: { version: '1.1.1', notes: '<script>alert(1)</script>' } }, true);
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.doesNotMatch(html, /<script>/);
  assert.match(html, /请先完成手表安装或待回复指令/);
  assert.match(html, /disabled=""[^>]*>.*?立即更新/s);
});

test('unknown download size shows bytes with an indeterminate progress bar', () => {
  const html = render({ phase: 'downloading', available: { version: '1.1.1', notes: '' }, downloaded: 1048576, total: null });
  assert.match(html, /正在下载 · 1.0 MB/);
  assert.match(html, /<progress[^>]*max="100"><\/progress>/);
  assert.doesNotMatch(html, /NaN|Infinity/);
});

test('announcement and installation states have readable status and disabled update controls', () => {
  const html = render({ phase: 'installing', available: { version: '1.1.1', notes: '' } }, false, 'DesktopUpdateNotice');
  assert.match(html, /桌面应用有新版本 · v1.1.1/);
  assert.match(html, /正在安装更新/);
  assert.match(html, /role="status"/);
  assert.match(html, /disabled=""/);
  assert.doesNotMatch(html, /稍后更新/);
});
