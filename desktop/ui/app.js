const invoke = (...args) => window.__TAURI__.core.invoke(...args);
const openUrl = (url) => window.__TAURI__.opener.openUrl(url);

const message = document.querySelector('#message');
const accounts = document.querySelector('#accounts');
const loginButton = document.querySelector('#login');
const pastePanel = document.querySelector('#paste-panel');
const loginLink = document.querySelector('#login-link');
const cancelLoginButton = document.querySelector('#cancel-login');
const authenticateButton = document.querySelector('#authenticate');
const overlayPreview = document.querySelector('#overlay-preview');
const syncStatusElement = document.querySelector('#sync-status');
const overlayServiceStatusElement = document.querySelector('#overlay-service-status');
const startOverlayServiceButton = document.querySelector('#start-overlay-service');
const stopOverlayServiceButton = document.querySelector('#stop-overlay-service');
const copyOverlayUrlButton = document.querySelector('#copy-overlay-url');
const copyRandomWeaponsUrlButton = document.querySelector('#copy-random-weapons-url');
const copyRandomWeaponsRulesUrlButton = document.querySelector('#copy-random-weapons-rules-url');
const refreshPrivateWeaponsButton = document.querySelector('#refresh-private-weapons');
const randomWeaponsPreviewFrame = document.querySelector('.weapon-source-preview iframe');
const backgroundClosePrompt = document.querySelector('#background-close-prompt');
const cancelCloseButton = document.querySelector('#cancel-close');
const exitFromCloseButton = document.querySelector('#exit-from-close');
const hideToTrayButton = document.querySelector('#hide-to-tray');

const OBS_OVERLAY_URL = 'http://127.0.0.1:17353/overlay';
const OBS_RANDOM_WEAPONS_URL = 'http://127.0.0.1:17353/random-weapons';
const OBS_RANDOM_WEAPONS_RULES_URL = 'http://127.0.0.1:17353/random-weapons-rules';

let currentSyncStatus = null;
let lastCompletedSyncAt = null;
let closePromptAcknowledged = false;

try {
  closePromptAcknowledged = localStorage.getItem('splat3-close-prompt-acknowledged') === 'true';
} catch {}

function acknowledgeClosePrompt() {
  closePromptAcknowledged = true;
  try {
    localStorage.setItem('splat3-close-prompt-acknowledged', 'true');
  } catch {}
}

window.__TAURI__.event.listen('main-close-requested', async () => {
  if (backgroundClosePrompt.open) {
    return;
  }
  if (closePromptAcknowledged) {
    try {
      await invoke('hide_main_window');
    } catch (error) {
      setMessage(`隐藏主窗口失败：${String(error)}`, true);
    }
    return;
  }
  backgroundClosePrompt.showModal();
}).catch((error) => setMessage(`初始化窗口关闭提示失败：${String(error)}`, true));

cancelCloseButton.addEventListener('click', () => backgroundClosePrompt.close());
backgroundClosePrompt.addEventListener('cancel', (event) => {
  event.preventDefault();
  backgroundClosePrompt.close();
});

hideToTrayButton.addEventListener('click', async () => {
  acknowledgeClosePrompt();
  backgroundClosePrompt.close();
  try {
    await invoke('hide_main_window');
  } catch (error) {
    setMessage(`隐藏主窗口失败：${String(error)}`, true);
  }
});

exitFromCloseButton.addEventListener('click', async () => {
  acknowledgeClosePrompt();
  backgroundClosePrompt.close();
  try {
    await invoke('exit_app');
  } catch (error) {
    setMessage(`退出应用失败：${String(error)}`, true);
  }
});

function setMessage(value, error = false) {
  message.textContent = value;
  message.hidden = !value;
  message.classList.toggle('error', error);
}

