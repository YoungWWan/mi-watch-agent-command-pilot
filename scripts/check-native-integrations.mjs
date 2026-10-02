import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const binary = process.argv[2];
if (!binary) throw new Error('Usage: node scripts/check-native-integrations.mjs <executable>');

function run(args, input) {
  const result = spawnSync(resolve(binary), args, {
    input,
    encoding: 'utf8',
    timeout: 15_000,
    windowsHide: true,
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim().split(/\r?\n/).map(line => JSON.parse(line));
}

// Test the actual release executable, including Windows GUI-subsystem stdio.
// Invalid tool arguments exercise JSON input without dispatching a watch command.
const requests = [
  { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'integration-check', version: '1' } } },
  { jsonrpc: '2.0', method: 'notifications/initialized' },
  { jsonrpc: '2.0', id: 2, method: 'tools/list' },
  { jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'ask_watch_question', arguments: { question: '中文输入检查 💡', options: [] } } },
];
const responses = run(['--mcp'], requests.map(request => JSON.stringify(request)).join('\n') + '\n');
assert.deepEqual(responses.map(response => response.id), [1, 2, 3]);
assert.equal(responses[0].result.serverInfo.name, 'agent-command-pilot');
assert.ok(responses[1].result.tools.some(tool => tool.name === 'ask_watch_question'));
assert.equal(responses[2].result.isError, true);
assert.deepEqual(run(['--hook', 'integration-check'], JSON.stringify({ message: '中文钩子输入 💡' })), [{}]);
console.log('Native MCP and hook stdio checks passed.');
