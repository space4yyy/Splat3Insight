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
      removeAttribute(name) { delete this.attributes[name]; },
      addEventListener() {},
      focus() { this.focused = true; },
      scrollIntoView() { this.scrolled = true; },
    });
    return nodes.get(selector);
  };
  const calls = [];
  const context = vm.createContext({
    $, app: { authorizing: false, authError: null, authFailure: null, syncStatus: {}, accounts: [], activeAccountId: null },
    invoke: async (command) => { calls.push(command); throw 'nxapi 服务暂时不可用'; },
    refreshAccounts: async () => {}, renderSyncStatus() {}, loadFacts() {},
  });
  vm.runInContext(source.slice(source.indexOf("  const message = $('#message');"), source.indexOf("  messageReveal.addEventListener")) +
    source.slice(source.indexOf('  async function refreshPermission('), source.indexOf('  async function exportAccount(')) +
    '\nthis.setMessage = setMessage; this.refreshPermission = refreshPermission; this.explainAuthError = explainAuthError;', context);
  return { $, context, calls };
}

function setupAuthenticate(invoke) {
  const env = setup();
  const { context } = env;
  context.authenticateButton = { disabled: false };
  context.loginLink = context.$('#login-link');
  context.loginLink.value = 'test-input';
  context.pasteHint = context.$('#paste-hint');
  context.loginTarget = null;
  context.closePastePanel = () => { context.pasteClosed = true; };
  context.clearLoginStatus = () => {};
  context.showLoginError = (title, error, target) => { context.loginError = { title, error: String(error), target }; };
  context.invoke = invoke;
  vm.runInContext(source.slice(source.indexOf('  async function authenticateNintendoAccount()'), source.indexOf("  authenticateButton.addEventListener('pointerdown'")) +
    '\nthis.authenticate = authenticateNintendoAccount;', context);
  return env;
}

test('feedback is visible on every page, outside the settings view', () => {
  for (const id of ['id="message"', 'id="account-alert"']) {
    const index = html.indexOf(id);
    assert.ok(index > html.indexOf('</header>') && index < html.indexOf('<section class="view"'), `${id} must live between the shared header and page sections`);
  }
});

test('login failures are shown next to the login controls', () => {
  const status = html.indexOf('id="login-status"');
  assert.ok(status > html.indexOf('id="paste-panel"') && status < html.indexOf('id="account-list"'));
});

test('authorization failure for another account explains the cause and restores retry', async () => {
  const { $, context, calls } = setup();
  const button = { disabled: false };
  await context.refreshPermission('test-account', button);
  assert.equal($('#message').hidden, false);
  assert.ok($('#message').classes.has('is-error'));
  assert.equal($('#message').scrolled, true);
  assert.equal($('#message-close').hidden, false);
  const text = $('#message-text').textContent;
  assert.match(text, /SplatNet 授权失败：第三方认证服务（nxapi）暂时不可用/);
  assert.match(text, /nxapi 服务暂时不可用/);
  assert.equal($('#message').attributes['aria-live'], 'assertive');
  assert.equal(button.disabled, false);
  assert.equal(context.app.authorizing, false);
  assert.deepEqual({ ...context.app.authFailure }, { accountId: 'test-account', error: 'nxapi 服务暂时不可用' });
  assert.deepEqual(calls, ['refresh_splatnet_tokens']);
});

// 当前账号授权失败时由顶部的账号提醒说明，不再叠一条同样的提示条。
test('authorization failure for the active account is left to the account alert', async () => {
  const { $, context } = setup();
  context.app.activeAccountId = 'test-account';
  await context.refreshPermission('test-account');
  assert.equal($('#message').hidden, true);
  assert.equal(context.app.authFailure.accountId, 'test-account');
  assert.equal(context.app.authError, 'nxapi 服务暂时不可用');
});

test('retrying authorization clears the previous failure', async () => {
  const { context } = setup();
  context.app.authFailure = { accountId: 'test-account', error: 'old' };
  context.invoke = async () => ({ added_battle_count: 0, added_coop_count: 0 });
  await context.refreshPermission('test-account');
  assert.equal(context.app.authFailure, null);
});

test('success feedback resets error styling and keeps export reveal available', () => {
  const { $, context } = setup();
  context.setMessage('授权失败', true);
  context.setMessage('导出完成', false, '/tmp/example.gz');
  assert.equal($('#message').classes.has('is-error'), false);
  assert.equal($('#message').attributes['aria-live'], 'polite');
  assert.equal($('#message-reveal').hidden, false);
  assert.equal($('#message-close').hidden, true);
});

test('raw authentication errors are summarized with a next step', () => {
  const { context } = setup();
  const cases = [
    ['Nintendo 登录状态已失效，请重新登录。HTTP 400', /登录状态已失效/, true],
    ['当前登录的是另一个 Nintendo 账号“B”，未保存更改。', /不是同一个/, true],
    ['请粘贴 Nintendo“选择此人”按钮的链接', /链接不正确/, true],
    ['relative URL without a base', /链接不正确/, true],
    ['Nintendo login state did not match', /已过期/, true],
    ['Nintendo session失败（HTTP 400）：error=invalid_grant', /已过期或已被使用/, true],
    ['该 Nintendo Account 尚未使用 Splatoon 3 在线服务', /Splatoon 3/, false],
    ['Nintendo Coral失败（HTTP 200）：status=9403', /会员/, false],
    ['f 参数服务失败（HTTP 429）：error=rate_limited', /频繁/, false],
    ['SplatNet token 自动刷新失败，请检查网络或稍后重试：nxapi 授权失败（HTTP 502）', /nxapi/, false],
    ['连接 Nintendo 账号服务失败，请检查网络后重试', /网络连接失败/, false],
    ['无法连接代理端口，请检查代理地址和端口', /代理/, false],
  ];
  for (const [raw, summary, relogin] of cases) {
    const info = context.explainAuthError(raw);
    assert.match(info.summary, summary, raw);
    assert.ok(info.hint, raw);
    assert.equal(Boolean(info.relogin), relogin, raw);
  }
});

// 首次登录后，账号列表刷新失败不能盖掉原始授权错误。
test('initial authorization failure survives a subsequent account refresh failure', async () => {
  const { $, context } = setupAuthenticate(async (command) => {
    if (command === 'complete_nintendo_login') return 'test-account';
    throw 'nxapi 服务暂时不可用';
  });
  context.refreshAccounts = async () => { throw '读取账号列表失败'; };
  await context.authenticate();
  assert.match($('#message-text').textContent, /nxapi 服务暂时不可用/);
  assert.doesNotMatch($('#message-text').textContent, /读取账号列表失败/);
  assert.equal(context.authenticateButton.disabled, false);
  assert.equal(context.loginError, undefined);
});

test('a rejected login link is reported next to the login controls', async () => {
  const { context } = setupAuthenticate(async () => { throw 'Nintendo login state did not match'; });
  await context.authenticate();
  assert.equal(context.pasteClosed, true);
  assert.equal(context.loginError.title, '添加账号失败');
  assert.equal(context.loginError.error, 'Nintendo login state did not match');
  assert.equal(context.authenticateButton.disabled, false);
});

test('an empty login link is flagged in place without starting authentication', async () => {
  const calls = [];
  const { context } = setupAuthenticate(async (command) => { calls.push(command); });
  context.loginLink.value = '  ';
  await context.authenticate();
  assert.deepEqual(calls, []);
  assert.equal(context.loginLink.attributes['aria-invalid'], 'true');
  assert.equal(context.loginLink.focused, true);
  assert.match(context.pasteHint.textContent, /请先粘贴/);
});
