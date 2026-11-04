import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../ui/app.js', import.meta.url), 'utf8');
const html = await readFile(new URL('../ui/index.html', import.meta.url), 'utf8');

function setup() {
  const nodes = new Map();
  const $ = (selector) => {
    if (!nodes.has(selector)) nodes.set(selector, {
      textContent: '', hidden: true, attributes: {}, classes: new Set(),
      classList: { toggle(name, enabled) { if (enabled) nodes.get(selector).classes.add(name); else nodes.get(selector).classes.delete(name); } },
      setAttribute(name, value) { this.attributes[name] = value; },
      addEventListener() {},
      scrollIntoView() { this.scrolled = true; },
    });
    return nodes.get(selector);
  };
  const calls = [];
  const context = vm.createContext({
    $, app: { authorizing: false, authError: null, syncStatus: {} },
    invoke: async (command) => { calls.push(command); throw 'nxapi 服务暂时不可用'; },
    refreshAccounts: async () => {}, renderSyncStatus() {}, loadFacts() {},
  });
  vm.runInContext(source.slice(source.indexOf("  const message = $('#message');"), source.indexOf("  messageReveal.addEventListener")) +
    source.slice(source.indexOf('  async function refreshPermission('), source.indexOf('  async function exportAccount(')) +
    '\nthis.setMessage = setMessage; this.refreshPermission = refreshPermission;', context);
  return { $, context, calls };
}

test('feedback is visible on every page, outside the settings view', () => {
  const message = html.indexOf('id="message"');
  assert.ok(message > html.indexOf('</header>') && message < html.indexOf('<section class="view"'), 'feedback must live between the shared header and page sections');
});

test('authorization failure shows only its cause and restores retry', async () => {
  const { $, context, calls } = setup();
  const button = { disabled: false };
  await context.refreshPermission('test-account', button);
  assert.equal($('#message').hidden, false);
  assert.ok($('#message').classes.has('is-error'));
  assert.equal($('#message').scrolled, true);
  assert.match($('#message-text').textContent, /SplatNet 授权失败/);
  assert.match($('#message-text').textContent, /nxapi 服务暂时不可用/);
  assert.equal($('#message-text').textContent, 'SplatNet 授权失败：nxapi 服务暂时不可用');
  assert.equal($('#message').attributes['aria-live'], 'assertive');
  assert.equal(button.disabled, false);
  assert.equal(context.app.authorizing, false);
  assert.deepEqual(calls, ['refresh_splatnet_tokens']);
});

test('success feedback resets error styling and keeps export reveal available', () => {
  const { $, context } = setup();
  context.setMessage('授权失败', true);
  context.setMessage('导出完成', false, '/tmp/example.gz');
  assert.equal($('#message').classes.has('is-error'), false);
  assert.equal($('#message').attributes['aria-live'], 'polite');
  assert.equal($('#message-reveal').hidden, false);
});

// 首次登录后，账号列表刷新失败不能盖掉原始授权错误。
test('initial authorization failure survives a subsequent account refresh failure', async () => {
  const { $, context } = setup();
  context.authenticateButton = { disabled: false };
  context.loginLink = { value: 'test-input' };
  context.loginTarget = null;
  context.closePastePanel = () => {};
  context.invoke = async (command) => {
    if (command === 'complete_nintendo_login') return 'test-account';
    throw 'nxapi 服务暂时不可用';
  };
  context.refreshAccounts = async () => { throw '读取账号列表失败'; };
  vm.runInContext(source.slice(source.indexOf('  async function authenticateNintendoAccount()'), source.indexOf("  authenticateButton.addEventListener('pointerdown'")) +
    '\nthis.authenticate = authenticateNintendoAccount;', context);
  await context.authenticate();
  assert.match($('#message-text').textContent, /nxapi 服务暂时不可用/);
  assert.doesNotMatch($('#message-text').textContent, /读取账号列表失败/);
  assert.equal(context.authenticateButton.disabled, false);
});
