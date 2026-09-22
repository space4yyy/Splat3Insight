const invoke = (...args) => window.__TAURI__.core.invoke(...args);
const openUrl = (url) => window.__TAURI__.opener.openUrl(url);

const message = document.querySelector('#message');
const accounts = document.querySelector('#accounts');
const loginButton = document.querySelector('#login');
const pastePanel = document.querySelector('#paste-panel');
const loginLink = document.querySelector('#login-link');
const authenticateButton = document.querySelector('#authenticate');

function setMessage(value, error = false) {
  message.textContent = value;
  message.classList.toggle('error', error);
}

function renderAccountList(items) {
  accounts.innerHTML = items.length
    ? items.map((account) => `
      <div class="account row">
        <div>
          <strong>${escapeHtml(account.display_name)}</strong>
          <br /><small>${escapeHtml(account.language || '—')} · ${escapeHtml(account.account_id)}</small>
          <br /><small>历史：${account.history_battle_count} 对战 · ${account.history_coop_count} 打工</small>
        </div>
        <div class="account-actions">
          ${account.active ? '<span class="badge">当前</span>' : `<button class="secondary" data-account="${escapeHtml(account.account_id)}">使用</button>`}
          ${account.splatnet_ready
            ? `<span class="badge badge-secondary">SplatNet</span><button class="secondary" data-sync-account="${escapeHtml(account.account_id)}">同步历史</button>`
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

  accounts.querySelectorAll('[data-sync-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      button.disabled = true;
      setMessage('正在从 SplatNet 获取最近 50 条对战和 50 条打工记录…');
      try {
        const summary = await invoke('sync_history', { accountId: button.dataset.syncAccount });
        await refreshAccounts();
        setMessage(`历史同步成功：新增 ${summary.added_battle_count} 条对战、${summary.added_coop_count} 条打工；归档共 ${summary.battle_count} 条对战、${summary.coop_count} 条打工。`);
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

refreshAccounts().catch((error) => setMessage(String(error), true));
