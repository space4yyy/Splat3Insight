const invoke = (...args) => window.__TAURI__.core.invoke(...args);
const openUrl = (url) => window.__TAURI__.opener.openUrl(url);

const message = document.querySelector('#message');
const accounts = document.querySelector('#accounts');
const loginButton = document.querySelector('#login');
const pastePanel = document.querySelector('#paste-panel');
const loginLink = document.querySelector('#login-link');
const authenticateButton = document.querySelector('#authenticate');
const overlayPreview = document.querySelector('#overlay-preview');
const refreshOverlayButton = document.querySelector('#refresh-overlay');
const syncStatusElement = document.querySelector('#sync-status');

let currentSyncStatus = null;
let lastCompletedSyncAt = null;

function setMessage(value, error = false) {
  message.textContent = value;
  message.classList.toggle('error', error);
}

function renderSyncStatus(status) {
  currentSyncStatus = status;
  if (!status) {
    syncStatusElement.textContent = '同步状态不可用。';
    syncStatusElement.classList.add('error');
    return;
  }

  const busy = ['preparing', 'syncing', 'refreshing'].includes(status.state);
  const prefix = status.initial ? '首次同步' : '后台同步';
  if (busy) {
    syncStatusElement.textContent = `${prefix}：${status.message}`;
  } else if (status.state === 'completed') {
    syncStatusElement.textContent = `最近同步完成：新增 ${status.added_battle_count} 条对战、${status.added_coop_count} 条打工；共 ${status.battle_count} 条对战、${status.coop_count} 条打工。`;
  } else if (status.state === 'error') {
    syncStatusElement.textContent = `同步失败：${status.error || status.message}`;
  } else {
    syncStatusElement.textContent = status.message;
  }
  syncStatusElement.classList.toggle('syncing', busy);
  syncStatusElement.classList.toggle('error', status.state === 'error');
}

function displayNumber(value, digits = 0) {
  return value === null || value === undefined || !Number.isFinite(Number(value))
    ? '—'
    : Number(value).toFixed(digits);
}

function placeholderClass(value) {
  return value === null || value === undefined || !Number.isFinite(Number(value))
    ? 'is-placeholder'
    : '';
}

const simplifiedRuleLabels = Object.freeze({
  '真格魚虎對戰': '真格鱼虎',
  '真格區域對戰': '真格区域',
  '真格塔樓對戰': '真格塔楼',
  '真格蛤蜊對戰': '真格蛤蜊',
  '真格魚虎': '真格鱼虎',
  '真格區域': '真格区域',
  '真格塔樓': '真格塔楼',
  'Rainmaker': '真格鱼虎',
  'Splat Zones': '真格区域',
  'Tower Control': '真格塔楼',
  'Clam Blitz': '真格蛤蜊',
  'Turf War': '占地对战'
});

function localizeRule(value) {
  const label = String(value ?? '').trim();
  return simplifiedRuleLabels[label] || label || '—';
}

function renderOverlay(state) {
  const battle = state?.selected;
  if (!battle) {
    overlayPreview.innerHTML = '<div class="overlay-empty">没有找到可显示的 X 或蛮颓比赛记录。</div>';
    return;
  }

  const modeClass = battle.mode === 'X_MATCH' ? 'mode-x-match' : 'mode-bankara';
  const bankaraMode = battle.bankara_mode === 'CHALLENGE' ? '挑战'
    : battle.bankara_mode === 'OPEN' ? '开放' : '';
  const summary = state.recent_summary || { win: 0, lose: 0, win_rate: 0 };
  const kills = battle.kills;
  const deaths = battle.deaths;
  const hasKills = kills !== null && kills !== undefined && Number.isFinite(Number(kills));
  const hasDeaths = deaths !== null && deaths !== undefined && Number.isFinite(Number(deaths));
  const kd = hasKills && hasDeaths
    ? Number(deaths) === 0 ? '∞' : displayNumber(Number(kills) / Number(deaths), 1)
    : '—';
  const kdMissing = !hasKills || !hasDeaths;
  const delta = battle.power_delta === null || battle.power_delta === undefined
    ? '—'
    : `${battle.power_delta > 0 ? '+' : ''}${displayNumber(battle.power_delta, 1)}`;
  const deltaClass = battle.power_delta > 0 ? 'positive' : battle.power_delta < 0 ? 'negative' : '';

  overlayPreview.innerHTML = `
    <section class="overlay-card">
      <div class="overlay-card__top">
        <div class="overlay-card__mode ${modeClass}">
          <span>${escapeHtml(localizeRule(battle.rule))}</span>
          ${bankaraMode ? `<small class="overlay-card__bankara-mode">${bankaraMode}</small>` : ''}
        </div>
        <div class="overlay-card__summary">
          <span class="overlay-card__summary-win">Win ${summary.win}</span>
          <span class="overlay-card__summary-lose">Lose ${summary.lose}</span>
          <span>${displayNumber(summary.win_rate, 0)}%</span>
        </div>
      </div>
      <div class="overlay-card__bottom">
        <div class="overlay-card__power">
          <span class="overlay-card__power-value ${placeholderClass(battle.power)}">${displayNumber(battle.power, 1)}</span>
          <span class="overlay-card__power-delta ${deltaClass} ${placeholderClass(battle.power_delta)}">${delta}</span>
        </div>
        <div class="overlay-card__stats">
          <div class="overlay-card__kad">
            <span class="${placeholderClass(kills)}">${displayNumber(kills)}<small>(${displayNumber(battle.assists)})k</small></span>
            <span class="${placeholderClass(deaths)}">${displayNumber(deaths)}<small>d</small></span>
            <span class="${kdMissing ? 'is-placeholder' : ''}">${kd}<small>k/d</small></span>
          </div>
        </div>
      </div>
    </section>`;
}

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
      <div class="account row">
        <div>
          <strong>${escapeHtml(account.display_name)}</strong>
          <br /><small>${escapeHtml(account.language || '—')} · ${escapeHtml(account.account_id)}</small>
          <br /><small>历史：${account.history_battle_count} 对战 · ${account.history_coop_count} 打工</small>
        </div>
        <div class="account-actions">
          ${account.active ? '<span class="badge">当前</span>' : `<button class="secondary" data-account="${escapeHtml(account.account_id)}">使用</button>`}
          ${account.splatnet_ready
            ? `<span class="badge badge-secondary">SplatNet</span><button class="secondary" data-sync-account="${escapeHtml(account.account_id)}" ${currentSyncStatus?.account_id === account.account_id && ['preparing', 'syncing', 'refreshing'].includes(currentSyncStatus.state) ? 'disabled' : ''}>${currentSyncStatus?.account_id === account.account_id && ['preparing', 'syncing', 'refreshing'].includes(currentSyncStatus.state) ? '同步中…' : '同步历史'}</button>`
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

  accounts.querySelectorAll('[data-sync-account]').forEach((button) => {
    button.addEventListener('click', async () => {
      button.disabled = true;
      setMessage('正在从 SplatNet 检查新数据…');
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
  await refreshOverlay(result.find((account) => account.active)?.account_id || null);
  setMessage(result.length ? '账号信息已保存到系统凭据库。' : '请先登录一个 Nintendo Account。');
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

refreshOverlayButton.addEventListener('click', () => refreshOverlay());

window.setInterval(() => {
  refreshSyncStatus();
}, 1000);

window.setInterval(() => {
  refreshOverlay();
}, 5000);

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
    const accountId = await invoke('complete_nintendo_login', { callbackUrl });
    pastePanel.classList.remove('visible');
    loginLink.value = '';
    await refreshAccounts();
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
