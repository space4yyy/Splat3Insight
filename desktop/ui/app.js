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
const salmonRunPreview = document.querySelector('#salmon-run-preview');
const xMatchPreview = document.querySelector('#x-match-preview');
const nonXMatchPreview = document.querySelector('#non-x-match-preview');
const syncStatusElement = document.querySelector('#sync-status');
const overlayServiceStateElement = document.querySelector('#overlay-service-state');
const startOverlayServiceButton = document.querySelector('#start-overlay-service');
const stopOverlayServiceButton = document.querySelector('#stop-overlay-service');
const backgroundClosePrompt = document.querySelector('#background-close-prompt');
const cancelCloseButton = document.querySelector('#cancel-close');
const exitFromCloseButton = document.querySelector('#exit-from-close');
const hideToTrayButton = document.querySelector('#hide-to-tray');

let currentSyncStatus = null;
let lastCompletedSyncAt = null;
let closePromptAcknowledged = false;
let obsCardCatalog = new Map();
let overlayServiceUrl = null;

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
  message.title = value;
  message.hidden = !value;
  message.classList.toggle('error', error);
}

function renderSyncStatus(status) {
  currentSyncStatus = status;
  if (!status || (status.state === 'idle' && !status.account_id)) {
    syncStatusElement.textContent = '';
    syncStatusElement.title = '';
    syncStatusElement.hidden = true;
    syncStatusElement.classList.remove('error', 'syncing');
    return;
  }

  const busy = ['preparing', 'syncing', 'refreshing'].includes(status.state);
  let label;
  if (busy) {
    label = '刷新中';
  } else if (status.state === 'completed') {
    const completedAt = status.completed_at_ms
      ? new Date(status.completed_at_ms).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
      : '';
    label = completedAt ? `已刷新 ${completedAt}` : '已刷新';
  } else if (status.state === 'error') {
    label = '刷新失败';
  } else {
    label = status.message?.includes('权限') ? '等待授权' : '等待刷新';
  }
  syncStatusElement.textContent = label;
  syncStatusElement.title = label;
  syncStatusElement.hidden = false;
  syncStatusElement.classList.toggle('syncing', busy);
  syncStatusElement.classList.toggle('error', status.state === 'error');
}

function renderOverlayServiceStatus(status) {
  const running = status?.state === 'running';
  const starting = status?.state === 'starting';
  if (status?.url) {
    try {
      overlayServiceUrl = new URL(status.url).origin;
      renderObsCardLinks();
    } catch {}
  }
  overlayServiceStateElement.textContent = running || starting ? '运行中' : '已停止';
  overlayServiceStateElement.className = `overlay-service-state ${running || starting ? 'running' : 'stopped'}`;
  startOverlayServiceButton.hidden = running || starting;
  stopOverlayServiceButton.hidden = !running && !starting;
  startOverlayServiceButton.disabled = starting;
  stopOverlayServiceButton.disabled = starting;
}

async function loadObsCardCatalog() {
  const response = await fetch('./obs-cards.json', { cache: 'no-store' });
  if (!response.ok) {
    throw new Error(`读取 OBS 卡片配置失败（HTTP ${response.status}）`);
  }
  const cards = await response.json();
  if (!Array.isArray(cards) || cards.some((card) => !card.id || !card.route || !card.width || !card.height)) {
    throw new Error('OBS 卡片配置无效');
  }
  obsCardCatalog = new Map(cards.map((card) => [card.id, card]));
  const overlayCard = obsCardCatalog.get('overlay');
  if (overlayCard) {
    document.documentElement.style.setProperty('--obs-card-width', `${overlayCard.width}px`);
    document.documentElement.style.setProperty('--obs-card-height', `${overlayCard.height}px`);
  }
  renderObsCardLinks();

  document.querySelectorAll('[data-obs-card-copy]').forEach((button) => {
    button.addEventListener('click', async () => {
      const card = obsCardCatalog.get(button.dataset.obsCardCopy);
      if (!card || !overlayServiceUrl) return;
      const url = new URL(card.route, overlayServiceUrl).href;
      try {
        if (navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(url);
        } else {
          const input = document.createElement('textarea');
          input.value = url;
          input.style.position = 'fixed';
          input.style.opacity = '0';
          document.body.appendChild(input);
          input.focus();
          input.select();
          document.execCommand('copy');
          input.remove();
        }
        setMessage(`${card.name} OBS 地址已复制。`);
      } catch (error) {
        setMessage(`复制${card.name} OBS 地址失败：${String(error)}`, true);
      }
    });
  });
}

function renderObsCardLinks() {
  if (!obsCardCatalog.size) return;
  for (const [id, card] of obsCardCatalog) {
    const url = overlayServiceUrl ? new URL(card.route, overlayServiceUrl).href : card.route;
    document.querySelectorAll(`[data-obs-card-url="${id}"]`).forEach((element) => {
      element.textContent = url;
    });
    document.querySelectorAll(`[data-obs-card-size="${id}"]`).forEach((element) => {
      element.textContent = `OBS 浏览器源尺寸：${card.width} × ${card.height} px`;
    });
    document.querySelectorAll(`[data-obs-card-description="${id}"]`).forEach((element) => {
      element.textContent = card.description || '';
    });
    document.querySelectorAll(`[data-obs-card-preview="${id}"]`).forEach((frame) => {
      frame.style.width = `${card.width}px`;
      frame.style.height = `${card.height}px`;
      const previewUrl = `${url}${card.previewQuery || ''}`;
      if (frame.src !== previewUrl) frame.src = previewUrl;
    });
  }
}

