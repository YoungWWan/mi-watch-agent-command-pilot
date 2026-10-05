import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';

const binary = process.argv[2];
if (!binary) throw new Error('Usage: node scripts/check-kimi-bridge.mjs <executable>');
const home = await mkdtemp(join(tmpdir(), 'kimi-hook-check-'));
const kimiHome = join(home, '.kimi-code');
const token = randomBytes(32).toString('base64url');
const received = [];
const server = createServer(async (request, response) => {
  response.setHeader('content-type', 'application/json');
  if (['/api/v1/meta', '/api/v1/sessions/session_test/snapshot'].includes(request.url)) {
    assert.equal(request.headers.authorization, `Bearer ${token}`);
    const data = request.url.endsWith('/meta') ? { server_id: 'server_test' }
      : { in_flight_turn: { turn_id: 7 }, session: { id: 'session_test' } };
    response.end(JSON.stringify({ code: 0, data }));
  } else if (request.url === '/api/v1/integrations/kimi/hook') {
    let body = '';
    for await (const chunk of request) body += chunk;
    received.push(JSON.parse(body));
    response.end(JSON.stringify({ status: 'accepted' }));
  } else {
    response.writeHead(404).end(JSON.stringify({ code: 40401 }));
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
try {
  const port = server.address().port;
  await mkdir(join(kimiHome, 'server/instances'), { recursive: true });
  await mkdir(join(home, '.agent-command-pilot'), { recursive: true });
  await writeFile(join(kimiHome, 'server.token'), token, { mode: 0o600 });
  await writeFile(join(kimiHome, 'server/instances/test.json'), JSON.stringify({ host: '127.0.0.1', port, server_id: 'different_registry_id' }));
  await writeFile(join(home, '.agent-command-pilot/settings.json'), JSON.stringify({ server_port: port }));
  async function hook(mode, payload) {
    const child = spawn(resolve(binary), ['--hook', mode, '--app', 'Kimi Code'], {
      env: { ...process.env, HOME: home, USERPROFILE: home, KIMI_CODE_HOME: kimiHome },
      windowsHide: true,
    });
    let output = '', errors = '';
    child.stdout.on('data', chunk => { output += chunk; });
    child.stderr.on('data', chunk => { errors += chunk; });
    child.stdin.end(JSON.stringify(payload));
    const timer = setTimeout(() => child.kill(), 15000);
    const code = await new Promise((resolve, reject) => { child.on('error', reject); child.on('close', resolve); });
    clearTimeout(timer);
    assert.equal(code, 0, errors);
    assert.equal(errors, '');
    assert.deepEqual(JSON.parse(output), {});
  }
  await hook('permission', { hook_event_name: 'PermissionRequest', session_id: 'session_test', id: 'approval_test', tool_call_id: 'call_test', tool_name: 'Bash', agent_id: 'main', turn_id: 7, cwd: '/workspace/中文' });
  await hook('stop', { hook_event_name: 'Stop', session_id: 'session_test', stop_hook_active: false, cwd: '/workspace/中文' });
  await hook('permission', { hook_event_name: 'PermissionRequest', session_id: 'session_test' });
  await hook('stop', { hook_event_name: 'Stop', session_id: 'session_test', stop_hook_active: true });
  assert.deepEqual(received, [
    { kind: 'approval', session_id: 'session_test', approval_id: 'approval_test', tool_call_id: 'call_test', tool_name: 'Bash', agent_id: 'main', turn_id: 7, cwd: '/workspace/中文' },
    { kind: 'completion', session_id: 'session_test', server_id: 'server_test', turn_id: 7, cwd: '/workspace/中文' },
  ]);
  console.log('Kimi release hook authentication and bridge checks passed.');
} finally {
  await new Promise(resolve => server.close(resolve));
  await rm(home, { recursive: true, force: true });
}
