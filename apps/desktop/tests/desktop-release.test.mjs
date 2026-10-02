import { mkdtempSync, mkdirSync, cpSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { projectRoot, platforms, repositoryName, validateVersion, releaseVersion, setVersion, prepareRelease, collectArtifacts, buildManifest } from '../../../scripts/desktop-release.mjs';

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'desktop-release-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const path of ['package.json', 'apps/desktop/package.json', 'apps/desktop/src-tauri/Cargo.toml', 'apps/desktop/src-tauri/Cargo.lock', 'apps/desktop/src-tauri/tauri.conf.json']) {
    mkdirSync(join(root, path, '..'), { recursive: true });
    cpSync(join(projectRoot, path), join(root, path));
  }
  return root;
}
const signature = readFileSync(new URL('../src-tauri/tests/fixtures/desktop-update.txt.sig', import.meta.url), 'utf8').trim();

test('repository validation supports GitHub URLs and rejects credentials and path traversal', () => {
  assert.equal(repositoryName('https://github.com/young/mi-watch-agent-command-pilot.git'), 'young/mi-watch-agent-command-pilot');
  for (const input of ['', '../repo', 'https://evil.test/o/r', 'https://user:pass@github.com/o/r', 'o/r/releases', 'o/r?x=y']) assert.throws(() => repositoryName(input));
  for (const version of ['1.1.0-beta.1', '01.1.1', 'v1.1.0', '1.1']) assert.throws(() => validateVersion(version));
});

test('version bump updates all desktop manifests and rejects a mismatched release tag', (t) => {
  const root = fixture(t);
  assert.equal(setVersion('1.1.1', root), '1.1.1');
  assert.equal(releaseVersion(root, 'v1.1.1'), '1.1.1');
  assert.throws(() => releaseVersion(root, 'v1.1.0'), /不一致/);
});

test('release versions read and update Cargo files checked out with Windows CRLF', (t) => {
  const root = fixture(t);
  const paths = ['apps/desktop/src-tauri/Cargo.toml', 'apps/desktop/src-tauri/Cargo.lock'];
  for (const relative of paths) {
    const path = join(root, relative);
    writeFileSync(path, readFileSync(path, 'utf8').replace(/\r?\n/g, '\r\n'));
  }
  const current = JSON.parse(readFileSync(join(root, 'package.json'))).version;
  assert.equal(releaseVersion(root, `v${current}`), current);
  assert.equal(setVersion('1.1.1', root), '1.1.1');
  assert.equal(releaseVersion(root, 'v1.1.1'), '1.1.1');
  for (const relative of paths) {
    assert.ok(readFileSync(join(root, relative), 'utf8').includes('\r\n'));
  }
});

test('release preparation writes the actual repository and requires a real signing public key', (t) => {
  const root = fixture(t);
  const version = releaseVersion(root);
  const result = prepareRelease('owner/mi-watch-agent-command-pilot', `v${version}`, root);
  assert.equal(result.endpoint, 'https://github.com/owner/mi-watch-agent-command-pilot/releases/latest/download/latest.json');
  const path = join(root, 'apps/desktop/src-tauri/tauri.conf.json');
  const config = JSON.parse(readFileSync(path));
  assert.equal(config.bundle.createUpdaterArtifacts, true);
  assert.equal(config.plugins.updater.requireSignedVersion, true);
  config.plugins.updater.pubkey = 'placeholder';
  writeFileSync(path, JSON.stringify(config));
  assert.throws(() => prepareRelease('owner/repo', `v${version}`, root), /公钥无效/);
});

test('complete manifest uses signature contents and every artifact points to its version tag', (t) => {
  const root = fixture(t);
  setVersion('1.1.1', root);
  const output = join(root, 'release-assets');
  mkdirSync(output);
  for (const platform of platforms) {
    const input = join(root, platform);
    mkdirSync(input);
    const extension = platform.startsWith('darwin') ? '.app.tar.gz' : '.exe';
    writeFileSync(join(input, `指令助手${extension}`), 'fixture');
    writeFileSync(join(input, `指令助手${extension}.sig`), signature);
    if (platform.startsWith('darwin')) writeFileSync(join(input, '指令助手.dmg'), 'fixture');
    collectArtifacts(input, output, platform, '1.1.1');
  }
  const manifest = buildManifest(output, 'owner/repo', 'v1.1.1', '更新说明', root);
  assert.deepEqual(Object.keys(manifest.platforms), platforms);
  assert.equal(manifest.notes, '更新说明');
  for (const item of Object.values(manifest.platforms)) {
    assert.equal(item.signature, signature);
    assert.match(item.url, /^https:\/\/github.com\/owner\/repo\/releases\/download\/v1.1.1\//);
  }
  assert.throws(() => buildManifest(output, 'owner/repo', 'v1.1.0', '', root), /不一致/);
  const missing = join(output, 'mi-watch-agent-command-pilot_1.1.1_windows-x86_64.exe.sig');
  rmSync(missing);
  assert.throws(() => buildManifest(output, 'owner/repo', 'v1.1.1', '', root), /缺少/);
});

test('manifest rejects a signature bound to another version', (t) => {
  const root = fixture(t);
  setVersion('1.1.2', root);
  const output = join(root, 'assets');
  mkdirSync(output);
  const path = join(output, 'mi-watch-agent-command-pilot_1.1.2_darwin-aarch64.app.tar.gz');
  writeFileSync(path, 'fixture');
  writeFileSync(`${path}.sig`, signature);
  assert.throws(() => buildManifest(output, 'owner/repo', 'v1.1.2', '', root), /签名版本不匹配/);
});
