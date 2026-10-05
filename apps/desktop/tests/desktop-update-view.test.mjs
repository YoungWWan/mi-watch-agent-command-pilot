import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import * as jsx from 'react/jsx-runtime';
import * as icons from 'lucide-react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
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
    case 'react-markdown': return Markdown;
    case 'remark-gfm': return remarkGfm;
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

test('raw HTML in release notes stays escaped and pending commands block update', () => {
  const html = render({ phase: 'available', available: { version: '1.1.1', notes: '<script>alert(1)</script>' } }, true);
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.doesNotMatch(html, /<script>/);
  assert.match(html, /请先完成手表安装或待回复指令/);
  assert.match(html, /disabled=""[^>]*>.*?立即更新/s);
});

test('release notes render Markdown headings, lists, inline code and installation tables', () => {
  const html = render({ phase: 'available', available: { version: '1.1.1', notes: [
    '## v1.1.1 更新说明',
    '',
    '- 提供 **Windows x64**、Mac Apple Silicon（M 系列）和 Mac Intel 安装包。',
    '- 已安装 `1.1.0` 的用户可检查 `1.1.1`。',
    '',
    '| 系统 | 首次安装或手动升级 |',
    '| --- | --- |',
    '| Mac Apple Silicon（M 系列） | `mi-watch-agent-command-pilot_1.1.1_darwin-aarch64.dmg` |',
    '| Mac Intel | `mi-watch-agent-command-pilot_1.1.1_darwin-x86_64.dmg` |',
    '| Windows x64 | `mi-watch-agent-command-pilot_1.1.1_windows-x86_64.exe` |',
  ].join('\n') } });
  assert.match(html, /<h2>v1\.1\.1 更新说明<\/h2>/);
  assert.match(html, /<ul>\s*<li>提供 <strong>Windows x64<\/strong>/);
  assert.match(html, /<code>1\.1\.0<\/code>/);
  assert.match(html, /class="desktop-update-table"><table><thead><tr><th>系统<\/th><th>首次安装或手动升级<\/th>/);
  assert.match(html, /<td>Mac Apple Silicon（M 系列）<\/td><td><code>mi-watch-agent-command-pilot_1\.1\.1_darwin-aarch64\.dmg<\/code><\/td>/);
  assert.equal((html.match(/<tbody>[\s\S]*?<\/tbody>/)?.[0].match(/<tr>/g) || []).length, 3);
  assert.doesNotMatch(html, /\| --- |`1\.1\.0`/);
});

test('release notes support code blocks, quotes, task lists and safe links', () => {
  const html = render({ phase: 'available', available: { version: '1.1.1', notes: [
    '> 更新完成后自动重启。',
    '',
    '```sh',
    'echo "<ready>"',
    '```',
    '',
    '- [x] ~~旧版问题~~已修复',
    '- [ ] 手动安装',
    '',
    '[下载](https://example.com/download) [危险链接](javascript:alert%281%29)',
  ].join('\n') } });
  assert.match(html, /<blockquote>\s*<p>更新完成后自动重启。<\/p>/);
  assert.match(html, /<pre><code class="language-sh">echo &quot;&lt;ready&gt;&quot;\n<\/code><\/pre>/);
  assert.match(html, /class="contains-task-list"/);
  assert.match(html, /type="checkbox" disabled="" checked=""/);
  assert.match(html, /<del>旧版问题<\/del>/);
  assert.match(html, /<a href="https:\/\/example\.com\/download" target="_blank" rel="noopener noreferrer">下载<\/a>/);
  assert.match(html, /<span>危险链接<\/span>/);
  assert.doesNotMatch(html, /href="javascript:/);
});

test('plain text release notes remain readable and empty notes show the fallback', () => {
  const html = render({ phase: 'available', available: { version: '1.1.1', notes: '修复连接问题。\n\n改进更新体验。' } });
  assert.match(html, /<p>修复连接问题。<\/p>\s*<p>改进更新体验。<\/p>/);
  for (const notes of ['', ' \n\t ', null]) {
    assert.match(render({ phase: 'available', available: { version: '1.1.1', notes } }), /<p>此版本包含改进与问题修复。<\/p>/);
  }
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