function renderSyncStatus(status) {
  currentSyncStatus = status;
  if (status?.state === 'idle' && !status.account_id) {
    syncStatusElement.textContent = '';
    syncStatusElement.hidden = true;
    syncStatusElement.classList.remove('error', 'syncing');
    return;
  }
  syncStatusElement.hidden = false;
  if (!status) {
    syncStatusElement.textContent = '同步状态不可用。';
    syncStatusElement.classList.add('error');
    return;
  }

  const busy = ['preparing', 'syncing', 'refreshing'].includes(status.state);
  const prefix = status.initial ? '首次同步' : '后台同步';
  if (busy) {
    const progress = status.progress_total > 0
      ? ` (${status.progress_completed}/${status.progress_total})`
      : '';
    const retry = status.retry_attempt > 0
      ? ` · 重试 ${status.retry_attempt}`
      : '';
    syncStatusElement.textContent = `${prefix}：${status.message}${progress}${retry}`;
  } else if (status.state === 'completed') {
    syncStatusElement.textContent = `最近同步完成：新增 ${status.added_battle_count} 条对战、${status.added_coop_count} 条打工。`;
  } else if (status.state === 'error') {
    syncStatusElement.textContent = `同步失败：${status.error || status.message}`;
  } else {
    syncStatusElement.textContent = status.message;
  }
  syncStatusElement.classList.toggle('syncing', busy);
  syncStatusElement.classList.toggle('error', status.state === 'error');
}

function renderOverlayServiceStatus(status) {
  if (!status) {
    overlayServiceStatusElement.textContent = 'OBS 服务状态不可用。';
    overlayServiceStatusElement.className = 'service-status error';
    return;
  }

  const running = status.state === 'running';
  const starting = status.state === 'starting';
  const stopped = status.state === 'stopped';
  overlayServiceStatusElement.textContent = running
    ? `● ${status.message}`
    : `${status.message}${status.error ? `：${status.error}` : ''}`;
  overlayServiceStatusElement.className = `service-status ${running ? 'running' : starting ? 'starting' : stopped ? 'stopped' : 'error'}`;
  startOverlayServiceButton.hidden = running || starting;
  stopOverlayServiceButton.hidden = !running && !starting;
  startOverlayServiceButton.disabled = starting;
  stopOverlayServiceButton.disabled = starting;
}

function fitOverlayPreview() {
  const viewport = overlayPreview.querySelector('.overlay-preview__viewport');
  const scaleElement = overlayPreview.querySelector('.overlay-preview__scale');
  const card = overlayPreview.querySelector('.overlay-card');
  if (!viewport || !scaleElement || !card) {
    return;
  }

  const naturalWidth = card.offsetWidth;
  const naturalHeight = card.offsetHeight;
  const availableWidth = overlayPreview.clientWidth;
  const scale = naturalWidth > 0 && availableWidth > 0
    ? Math.min(1, availableWidth / naturalWidth)
    : 1;

  scaleElement.style.transform = `scale(${scale})`;
  viewport.style.height = `${Math.ceil(naturalHeight * scale)}px`;
}

function renderOverlay(state) {
  if (!state?.selected) {
    overlayPreview.innerHTML = '<div class="overlay-empty">没有找到可显示的 X 或蛮颓比赛记录。</div>';
    return;
  }

  let scaleElement = overlayPreview.querySelector('.overlay-preview__scale');
  if (!scaleElement) {
    overlayPreview.innerHTML = `
      <div class="overlay-preview__viewport">
        <div class="overlay-preview__scale"></div>
      </div>`;
    scaleElement = overlayPreview.querySelector('.overlay-preview__scale');
  }

  window.Splat3OverlayCard.update(scaleElement, state);
  fitOverlayPreview();
}

window.addEventListener('resize', fitOverlayPreview);

async function refreshOverlay(accountId = null) {
  try {
    await document.fonts.load('16px "DFP GBZY7"');
    const state = await invoke('get_overlay_state', { accountId });
    renderOverlay(state);
  } catch (error) {
    overlayPreview.innerHTML = `<div class="overlay-empty">${escapeHtml(String(error))}</div>`;
  }
}

