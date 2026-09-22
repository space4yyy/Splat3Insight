const invoke = (...args) => window.__TAURI__.core.invoke(...args);
const openUrl = (url) => window.__TAURI__.opener.openUrl(url);

const message = document.querySelector('#message');
const accounts = document.querySelector('#accounts');
const loginButton = document.querySelector('#login');
const pastePanel = document.querySelector('#paste-panel');
const loginLink = document.querySelector('#login-link');
const authenticateButton = document.querySelector('#authenticate');
const fProvider = document.querySelector('#f-provider');
const nxapiClientId = document.querySelector('#nxapi-client-id');
const saveFProvider = document.querySelector('#save-f-provider');
const fProviderUrl = document.querySelector('#f-provider-url');

function setMessage(value, error = false) {
  message.textContent = value;
  message.classList.toggle('error', error);
}

function updateFProviderForm() {
  const usesNxapi = fProvider.value === 'nxapi';
  nxapiClientId.disabled = !usesNxapi;
  nxapiClientId.closest('.field').classList.toggle('disabled', !usesNxapi);
  fProviderUrl.textContent = usesNxapi
    ? 'https://nxapi-znca-api.fancy.org.uk/api/znca/f'
    : 'https://api.imink.app/f';
}

async function loadFProviderSettings() {
  const settings = await invoke('get_auth_provider_settings');
  fProvider.value = settings.fGenerationProvider;
  nxapiClientId.value = settings.nxapiClientId || '';
  fProviderUrl.textContent = settings.activeFGenerationUrl;
  updateFProviderForm();
}

fProvider.addEventListener('change', updateFProviderForm);
saveFProvider.addEventListener('click', async () => {
  saveFProvider.disabled = true;
  try {
    await invoke('save_auth_provider_settings', {
      fGenerationProvider: fProvider.value,
      nxapiClientId: nxapiClientId.value,
    });
    setMessage('f 参数服务配置已保存。');
  } catch (error) {
    setMessage(String(error), true);
  } finally {
    saveFProvider.disabled = false;
  }
});

function renderAccountList(items) {
  accounts.innerHTML = items.length
    ? items.map((account) => `
      <div class="account row">
        <div>
          <strong>${escapeHtml(account.display_name)}</strong>
          <br /><small>${escapeHtml(account.language || '—')} · ${escapeHtml(account.account_id)}</small>
        </div>
        <div class="account-actions">
          ${account.active ? '<span class="badge">当前</span>' : `<button class="secondary" data-account="${escapeHtml(account.account_id)}">使用</button>`}
          ${account.splatnet_ready
            ? '<span class="badge badge-secondary">SplatNet</span>'
            : `<button class="secondary" data-refresh-account="${escapeHtml(account.account_id)}">获取 SplatNet 权限</button>`}
        </div>
      </div>`).join('')
    : '<p class="status">还没有登录账号。</p>';

  accounts.querySelectorAll('[data-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      await invoke('set_active_account', { accountId: button.dataset.account });
      await refreshAccounts();
    });
  });

  accounts.querySelectorAll('[data-refresh-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      button.disabled = true;
      setMessage('正在使用已保存的 Nintendo 登录信息获取 SplatNet 权限…');
      try {
        await invoke('refresh_splatnet_tokens', { accountId: button.dataset.refreshAccount });
        await refreshAccounts();
        setMessage('SplatNet 权限获取成功。');
      } catch (error) {
        setMessage(String(error), true);
        button.disabled = false;
      }
    });
  });
}

async function refreshAccounts() {
  const result = await invoke('list_accounts');
  renderAccountList(result);
  setMessage(result.length ? '账号信息已保存到系统凭据库。' : '请先登录一个 Nintendo Account。');
}

loginButton.addEventListener('click', async () => {
  loginButton.disabled = true;
  setMessage('正在打开 Nintendo 登录页面…');
  try {
    const url = await invoke('begin_nintendo_login');
    await openUrl(url);
    pastePanel.classList.add('visible');
    loginLink.focus();
    setMessage('请在浏览器登录，然后复制“选择此人”按钮的链接。');
  } catch (error) {
    setMessage(String(error), true);
  } finally {
    loginButton.disabled = false;
  }
});

authenticateButton.addEventListener('click', async () => {
  const callbackUrl = loginLink.value.trim();
  if (!callbackUrl) {
    setMessage('请先粘贴 Nintendo“选择此人”按钮的链接。', true);
    return;
  }
  authenticateButton.disabled = true;
  setMessage('正在认证 Nintendo Account 并生成 SplatNet 凭据…');
  try {
    await invoke('complete_nintendo_login', { callbackUrl });
    pastePanel.classList.remove('visible');
    loginLink.value = '';
    await refreshAccounts();
    setMessage('Nintendo Account 登录成功。SplatNet 权限可单独获取。');
  } catch (error) {
    setMessage(String(error), true);
  } finally {
    authenticateButton.disabled = false;
  }
});

function escapeHtml(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}

Promise.all([refreshAccounts(), loadFProviderSettings()])
  .catch((error) => setMessage(String(error), true));