function fitCardPreview(preview, selector) {
  const viewport = preview.querySelector('.overlay-preview__viewport');
  const scaleElement = preview.querySelector('.overlay-preview__scale');
  const card = preview.querySelector(selector);
  if (!viewport || !scaleElement || !card) {
    return;
  }

  const naturalWidth = card.offsetWidth;
  const naturalHeight = card.offsetHeight;
  const availableWidth = preview.clientWidth;
  const scale = naturalWidth > 0 && availableWidth > 0
    ? Math.min(1, availableWidth / naturalWidth)
    : 1;

  scaleElement.style.transform = `scale(${scale})`;
  viewport.style.height = `${Math.ceil(naturalHeight * scale)}px`;
}

function renderBattlePreview(preview, state, emptyMessage) {
  let scaleElement = preview.querySelector('.overlay-preview__scale');
  if (!scaleElement) {
    preview.innerHTML = `
      <div class="overlay-preview__viewport">
        <div class="overlay-preview__scale"></div>
      </div>`;
    scaleElement = preview.querySelector('.overlay-preview__scale');
  }
  if (window.Splat3OverlayCard.update(scaleElement, state)) {
    fitCardPreview(preview, '.overlay-card');
  } else {
    preview.innerHTML = `<div class="overlay-empty">${emptyMessage}</div>`;
  }
}

function renderOverlay(state, matchStates) {
  let autoScaleElement = overlayPreview.querySelector('.overlay-preview__scale');
  if (!autoScaleElement) {
    overlayPreview.innerHTML = `
      <div class="overlay-preview__viewport">
        <div class="overlay-preview__scale"></div>
      </div>`;
    autoScaleElement = overlayPreview.querySelector('.overlay-preview__scale');
  }
  const autoType = window.Splat3OverlayCard.updateAuto(autoScaleElement, state);
  const autoCardSelector = autoType === 'salmon-run' ? '.salmon-run-card' : '.overlay-card';
  if (autoScaleElement.querySelector(autoCardSelector)) {
    fitCardPreview(overlayPreview, autoCardSelector);
  } else {
    overlayPreview.innerHTML = '<div class="overlay-empty">登录并同步历史后显示最近活动对应的数据卡片。</div>';
  }

  let salmonScaleElement = salmonRunPreview.querySelector('.overlay-preview__scale');
  if (state?.salmon_run) {
    if (!salmonScaleElement) {
      salmonRunPreview.innerHTML = `
        <div class="overlay-preview__viewport">
          <div class="overlay-preview__scale"></div>
        </div>`;
      salmonScaleElement = salmonRunPreview.querySelector('.overlay-preview__scale');
    }
    window.Splat3OverlayCard.updateSalmonRun(salmonScaleElement, state);
    fitCardPreview(salmonRunPreview, '.salmon-run-card');
  } else {
    salmonRunPreview.innerHTML = '<div class="overlay-empty">同步打工历史后显示真实鲑鱼跑数据。</div>';
  }

  renderBattlePreview(xMatchPreview, matchStates.xMatch, '同步对战历史后显示最近 X 赛数据。');
  renderBattlePreview(nonXMatchPreview, matchStates.nonXMatch, '同步对战历史后显示最近非 X 赛数据。');
}

window.addEventListener('resize', () => {
  fitCardPreview(overlayPreview, '.overlay-card, .salmon-run-card');
  fitCardPreview(salmonRunPreview, '.salmon-run-card');
  fitCardPreview(xMatchPreview, '.overlay-card');
  fitCardPreview(nonXMatchPreview, '.overlay-card');
});

async function refreshOverlay(accountId = null) {
  try {
    await Promise.all([
      document.fonts.load('16px "DFP GBZY7"').catch(() => {}),
      document.fonts.load('16px "BlitzMain"').catch(() => {})
    ]);
    const state = await invoke('get_overlay_state', { accountId });
    const xMatchState = await invoke('get_overlay_state', { accountId, matchType: 'xmatch' });
    const nonXMatchState = await invoke('get_overlay_state', { accountId, matchType: 'nonxmatch' });
    renderOverlay(state, { xMatch: xMatchState, nonXMatch: nonXMatchState });
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
          <small class="account-history">${account.history_battle_count} 对战 · ${account.history_coop_count} 打工</small>
        </div>
        <div class="account-actions">
          ${account.active ? '<span class="badge">当前</span>' : `<button class="secondary" data-account="${escapeHtml(account.account_id)}">使用</button>`}
          <button class="secondary" data-export-account="${escapeHtml(account.account_id)}">导出</button>
          <button class="danger" data-delete-account="${escapeHtml(account.account_id)}" data-account-name="${escapeHtml(account.display_name)}">删除</button>
          ${account.splatnet_ready === false || (account.active && account.splatnet_ready === null) ? `<button class="secondary" data-refresh-account="${escapeHtml(account.account_id)}">获取权限</button>` : ''}
        </div>
      </div>`).join('')
    : '<p class="status">点击“添加账号”开始。</p>';

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
  setMessage('');
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
    setMessage('已取消添加账号。');
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

loadObsCardCatalog()
  .then(() => refreshAccounts())
  .catch((error) => setMessage(String(error), true));
refreshSyncStatus();
refreshOverlayServiceStatus();