function renderAccountList(items) {
  accounts.innerHTML = items.length
    ? items.map((account) => `
      <div class="account">
        <div class="account-main">
          <strong class="account-name">${escapeHtml(account.display_name)}</strong>
          <small class="account-history">历史：${account.history_battle_count} 对战 · ${account.history_coop_count} 打工</small>
        </div>
        <div class="account-actions">
          ${account.active ? '<span class="badge">当前</span>' : `<button class="secondary" data-account="${escapeHtml(account.account_id)}">使用</button>`}
          <button class="secondary" data-export-account="${escapeHtml(account.account_id)}">导出</button>
          <button class="danger" data-delete-account="${escapeHtml(account.account_id)}" data-account-name="${escapeHtml(account.display_name)}">删除</button>
          ${account.splatnet_ready === false || (account.active && account.splatnet_ready === null) ? `<button class="secondary" data-refresh-account="${escapeHtml(account.account_id)}">获取权限</button>` : ''}
        </div>
      </div>`).join('')
    : '<p class="status">点击“添加 Nintendo Account”开始。</p>';

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
        setMessage('SplatNet 权限获取成功，正在开始首次历史同步…');
        const summary = await invoke('sync_history', { accountId: button.dataset.refreshAccount });
        await refreshAccounts();
        setMessage(`首次历史同步完成：新增 ${summary.added_battle_count} 条对战、${summary.added_coop_count} 条打工。`);
      } catch (error) {
        setMessage(String(error), true);
        button.disabled = false;
      }
    });
  });

  accounts.querySelectorAll('[data-export-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      button.disabled = true;
      try {
        const path = await invoke('export_history', { accountId: button.dataset.exportAccount });
        setMessage(`历史数据已导出：${path}`);
      } catch (error) {
        setMessage(`导出历史数据失败：${String(error)}`, true);
      } finally {
        button.disabled = false;
      }
    });
  });

  accounts.querySelectorAll('[data-delete-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      const accountName = button.dataset.accountName || '此账号';
      if (button.dataset.deleteConfirmed !== 'true') {
        button.dataset.deleteConfirmed = 'true';
        button.textContent = '确认删除';
        setMessage(`请再次点击“确认删除”以删除${accountName}及其本地历史数据。`);
        window.setTimeout(() => {
          if (button.isConnected && button.dataset.deleteConfirmed === 'true') {
            button.dataset.deleteConfirmed = 'false';
            button.textContent = '删除';
          }
        }, 5000);
        return;
      }

      button.dataset.deleteConfirmed = 'false';
      button.disabled = true;
      setMessage(`正在删除${accountName}及其本地历史数据…`);
      try {
        await invoke('remove_account', { accountId: button.dataset.deleteAccount });
        await refreshAccounts();
        setMessage(`已删除${accountName}及其本地历史数据。`);
      } catch (error) {
        setMessage(`删除${accountName}失败：${String(error)}`, true);
      } finally {
        button.disabled = false;
      }
    });
  });

}

async function refreshAccounts() {
  const result = await invoke('list_accounts');
  renderAccountList(result);
  await refreshOverlay(result.find((account) => account.active)?.account_id || null);
  setMessage(result.length ? '账号信息已保存到系统凭据库。' : '');
}

async function refreshSyncStatus() {
  try {
    const status = await invoke('get_sync_status');
    const completedAt = status?.completed_at_ms || null;
    renderSyncStatus(status);
    if (status?.state === 'completed' && completedAt && completedAt !== lastCompletedSyncAt) {
      lastCompletedSyncAt = completedAt;
      await refreshAccounts();
    }
  } catch (error) {
    renderSyncStatus({ state: 'error', message: String(error), error: String(error) });
  }
}

async function refreshOverlayServiceStatus() {
  try {
    const status = await invoke('get_overlay_service_status');
    renderOverlayServiceStatus(status);
  } catch (error) {
    renderOverlayServiceStatus({ error: String(error) });
  }
}

startOverlayServiceButton.addEventListener('click', async () => {
  startOverlayServiceButton.disabled = true;
  try {
    const status = await invoke('start_overlay_service');
    renderOverlayServiceStatus(status);
    setMessage('正在启动 OBS 服务…');
  } catch (error) {
    setMessage(String(error), true);
    await refreshOverlayServiceStatus();
  }
});

stopOverlayServiceButton.addEventListener('click', async () => {
  stopOverlayServiceButton.disabled = true;
  try {
    const status = await invoke('stop_overlay_service');
    renderOverlayServiceStatus(status);
    setMessage('OBS 服务已停止。');
  } catch (error) {
    setMessage(String(error), true);
    await refreshOverlayServiceStatus();
  }
});

copyOverlayUrlButton.addEventListener('click', async () => {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(OBS_OVERLAY_URL);
    } else {
      const input = document.createElement('textarea');
      input.value = OBS_OVERLAY_URL;
      input.style.position = 'fixed';
      input.style.opacity = '0';
      document.body.appendChild(input);
      input.focus();
      input.select();
      document.execCommand('copy');
      input.remove();
    }
    setMessage('OBS 地址已复制。');
  } catch (error) {
    setMessage(`复制 OBS 地址失败：${String(error)}`, true);
  }
});

