import { readFileSync, writeFileSync, readdirSync, mkdirSync, copyFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const platforms = ['darwin-aarch64', 'darwin-x86_64', 'windows-x86_64'];
const configPath = 'apps/desktop/src-tauri/tauri.conf.json';
const cargoPath = 'apps/desktop/src-tauri/Cargo.toml';
const lockPath = 'apps/desktop/src-tauri/Cargo.lock';
const packages = ['package.json', 'apps/desktop/package.json'];
const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));
const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);

export function repositoryName(value = '') {
  value = value.trim().replace(/^https:\/\/github\.com\//, '').replace(/\.git$/, '').replace(/\/$/, '');
  if (!/^[A-Za-z0-9][A-Za-z0-9-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(value)) {
    throw new Error('请提供 GitHub OWNER/REPO 或完整仓库地址');
  }
  return value;
}

export function validateVersion(version) {
  // Stable releases only; previews must not replace the public stable update feed.
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error('版本必须为稳定版本，例如 1.1.1');
  return version;
}

export function releaseVersion(root = projectRoot, tag) {
  const version = validateVersion(readJson(join(root, configPath)).version);
  const cargo = readFileSync(join(root, cargoPath), 'utf8').match(/^\[package\][\s\S]*?^version = "([^"]+)"/m)?.[1];
  const lock = readFileSync(join(root, lockPath), 'utf8').match(/name = "agent-command-pilot"\nversion = "([^"]+)"/)?.[1];
  if ([cargo, lock, ...packages.map((path) => readJson(join(root, path)).version)].some((other) => other !== version)) {
    throw new Error('桌面版本不一致，请运行 node scripts/desktop-release.mjs version <版本>');
  }
  if (tag && tag !== `v${version}`) throw new Error(`标签 ${tag} 与桌面版本 v${version} 不一致`);
  return version;
}

export function setVersion(version, root = projectRoot) {
  validateVersion(version);
  for (const path of [...packages, configPath]) {
    const json = readJson(join(root, path));
    json.version = version;
    writeJson(join(root, path), json);
  }
  const cargo = join(root, cargoPath);
  writeFileSync(cargo, readFileSync(cargo, 'utf8').replace(/(^\[package\][\s\S]*?^version = ")[^"]+("$)/m, `$1${version}$2`));
  const lock = join(root, lockPath);
  writeFileSync(lock, readFileSync(lock, 'utf8').replace(/(name = "agent-command-pilot"\nversion = ")[^"]+("\n)/, `$1${version}$2`));
  return releaseVersion(root);
}

export function prepareRelease(repository, tag, root = projectRoot) {
  const repo = repositoryName(repository);
  const version = releaseVersion(root, tag);
  const config = readJson(join(root, configPath));
  const key = Buffer.from(config.plugins?.updater?.pubkey || '', 'base64').toString('utf8').trim().split('\n');
  if (!key[0]?.startsWith('untrusted comment: minisign public key:') || Buffer.from(key[1] || '', 'base64').length !== 42) {
    throw new Error('更新公钥无效，不能发布签名更新');
  }
  config.bundle.createUpdaterArtifacts = true;
  config.plugins.updater.endpoints = [`https://github.com/${repo}/releases/latest/download/latest.json`];
  config.plugins.updater.requireSignedVersion = true;
  writeJson(join(root, configPath), config);
  return { repository: repo, version, endpoint: config.plugins.updater.endpoints[0] };
}

function filesUnder(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? filesUnder(path) : [path];
  });
}

export function collectArtifacts(directory, output, platform, version) {
  validateVersion(version);
  if (!platforms.includes(platform)) throw new Error(`不支持的平台：${platform}`);
  const extension = platform.startsWith('darwin-') ? '.app.tar.gz' : '.exe';
  const candidates = filesUnder(directory).filter((path) => path.endsWith(extension) && existsSync(`${path}.sig`));
  if (candidates.length !== 1) throw new Error(`平台 ${platform} 需要且只能有一个签名更新包，实际为 ${candidates.length}`);
  mkdirSync(output, { recursive: true });
  const name = `mi-watch-agent-command-pilot_${version}_${platform}${extension}`;
  copyFileSync(candidates[0], join(output, name));
  copyFileSync(`${candidates[0]}.sig`, join(output, `${name}.sig`));
  if (platform.startsWith('darwin-')) {
    const images = filesUnder(directory).filter((path) => path.endsWith('.dmg'));
    if (images.length !== 1) throw new Error(`平台 ${platform} 缺少唯一的 DMG 安装包`);
    copyFileSync(images[0], join(output, `mi-watch-agent-command-pilot_${version}_${platform}.dmg`));
  }
  return name;
}

export function buildManifest(directory, repository, tag, notes = '', root = projectRoot) {
  const repo = repositoryName(repository);
  const version = releaseVersion(root, tag);
  const manifest = { version, notes, pub_date: new Date().toISOString(), platforms: {} };
  for (const platform of platforms) {
    const extension = platform.startsWith('darwin-') ? '.app.tar.gz' : '.exe';
    const name = `mi-watch-agent-command-pilot_${version}_${platform}${extension}`;
    const path = join(directory, name);
    if (!existsSync(path) || !existsSync(`${path}.sig`)) throw new Error(`缺少 ${platform} 更新包或签名`);
    const signature = readFileSync(`${path}.sig`, 'utf8').trim();
    const decoded = Buffer.from(signature, 'base64').toString('utf8');
    if (!decoded.startsWith('untrusted comment:') || !decoded.includes('\ntrusted comment:')) throw new Error(`签名格式无效：${platform}`);
    // Version binding is signed by Tauri CLI; reject an accidental old artifact.
    if (!decoded.split('\n').some((line) => line.startsWith('trusted comment:') && line.slice('trusted comment:'.length).trim().split(/\s+/).includes(`version:${version}`))) {
      throw new Error(`更新包签名版本不匹配：${platform}`);
    }
    manifest.platforms[platform] = { signature, url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(name)}` };
  }
  writeJson(join(directory, 'latest.json'), manifest);
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, ...args] = process.argv.slice(2);
  const option = (name) => { const index = args.indexOf(`--${name}`); return index < 0 ? undefined : args[index + 1]; };
  try {
    if (command === 'version') console.log(`桌面版本：${setVersion(args[0])}`);
    else if (command === 'prepare') console.log(prepareRelease(option('repository') || process.env.GITHUB_REPOSITORY, option('tag') || process.env.GITHUB_REF_NAME));
    else if (command === 'collect') console.log(collectArtifacts(option('input'), option('output'), option('platform'), releaseVersion()));
    else if (command === 'manifest') {
      const notesPath = option('notes');
      const result = buildManifest(option('input'), option('repository') || process.env.GITHUB_REPOSITORY, option('tag') || process.env.GITHUB_REF_NAME, notesPath ? readFileSync(notesPath, 'utf8') : '');
      console.log(`latest.json 已生成，包含 ${Object.keys(result.platforms).join(', ')}`);
    } else throw new Error('用法：desktop-release.mjs version|prepare|collect|manifest');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
