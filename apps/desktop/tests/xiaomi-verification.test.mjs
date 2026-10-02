import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const script = readFileSync(new URL('../src-tauri/src/account/xiaomi_verification_complete.js', import.meta.url), 'utf8');
function completed(url, body) {
  return runInNewContext(script, { location: new URL(url), document: { body: body === null ? null : { innerText: body } } });
}
test('official plain ok page triggers automatic continuation', () => {
  assert.equal(completed('https://account.xiaomi.com/identity/result', 'ok'), true);
  assert.equal(completed('https://account.xiaomi.com/identity/result', '\n OK \n'), true);
  assert.equal(completed('https://sts-hlth.io.mi.com/healthapp/sts', 'ok'), true);
});
test('code entry, loading and failure pages never trigger login', () => {
  for (const body of [null, '', '请输入验证码', '验证码错误', 'ok\n请输入验证码', 'not ok']) {
    assert.equal(completed('https://account.xiaomi.com/identity/verify', body), false);
  }
});
test('untrusted pages cannot signal completion', () => {
  for (const url of ['http://account.xiaomi.com/result', 'https://xiaomi.com.evil.test/result', 'https://evilxiaomi.com/result', 'https://account.xiaomi.com:444/result', 'about:blank']) {
    assert.equal(completed(url, 'ok'), false);
  }
});