copyRandomWeaponsUrlButton.addEventListener('click', async () => {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(OBS_RANDOM_WEAPONS_URL);
    } else {
      const input = document.createElement('textarea');
      input.value = OBS_RANDOM_WEAPONS_URL;
      input.style.position = 'fixed';
      input.style.opacity = '0';
      document.body.appendChild(input);
      input.focus();
      input.select();
      document.execCommand('copy');
      input.remove();
    }
    setMessage('随机武器 OBS 地址已复制。');
  } catch (error) {
    setMessage(`复制随机武器 OBS 地址失败：${String(error)}`, true);
  }
});

copyRandomWeaponsRulesUrlButton.addEventListener('click', async () => {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(OBS_RANDOM_WEAPONS_RULES_URL);
    } else {
      const input = document.createElement('textarea');
      input.value = OBS_RANDOM_WEAPONS_RULES_URL;
      input.style.position = 'fixed';
      input.style.opacity = '0';
      document.body.appendChild(input);
      input.focus();
      input.select();
      document.execCommand('copy');
      input.remove();
    }
    setMessage('私房随武规则 OBS 地址已复制。');
  } catch (error) {
    setMessage(`复制私房随武规则 OBS 地址失败：${String(error)}`, true);
  }
});

refreshPrivateWeaponsButton.addEventListener('click', async () => {
  refreshPrivateWeaponsButton.disabled = true;
  try {
    await invoke('refresh_private_weapons');
    randomWeaponsPreviewFrame?.contentWindow?.postMessage(
      { type: 'refresh-random-weapons' },
      new URL(OBS_RANDOM_WEAPONS_URL).origin,
    );
    setMessage('随机武器已重新抽取。');
  } catch (error) {
    setMessage(String(error), true);
  } finally {
    refreshPrivateWeaponsButton.disabled = false;
  }
});

window.setInterval(() => {
  refreshSyncStatus();
}, 1000);

window.setInterval(() => {
  refreshOverlay();
}, 5000);

window.setInterval(() => {
  refreshOverlayServiceStatus();
}, 2000);

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

cancelLoginButton.addEventListener('click', async () => {
  pastePanel.classList.remove('visible');
  loginLink.value = '';
  try {
    await invoke('cancel_nintendo_login');
    setMessage('已取消添加 Nintendo Account。');
  } catch (error) {
    setMessage(`取消添加账号失败：${String(error)}`, true);
  }
});

async function authenticateNintendoAccount() {
  if (authenticateButton.disabled) {
    return;
  }
  const callbackUrl = loginLink.value.trim();
  if (!callbackUrl) {
    setMessage('请先粘贴 Nintendo“选择此人”按钮的链接。', true);
    return;
  }
  authenticateButton.disabled = true;
  setMessage('正在认证 Nintendo Account 并生成 SplatNet 凭据…');
  try {
    const accountId = await invoke('complete_nintendo_login', { callbackUrl });
    pastePanel.classList.remove('visible');
    loginLink.value = '';
    setMessage('Nintendo Account 登录成功，正在获取 SplatNet 权限并开始首次历史同步…');
    try {
      await invoke('refresh_splatnet_tokens', { accountId });
      const summary = await invoke('sync_history', { accountId });
      await refreshAccounts();
      setMessage(`首次历史同步完成：新增 ${summary.added_battle_count} 条对战、${summary.added_coop_count} 条打工。`);
    } catch (error) {
      await refreshAccounts();
      setMessage(`Nintendo Account 已登录，但 SplatNet 首次同步失败：${String(error)}。可在账号列表中重试。`, true);
    }
  } catch (error) {
    setMessage(String(error), true);
  } finally {
    authenticateButton.disabled = false;
  }
}

authenticateButton.addEventListener('pointerdown', (event) => {
  if (event.button === 0) {
    event.preventDefault();
    void authenticateNintendoAccount();
  }
});
authenticateButton.addEventListener('click', () => {
  void authenticateNintendoAccount();
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
refreshSyncStatus();
refreshOverlayServiceStatus();
