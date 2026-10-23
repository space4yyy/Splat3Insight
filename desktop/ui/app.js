(() => {
  'use strict';

  const tauri = window.__TAURI__;
  const invoke = (...args) => tauri.core.invoke(...args);
  const openUrl = (url) => tauri.opener.openUrl(url);
  const revealPath = (path) => tauri.opener.revealItemInDir(path);
  const $ = (selector) => document.querySelector(selector);
  const dashboard = window.Splat3Dashboard;
  const records = window.Splat3Records;
  const analysis = window.Splat3Analysis;
  const cards = window.Splat3OverlayCard;

  // Windows 上换成系统自己的叫法：凭据管理器、系统托盘、文件资源管理器。
  if (/Windows/i.test(navigator.userAgent)) {
    document.querySelectorAll('[data-windows]').forEach((node) => {
      node.textContent = node.dataset.windows;
    });
  }

  // 鼠标点开的下拉框也算 :focus-visible，不改选项就关掉菜单、或者筛选后重新聚焦时，焦点框会一直留着。
  // 记下最近一次用的是鼠标还是键盘；用鼠标时样式里不给下拉框画焦点框，按键后再画。
  const setInput = (mode) => {
    if (document.documentElement.dataset.input !== mode) document.documentElement.dataset.input = mode;
  };
  document.addEventListener('pointerdown', () => setInput('pointer'), true);
  document.addEventListener('keydown', (event) => {
    if (!event.metaKey && !event.ctrlKey && !event.altKey) setInput('keyboard');
  }, true);

  const VIEWS = {
    overview: ['总览', '最近的状态、战力走势和常用配置，一页看完。'],
    battles: ['对战', '规则、连胜连败、击倒和对局时长。'],
    weapons: ['武器', '每把武器的表现、挑战战力，以及对手武器、副武器和特殊武器。'],
    stages: ['场地', '每张地图的胜率、出现时间和武器表现。'],
    salmon: ['鲑鱼跑', '各期打工、评级、头目鲑鱼、特殊波次、个人贡献和击倒数量。'],
    log: ['对战记录', '所有对局，点一行查看详情。'],
    obs: ['其他工具', '直播用的 OBS 数据卡片等附加工具。'],
    settings: ['设置', '账号和开机启动。'],
  };
  const VIEW_KEY = 'splat3-view';
  const OBS_SELECTION_KEY = 'splat3-obs-card-selection';
  const OBS_TRANSPARENCY_KEY = 'splat3-obs-card-transparency';
  // 卡片默认背景 #383838（overlay-card.css），透明度只改它的 alpha。
  const OBS_CARD_RGB = '56, 56, 56';
  const OBS_EMPTY = {
    overlay: '登录并同步历史后显示最近活动卡片。',
    'bankara-match': '同步蛮颓开放或挑战记录后显示最近比赛。',
    'x-match': '同步 X 比赛记录后显示最近比赛。',
    'salmon-run': '同步打工历史后显示鲑鱼跑卡片。',
    'defeat-counts': '同步打工历史后显示击倒数量。',
  };

  const app = {
    view: 'overview',
    accounts: [],
    accountsLoaded: false,
    accountPlayer: null,
    activeAccountId: null,
    facts: { accountId: null, status: 'idle', error: null, loadedAt: 0 },
    lastCompletedSyncAt: null,
    lastStoppedSync: null,
    syncStatus: null,
    authorizing: false,
    authError: null,
    serviceOrigin: null,
    serviceRunning: false,
    obs: { catalog: [], selected: ['overlay'], active: 'overlay', copiedUrl: null, copiedCss: null, transparency: 0, states: null, latest: null },
  };
  const share = window.Splat3Share.init(() => {
    const data = dashboard.records();
    return {
      data,
      view: app.view,
      mode: data.mode,
      name: app.accountPlayer?.name || app.accounts.find((account) => account.account_id === app.activeAccountId)?.display_name || '',
      // 打工分享图的救援图标按物种区分，取 OBS 预览已经读到的最近一场打工。
      species: app.obs.states?.overlay?.salmon_run?.species ?? null,
    };
  }, { invoke, revealPath });
  let factsRequest = 0;
  let overlayRequest = 0;

  function el(tag, props = {}, children = []) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (key === 'text') node.textContent = value;
      else if (key === 'class') node.className = value;
      else node.setAttribute(key, value);
    }
    for (const child of [].concat(children)) if (child != null) node.append(child);
    return node;
  }
  // 原生 append / replaceChildren 会把 null 写成文字，先过滤掉空项。
  const present = (...items) => items.filter((item) => item != null);
  const pad2 = (value) => String(value).padStart(2, '0');
  const fmtClock = (ms) => { const d = new Date(ms); return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`; };

  const message = $('#message');
  const messageText = $('#message-text');
  const messageReveal = $('#message-reveal');
  let messagePath = null;
  // 传入 path 时提示后面带「在访达中显示」，例如导出完成后打开导出文件所在的文件夹。
  function setMessage(value, error = false, path = null) {
    messageText.textContent = value;
    message.title = value;
    message.hidden = !value;
    message.classList.toggle('is-error', error);
    messagePath = path;
    messageReveal.hidden = !path;
  }
  messageReveal.addEventListener('click', async () => {
    if (!messagePath) return;
    try {
      await revealPath(messagePath);
    } catch (error) {
      setMessage(`无法打开所在位置：${String(error)}`, true);
    }
  });

  // 关闭窗口：第一次询问是否在后台运行，之后直接在后台运行。
  const closePrompt = $('#background-close-prompt');
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
  tauri.event.listen('main-close-requested', async () => {
    if (closePrompt.open) return;
    if (closePromptAcknowledged) {
      try {
        await invoke('hide_main_window');
      } catch (error) {
        setMessage(`隐藏主窗口失败：${String(error)}`, true);
      }
      return;
    }
    closePrompt.showModal();
  }).catch((error) => setMessage(`初始化窗口关闭提示失败：${String(error)}`, true));
  $('#cancel-close').addEventListener('click', () => closePrompt.close());
  closePrompt.addEventListener('cancel', (event) => {
    event.preventDefault();
    closePrompt.close();
  });
  $('#hide-to-background').addEventListener('click', async () => {
    acknowledgeClosePrompt();
    closePrompt.close();
    try {
      await invoke('hide_main_window');
    } catch (error) {
      setMessage(`隐藏主窗口失败：${String(error)}`, true);
    }
  });
  $('#exit-from-close').addEventListener('click', async () => {
    acknowledgeClosePrompt();
    closePrompt.close();
    try {
      await invoke('exit_app');
    } catch (error) {
      setMessage(`退出应用失败：${String(error)}`, true);
    }
  });

  // 外观
  const themeSwitch = $('#theme-switch');
  function syncThemeSwitch() {
    const choice = document.documentElement.getAttribute('data-theme') || 'system';
    for (const button of themeSwitch.querySelectorAll('button[data-theme-choice]')) {
      button.setAttribute('aria-pressed', String(button.getAttribute('data-theme-choice') === choice));
    }
  }
  function onThemeChange() {
    dashboard.onThemeChange();
    syncThemeSwitch();
    renderDataView();
  }
  themeSwitch.addEventListener('click', (event) => {
    const button = event.target.closest('button[data-theme-choice]');
    if (!button) return;
    const choice = button.getAttribute('data-theme-choice');
    try {
      if (choice === 'system') localStorage.removeItem('splat3-theme');
      else localStorage.setItem('splat3-theme', choice);
    } catch {}
    if (choice === 'system') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', choice);
  });
  new MutationObserver(onThemeChange).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  matchMedia('(prefers-color-scheme: light)').addEventListener('change', onThemeChange);
  syncThemeSwitch();

  // 开机启动：开关的状态以系统登录项为准，每次打开设置页时重新读取。
  const autostartToggle = $('#autostart');
  const settingsMessage = $('#settings-message');
  function setSettingsMessage(text) {
    settingsMessage.textContent = text;
    settingsMessage.hidden = !text;
  }
  async function refreshAutostart() {
    try {
      autostartToggle.checked = await invoke('get_autostart');
      autostartToggle.disabled = false;
    } catch (error) {
      setSettingsMessage(String(error));
    }
  }
  autostartToggle.addEventListener('change', async () => {
    const enabled = autostartToggle.checked;
    autostartToggle.disabled = true;
    setSettingsMessage('');
    try {
      autostartToggle.checked = await invoke('set_autostart', { enabled });
    } catch (error) {
      autostartToggle.checked = !enabled;
      setSettingsMessage(String(error));
    } finally {
      autostartToggle.disabled = false;
    }
  });

  // 数据导出位置：默认是下载文件夹，「更改」打开系统的文件夹选择窗口。
  const exportDirectory = $('#export-directory');
  const exportDirectoryButton = $('#export-directory-button');
  async function refreshExportDirectory() {
    try {
      exportDirectory.textContent = await invoke('get_export_directory');
    } catch (error) {
      setSettingsMessage(String(error));
    }
  }
  exportDirectoryButton.addEventListener('click', async () => {
    exportDirectoryButton.disabled = true;
    setSettingsMessage('');
    try {
      const directory = await invoke('choose_export_directory');
      if (directory) exportDirectory.textContent = directory;
    } catch (error) {
      setSettingsMessage(String(error));
    } finally {
      exportDirectoryButton.disabled = false;
    }
  });

  const logDirectory = $('#log-directory');
  const openLogDirectoryButton = $('#open-log-directory-button');
  let logDirectoryPath = null;
  async function refreshLogDirectory() {
    logDirectoryPath = null;
    logDirectory.textContent = '读取中…';
    openLogDirectoryButton.disabled = true;
    try {
      logDirectoryPath = await invoke('get_log_directory');
      logDirectory.textContent = logDirectoryPath;
    } catch (error) {
      logDirectory.textContent = '读取失败';
      setSettingsMessage(`读取日志目录失败：${String(error)}`);
    } finally {
      openLogDirectoryButton.disabled = !logDirectoryPath;
    }
  }
  openLogDirectoryButton.addEventListener('click', async () => {
    if (!logDirectoryPath) return;
    openLogDirectoryButton.disabled = true;
    setSettingsMessage('');
    try {
      await invoke('open_log_directory');
    } catch (error) {
      setSettingsMessage(`打开日志目录失败：${String(error)}`);
    } finally {
      openLogDirectoryButton.disabled = !logDirectoryPath;
    }
  });

  // 应用更新：后台发现新版本时提示，点按钮才下载安装并重启，不会自动安装。
  const updateButton = $('#update-button');
  const settingsNav = $('#nav button[data-view="settings"]');
  const update = { version: null, busy: false };
  function renderUpdate(label) {
    updateButton.textContent = label ?? (update.version ? `更新到 ${update.version}` : '检查更新');
    updateButton.className = update.version ? 'button-primary' : 'ghost';
    updateButton.disabled = update.busy;
    settingsNav.classList.toggle('has-update', Boolean(update.version));
  }
  function updateFound(info) {
    update.version = info?.version ?? null;
    if (!update.busy) renderUpdate();
  }
  invoke('get_app_version').then((version) => { $('#app-version').textContent = version; }).catch(() => {});
  tauri.event.listen('update-available', (event) => updateFound(event.payload));
  tauri.event.listen('update-progress', (event) => {
    const { downloaded, total } = event.payload || {};
    renderUpdate(total ? `正在下载 ${Math.min(99, Math.floor((downloaded / total) * 100))}%` : '正在下载…');
  });
  updateButton.addEventListener('click', async () => {
    if (update.busy) return;
    update.busy = true;
    setSettingsMessage('');
    let label;
    try {
      if (update.version) {
        renderUpdate('正在下载…');
        // 安装完成后应用自动重启。
        await invoke('install_update');
      } else {
        renderUpdate('正在检查…');
        const info = await invoke('check_for_update');
        update.version = info?.version ?? null;
        if (!info) label = '已是最新版本';
      }
    } catch (error) {
      setSettingsMessage(String(error));
    } finally {
      update.busy = false;
      renderUpdate(label);
      if (label) setTimeout(() => { if (!update.busy && !update.version) renderUpdate(); }, 3000);
    }
  });

  // 页面切换。每个页面各自记住滚动位置，切回来时回到原处。
  const scrollPositions = {};
  function showView(view) {
    // 账号管理并进了设置页，旧的 #accounts 链接也打开设置。
    if (view === 'accounts') view = 'settings';
    if (!VIEWS[view]) view = 'overview';
    const previous = app.view;
    if (previous !== view) scrollPositions[previous] = window.scrollY;
    app.view = view;
    // 重新打开应用时回到这一页。
    try {
      localStorage.setItem(VIEW_KEY, view);
    } catch {}
    for (const button of document.querySelectorAll('#nav button[data-view]')) {
      if (button.getAttribute('data-view') === view) button.setAttribute('aria-current', 'page');
      else button.removeAttribute('aria-current');
    }
    for (const section of document.querySelectorAll('.main > .view')) section.hidden = section.id !== `view-${view}`;
    $('#page-title').textContent = VIEWS[view][0];
    $('#page-sub').textContent = VIEWS[view][1];
    dashboard.hideTip();
    $('#filters').hidden = true;
    $('#share-button').hidden = !['overview', 'battles', 'weapons', 'stages', 'salmon'].includes(view);
    dashboard.setModeScope(view === 'salmon' ? 'salmon' : ['battles', 'weapons', 'stages'].includes(view) ? 'battles' : 'all');
    if (previous !== view) records.close();
    if (view === 'overview') renderOverview();
    else if (analysis.VIEWS.includes(view)) renderAnalysis(view);
    else if (view === 'log') renderLog();
    else if (view === 'obs') {
      renderObsPreview();
      refreshOverlayStates();
    } else if (view === 'settings') {
      refreshAutostart();
      refreshExportDirectory();
      refreshLogDirectory();
      refreshAccounts().catch((error) => setMessage(String(error), true));
    }
    if (previous !== view) window.scrollTo(0, scrollPositions[view] ?? 0);
  }
  $('#nav').addEventListener('click', (event) => {
    const button = event.target.closest('button[data-view]');
    if (!button) return;
    const view = button.getAttribute('data-view');
    if (location.hash !== `#${view}`) location.hash = view;
    else showView(view);
  });
  $('#recent-more').addEventListener('click', () => goTo('log'));
  window.addEventListener('hashchange', () => showView(location.hash.slice(1)));
  function goTo(view) {
    if (location.hash !== `#${view}`) location.hash = view;
    else showView(view);
  }

  // 总览、分析页和对战记录共用的提示：账号、数据整理和读取失败等状态；没有提示时才绘制页面。
  function dataNotice() {
    if (!app.accountsLoaded) return { title: '正在读取账号…' };
    if (!app.activeAccountId) {
      return {
        title: '还没有添加账号',
        body: '添加 Nintendo 账号并完成首次同步后，这里会显示你的对战和打工数据。',
        action: { label: '添加账号', run: startLogin },
      };
    }
    if (app.facts.accountId === app.activeAccountId && app.facts.status === 'error') {
      return { title: '读取历史数据失败', body: app.facts.error, action: { label: '重试', run: () => loadFacts({ force: true }) } };
    }
    if (app.facts.accountId !== app.activeAccountId || app.facts.status !== 'ready') {
      return { title: '正在整理历史数据…', body: '第一次打开时会为全部历史记录建立索引，记录很多时需要一点时间。' };
    }
    if (!dashboard.hasRecords()) return { title: '还没有对战或打工记录', body: '同步完成后，这里会显示你的数据。同步进度显示在左下角。' };
    return null;
  }
  function showDataNotice(status, content) {
    const notice = dataNotice();
    status.hidden = !notice;
    content.hidden = Boolean(notice);
    $('#filters').hidden = Boolean(notice);
    if (!notice) return false;
    const action = notice.action ? el('button', { type: 'button', class: 'button-primary', text: notice.action.label }) : null;
    if (action) action.addEventListener('click', notice.action.run);
    status.replaceChildren(...present(el('strong', { text: notice.title }), notice.body ? el('span', { text: notice.body }) : null, action));
    return true;
  }
  function renderOverview() {
    if (!showDataNotice($('#overview-status'), $('#overview-content'))) dashboard.render();
  }
  function renderLog() {
    if (!showDataNotice($('#log-status'), $('#log-content'))) records.render();
  }
  function renderAnalysis(view) {
    if (!showDataNotice($(`#${view}-status`), $(`#${view}-content`))) analysis.render(view);
  }
  // 重新绘制时页面会先变短，浏览器随之把滚动位置往上收，画完也不会回去。画之前撑住当前页面的
  // 高度，画完放开，再回到原来的位置：数据更新、换筛选、窗口变宽变窄时都停在原处。
  function keepScroll(render) {
    const view = document.querySelector('.main > .view:not([hidden])');
    const top = window.scrollY;
    if (view) view.style.minHeight = `${view.offsetHeight}px`;
    try {
      render();
    } finally {
      if (view) view.style.minHeight = '';
      if (window.scrollY !== top) window.scrollTo(0, top);
    }
  }
  function renderDataView() {
    keepScroll(() => {
      if (app.view === 'overview') renderOverview();
      else if (analysis.VIEWS.includes(app.view)) renderAnalysis(app.view);
      else if (app.view === 'log') renderLog();
      // 击倒数量卡片的预览用数据面板的数据，读到新数据时一起刷新。
      else if (app.view === 'obs') renderObsPreview();
    });
  }
  dashboard.onFiltersChange(renderDataView);
  // 总览里点最近的一场：就在总览上打开这一条的详情，不切换页面。
  dashboard.onOpenRecord((kind, id) => records.show(kind, id));

  async function loadFacts({ force = false } = {}) {
    const accountId = app.activeAccountId;
    if (!accountId) {
      app.facts = { accountId: null, status: 'idle', error: null, loadedAt: 0 };
      dashboard.setData(null);
      share.close();
      renderDataView();
      return;
    }
    const firstLoad = app.facts.accountId !== accountId || app.facts.status !== 'ready';
    if (!force && app.facts.accountId === accountId && app.facts.status === 'loading') return;
    const requestId = ++factsRequest;
    if (firstLoad) {
      app.facts = { accountId, status: 'loading', error: null, loadedAt: 0 };
      renderDataView();
    }
    try {
      const facts = await invoke('get_dashboard_facts', { accountId });
      if (requestId !== factsRequest) return;
      dashboard.setData(facts);
      void refreshAccountPlayer(facts, accountId, requestId);
      app.facts = { accountId, status: 'ready', error: null, loadedAt: Date.now() };
    } catch (error) {
      if (requestId !== factsRequest) return;
      // 已有数据时保留旧图表，只有首次读取失败才显示错误。
      if (firstLoad) app.facts = { accountId, status: 'error', error: String(error), loadedAt: 0 };
    }
    renderDataView();
  }

  // 账号
  const accountButton = $('#account-button');
  const accountMenu = $('#account-menu');
  const accountInitial = (name) => Array.from(String(name || '').trim())[0] || '?';
  const accountSummary = (account) => `${account.history_battle_count} 场对战 · ${account.history_coop_count} 场打工`;
  const activeAccount = () => app.accounts.find((account) => account.active) || null;

  const accountPlayers = new Map();
  const accountPlayerRequests = new Map();
  const accountHistorySignature = (account) => [account.history_fetched_at_ms, account.history_battle_count, account.history_coop_count].join('|');

  // 取最近一次游戏活动中的本人名牌，不受面板的时间范围和模式筛选影响。
  async function loadAccountPlayer(facts, accountId) {
    let latest = null;
    for (const [kind, rows] of [['battle', facts.battles], ['coop', facts.coop]]) {
      for (const row of rows || []) {
        if (row.played_at_ms != null && (!latest || row.played_at_ms > latest.time)) {
          latest = { kind, id: row.id, time: row.played_at_ms };
        }
      }
    }
    const raw = latest ? await invoke('get_history_record', { accountId, recordType: latest.kind, recordId: latest.id }) : null;
    const detail = window.Splat3RecordDetail;
    return raw ? (latest.kind === 'battle'
      ? detail.battleDetail(raw).teams[0]?.players.find((player) => player.me)
      : detail.shiftDetail(raw).members.find((player) => player.me)) : null;
  }

  async function refreshAccountPlayer(facts, accountId, requestId) {
    try {
      const player = await loadAccountPlayer(facts, accountId);
      if (accountId !== app.activeAccountId || requestId !== factsRequest) return;
      app.accountPlayer = player;
      const account = activeAccount();
      if (account) accountPlayers.set(accountId, { signature: accountHistorySignature(account), player });
      renderAccountDisplay();
      updateAccountMenuPlates();
    } catch {
      // 暂时读不到归档时保留当前名牌，首次读取则使用账号名称。
    }
  }

  function renderAccountItem(button, account) {
    const player = account.active ? app.accountPlayer : accountPlayers.get(account.account_id)?.player;
    const name = player?.name || account.display_name;
    button.replaceChildren(records.nameplate(name, player?.nameplate));
    button.setAttribute('aria-label', `${name}，Nintendo 账号 ${account.display_name}${account.active ? '，当前账号' : ''}`);
    button.title = `Nintendo 账号：${account.display_name}`;
  }

  function updateAccountMenuPlates() {
    for (const button of accountMenu.querySelectorAll('[data-account]')) {
      const account = app.accounts.find((item) => item.account_id === button.getAttribute('data-account'));
      if (account) renderAccountItem(button, account);
    }
  }

  // 菜单展开时按需读取其他账号；用同步时间缓存，不在每次展开时重复读取历史。
  async function loadAccountMenuPlates() {
    await Promise.allSettled(app.accounts.filter((account) => !account.active).map(async (account) => {
      const id = account.account_id;
      const signature = accountHistorySignature(account);
      if (accountPlayers.get(id)?.signature === signature || accountPlayerRequests.has(id)) return;
      const request = (async () => {
        const facts = await invoke('get_dashboard_facts', { accountId: id });
        const player = await loadAccountPlayer(facts, id);
        const current = app.accounts.find((item) => item.account_id === id);
        if (!current || accountHistorySignature(current) !== signature) return;
        accountPlayers.set(id, { signature, player });
        updateAccountMenuPlates();
      })();
      accountPlayerRequests.set(id, request);
      try { await request; } finally { accountPlayerRequests.delete(id); }
    }));
  }

  function renderAccountDisplay() {
    const active = activeAccount();
    const player = app.accountPlayer;
    const name = active ? player?.name || active.display_name : '未添加账号';
    accountButton.classList.toggle('has-nameplate', Boolean(active));
    accountButton.replaceChildren(...(active ? [
      records.nameplate(name, player?.nameplate),
      el('span', { class: 'account-caret', 'aria-hidden': 'true' }),
    ] : present(
      el('span', { class: active ? 'avatar' : 'avatar is-idle', 'aria-hidden': 'true', text: active ? accountInitial(active.display_name) : '+' }),
      el('span', { class: 'account-text' }, [
        el('strong', { text: active ? active.display_name : '未添加账号' }),
        active ? null : el('small', { text: '添加 Nintendo 账号' }),
      ]),
      el('span', { class: 'account-caret', 'aria-hidden': 'true' }),
    )));
    accountButton.setAttribute('aria-label', active ? `当前账号 ${name}，切换账号` : '添加账号');
  }

  function renderAccountSwitcher() {
    renderAccountDisplay();
    const items = app.accounts.map((account) => {
      const button = el('button', {
        type: 'button',
        class: 'account-item',
        role: 'menuitemradio',
        'aria-checked': String(account.active),
        'data-account': account.account_id,
      });
      renderAccountItem(button, account);
      return button;
    });
    accountMenu.replaceChildren(...present(
      el('div', { class: 'account-menu-title', text: '切换账号' }),
      ...items,
      items.length ? el('div', { class: 'account-menu-divider', role: 'separator' }) : null,
      el('button', { type: 'button', class: 'account-action', role: 'menuitem', 'data-account-action': 'add', text: '添加账号' }),
      el('button', { type: 'button', class: 'account-action', role: 'menuitem', 'data-account-action': 'manage', text: '管理账号' }),
    ));
  }
  function setAccountMenu(open) {
    accountMenu.hidden = !open;
    accountButton.setAttribute('aria-expanded', String(open));
    if (open) void loadAccountMenuPlates();
    if (open) (accountMenu.querySelector('[aria-checked="true"]') || accountMenu.querySelector('button'))?.focus();
  }
  accountButton.addEventListener('click', () => setAccountMenu(accountMenu.hidden));
  accountMenu.addEventListener('click', (event) => {
    const item = event.target.closest('[data-account]');
    const action = event.target.closest('[data-account-action]');
    if (item) {
      setAccountMenu(false);
      accountButton.focus();
      if (item.getAttribute('aria-checked') !== 'true') switchAccount(item.getAttribute('data-account'));
    } else if (action) {
      setAccountMenu(false);
      if (action.getAttribute('data-account-action') === 'add') startLogin();
      else goTo('settings');
    }
  });
  document.addEventListener('click', (event) => {
    if (!accountMenu.hidden && !event.target.closest('.account-switcher')) setAccountMenu(false);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !accountMenu.hidden) {
      setAccountMenu(false);
      accountButton.focus();
    }
  });

  async function switchAccount(accountId) {
    try {
      await invoke('set_active_account', { accountId });
      await refreshAccounts();
    } catch (error) {
      setMessage(String(error), true);
    }
  }

  // 导出在后台进行，期间账号列表可能重画：按账号记下正在导出的，按钮保持不可点。
  const exporting = new Set();
  function accountActionButton(label, run, className = 'ghost') {
    const button = el('button', { type: 'button', class: className, text: label });
    button.addEventListener('click', () => run(button));
    return button;
  }
  function renderAccountList() {
    const list = $('#account-list');
    if (!app.accounts.length) {
      list.replaceChildren(el('p', { class: 'account-empty', text: '点击“添加账号”开始。' }));
      return;
    }
    list.replaceChildren(...app.accounts.map((account) => {
      const id = account.account_id;
      const name = account.display_name;
      const actions = el('div', { class: 'account-row-actions' });
      if (account.active) actions.append(el('span', { class: 'account-badge', text: '当前' }));
      else actions.append(accountActionButton('使用', () => switchAccount(id)));
      if (account.splatnet_ready === false || (account.active && account.splatnet_ready === null)) {
        actions.append(accountActionButton('获取权限', (button) => refreshPermission(id, button)));
      }
      const exportButton = accountActionButton('导出', (button) => exportAccount(id, button));
      exportButton.disabled = exporting.has(id);
      actions.append(exportButton);
      actions.append(accountActionButton('导入', (button) => chooseImportFile(id, button)));
      actions.append(accountActionButton('删除', () => openDeleteDialog(id, name)));
      return el('div', { class: 'account-row' }, [
        el('span', { class: account.active ? 'avatar' : 'avatar is-idle', 'aria-hidden': 'true', text: accountInitial(name) }),
        el('span', { class: 'account-text' }, [el('strong', { text: name }), el('small', { text: accountSummary(account) })]),
        actions,
      ]);
    }));
  }

  // 获取 SplatNet 权限：设置页账号列表里的「获取权限」和侧栏的「等待授权」共用。
  async function refreshPermission(accountId, button) {
    if (app.authorizing) return;
    app.authorizing = true;
    app.authError = null;
    button.disabled = true;
    renderSyncStatus(app.syncStatus);
    setMessage('正在使用已保存的 Nintendo 登录信息获取 SplatNet 权限…');
    try {
      await invoke('refresh_splatnet_tokens', { accountId });
    } catch (error) {
      app.authError = String(error);
      setMessage(String(error), true);
      button.disabled = false;
      return;
    } finally {
      app.authorizing = false;
      renderSyncStatus(app.syncStatus);
    }
    try {
      await refreshAccounts();
      setMessage('SplatNet 权限获取成功，正在同步历史…');
      const summary = await invoke('sync_history', { accountId });
      await refreshAccounts();
      setMessage(`历史同步完成：新增 ${summary.added_battle_count} 条对战、${summary.added_coop_count} 条打工。`);
      await loadFacts({ force: true });
    } catch (error) {
      setMessage(String(error), true);
      button.disabled = false;
    }
  }

  async function exportAccount(accountId, button) {
    if (exporting.has(accountId)) return;
    exporting.add(accountId);
    button.disabled = true;
    setMessage('正在导出…');
    try {
      const path = await invoke('export_history', { accountId });
      setMessage(`历史数据已导出：${path}`, false, path);
    } catch (error) {
      setMessage(`导出历史数据失败：${String(error)}`, true);
    } finally {
      exporting.delete(accountId);
      renderAccountList();
    }
  }

  // 导入：读入本应用导出的 .json.gz，本地已有的记录不会重复写入。
  const importInput = $('#import-file');
  let importTarget = null;
  let importing = false;
  function chooseImportFile(accountId, button) {
    if (importing) return;
    importTarget = { accountId, button };
    importInput.value = '';
    importInput.click();
  }
  importInput.addEventListener('change', async () => {
    const file = importInput.files?.[0];
    const target = importTarget;
    importTarget = null;
    if (!file || !target || importing) return;
    importing = true;
    target.button.disabled = true;
    setMessage('正在导入…');
    try {
      const archive = new Uint8Array(await file.arrayBuffer());
      const summary = await invoke('import_history', archive, { headers: { 'Account-Id': target.accountId } });
      await refreshAccounts();
      const { added_battle_count: battles, added_coop_count: coop, skipped_count: skipped } = summary;
      setMessage(battles + coop
        ? `导入完成：新增 ${battles} 条对战、${coop} 条打工${skipped ? `，${skipped} 条本地已有` : ''}。`
        : '文件里的记录本地都已有。');
      if (target.accountId === app.activeAccountId) {
        await loadFacts({ force: true });
        refreshOverlayStates();
      }
    } catch (error) {
      setMessage(`导入历史数据失败：${String(error)}`, true);
    } finally {
      importing = false;
      target.button.disabled = false;
    }
  });

  // 删除账号：默认只移除登录信息、保留本地数据；勾选后一并删除，并提示先导出一份备份。
  const deleteDialog = $('#delete-account-dialog');
  const deleteLocalData = $('#delete-local-data');
  const deleteConfirm = $('#delete-account-confirm');
  const deleteExport = $('#delete-export-button');
  const deleteReveal = $('#delete-export-reveal');
  const deleteStatus = $('#delete-account-status');
  let deleteTarget = null;

  function setDeleteStatus(text, error = false) {
    deleteStatus.textContent = text;
    deleteStatus.hidden = !text;
    deleteStatus.classList.toggle('is-error', error);
  }
  function renderDeleteDialog() {
    const withData = deleteLocalData.checked;
    $('#delete-export').hidden = !withData;
    deleteConfirm.textContent = withData ? '删除账号和数据' : '删除账号';
    deleteExport.textContent = deleteTarget?.exportPath ? '再导出一份' : '导出数据';
    deleteReveal.hidden = !deleteTarget?.exportPath;
  }
  function openDeleteDialog(accountId, name) {
    deleteTarget = { id: accountId, name: name || '此账号', exportPath: null };
    $('#delete-account-title').textContent = `删除账号“${deleteTarget.name}”`;
    deleteLocalData.checked = false;
    setDeleteStatus('');
    renderDeleteDialog();
    deleteDialog.showModal();
  }
  deleteLocalData.addEventListener('change', renderDeleteDialog);
  deleteExport.addEventListener('click', async () => {
    const target = deleteTarget;
    if (!target) return;
    deleteExport.disabled = true;
    deleteConfirm.disabled = true;
    setDeleteStatus('正在导出…');
    try {
      target.exportPath = await invoke('export_history', { accountId: target.id });
      if (deleteTarget === target) setDeleteStatus(`已导出：${target.exportPath}`);
    } catch (error) {
      if (deleteTarget === target) setDeleteStatus(`导出失败：${String(error)}`, true);
    } finally {
      deleteExport.disabled = false;
      deleteConfirm.disabled = false;
      if (deleteTarget === target) renderDeleteDialog();
    }
  });
  deleteReveal.addEventListener('click', async () => {
    try {
      await revealPath(deleteTarget.exportPath);
    } catch (error) {
      setDeleteStatus(`无法打开所在位置：${String(error)}`, true);
    }
  });
  $('#delete-account-cancel').addEventListener('click', () => deleteDialog.close());
  // 正在删除或导出时不能用 Esc 关掉对话框。
  deleteDialog.addEventListener('cancel', (event) => {
    if (deleteConfirm.disabled) event.preventDefault();
  });
  deleteDialog.addEventListener('close', () => {
    deleteTarget = null;
  });
  deleteConfirm.addEventListener('click', async () => {
    const target = deleteTarget;
    if (!target) return;
    const withData = deleteLocalData.checked;
    deleteConfirm.disabled = true;
    deleteExport.disabled = true;
    setDeleteStatus(withData ? '正在删除账号和本地数据…' : '正在删除账号…');
    try {
      await invoke('remove_account', { accountId: target.id, deleteLocalData: withData });
      deleteDialog.close();
      setMessage(withData ? `已删除“${target.name}”及其本地数据。` : `已删除“${target.name}”，本地数据已保留。`);
      await refreshAccounts();
    } catch (error) {
      setDeleteStatus(`删除失败：${String(error)}`, true);
    } finally {
      deleteConfirm.disabled = false;
      deleteExport.disabled = false;
    }
  });

  async function refreshAccounts() {
    const accounts = await invoke('list_accounts');
    app.accounts = accounts;
    for (const id of accountPlayers.keys()) {
      if (!accounts.some((account) => account.account_id === id)) accountPlayers.delete(id);
    }
    app.accountsLoaded = true;
    const activeId = accounts.find((account) => account.active)?.account_id ?? null;
    const changed = activeId !== app.activeAccountId;
    app.activeAccountId = activeId;
    if (changed) {
      app.accountPlayer = null;
      share.close();
    }
    renderAccountSwitcher();
    renderAccountList();
    if (!accountMenu.hidden) void loadAccountMenuPlates();
    if (changed) {
      app.obs.latest = null;
      app.obs.states = null;
      loadFacts();
      refreshOverlayStates();
    }
    renderDataView();
  }

  // 添加账号
  const loginButton = $('#login');
  const pastePanel = $('#paste-panel');
  const loginLink = $('#login-link');
  const authenticateButton = $('#authenticate');
  function startLogin() {
    goTo('settings');
    if (!loginButton.disabled) loginButton.click();
  }
  // nxapi 的使用要求：登录前说明哪些数据会发给第三方服务，用户确认后才开始。
  const loginConsent = $('#login-consent');
  $('#login-consent-link').addEventListener('click', (event) => {
    event.preventDefault();
    void openUrl(event.currentTarget.href);
  });
  document.querySelectorAll('.settings-external-link').forEach((link) => {
    link.addEventListener('click', (event) => {
      event.preventDefault();
      void openUrl(link.href).catch((error) => setSettingsMessage(`打开链接失败：${String(error)}`));
    });
  });
  $('#login-consent-cancel').addEventListener('click', () => loginConsent.close());
  $('#login-consent-accept').addEventListener('click', () => {
    loginConsent.close();
    void beginLogin();
  });
  loginButton.addEventListener('click', () => loginConsent.showModal());
  async function beginLogin() {
    loginButton.disabled = true;
    setMessage('正在打开 Nintendo 登录页面…');
    try {
      const url = await invoke('begin_nintendo_login');
      await openUrl(url);
      pastePanel.hidden = false;
      loginLink.focus();
      setMessage('请在浏览器登录，然后复制“选择此人”按钮的链接。');
    } catch (error) {
      setMessage(String(error), true);
    } finally {
      loginButton.disabled = false;
    }
  }
  $('#cancel-login').addEventListener('click', async () => {
    pastePanel.hidden = true;
    loginLink.value = '';
    try {
      await invoke('cancel_nintendo_login');
      setMessage('已取消添加账号。');
    } catch (error) {
      setMessage(`取消添加账号失败：${String(error)}`, true);
    }
  });
  async function authenticateNintendoAccount() {
    if (authenticateButton.disabled) return;
    const callbackUrl = loginLink.value.trim();
    if (!callbackUrl) {
      setMessage('请先粘贴 Nintendo“选择此人”按钮的链接。', true);
      return;
    }
    authenticateButton.disabled = true;
    setMessage('正在认证 Nintendo Account 并生成 SplatNet 凭据…');
    try {
      const accountId = await invoke('complete_nintendo_login', { callbackUrl });
      pastePanel.hidden = true;
      loginLink.value = '';
      setMessage('Nintendo Account 登录成功，正在获取 SplatNet 权限并开始首次历史同步…');
      try {
        await invoke('refresh_splatnet_tokens', { accountId });
        const summary = await invoke('sync_history', { accountId });
        await refreshAccounts();
        setMessage(`首次历史同步完成：新增 ${summary.added_battle_count} 条对战、${summary.added_coop_count} 条打工。`);
        await loadFacts({ force: true });
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

  // 同步状态
  function syncSummary(status) {
    if (!status || (status.state === 'idle' && !status.account_id)) return { text: '还没有账号', tone: 'is-idle' };
    if (['preparing', 'syncing', 'refreshing'].includes(status.state)) {
      return { text: status.progress_total > 0 ? `同步中 ${status.progress_completed}/${status.progress_total}` : '同步中…', tone: 'is-busy' };
    }
    if (status.state === 'completed') return { text: status.completed_at_ms ? `已同步 · ${fmtClock(status.completed_at_ms)}` : '已同步', tone: '' };
    if (status.state === 'error') return { text: '同步失败', tone: 'is-error' };
    if (!status.message?.includes('权限')) return { text: '等待同步', tone: 'is-idle' };
    // 等待授权时，这段文字本身就是按钮，点一下获取 SplatNet 权限。
    if (app.authorizing) return { text: '正在授权…', tone: 'is-busy' };
    if (app.authError) return { text: '授权失败', tone: 'is-error', authorize: true };
    return { text: '等待授权', tone: 'is-idle', authorize: true };
  }
  function renderSyncStatus(status) {
    app.syncStatus = status;
    const summary = syncSummary(status);
    if (!summary.authorize && !app.authorizing) app.authError = null;
    const foot = $('#sidebar-sync');
    // 每秒都会刷新状态；内容没变时不重画，免得按钮在点击时被换掉。
    const signature = [summary.text, summary.tone, summary.authorize ? status.account_id : ''].join('|');
    if (foot.dataset.signature !== signature) {
      foot.dataset.signature = signature;
      const dot = el('span', { class: `status-dot ${summary.tone}`.trim(), 'aria-hidden': 'true' });
      if (summary.authorize) {
        const button = el('button', { type: 'button', class: 'sync-auth', text: summary.text });
        button.addEventListener('click', () => void refreshPermission(status.account_id, button));
        foot.replaceChildren(dot, button);
      } else {
        foot.replaceChildren(dot, document.createTextNode(summary.text));
      }
    }
    foot.title = (summary.authorize && app.authError) || status?.error || status?.message || '';
    foot.classList.toggle('is-error', summary.tone === 'is-error');
    // 有账号、已获取权限、当前没在同步时，可以立即同步一次。
    const busy = ['preparing', 'syncing', 'refreshing'].includes(status?.state);
    $('#sync-now').hidden = !status?.account_id || busy || Boolean(status?.message?.includes('权限'));
  }
  // reason 写进同步日志：manual 是“立即同步”，window_focus 是窗口回到前台。
  async function requestSync(reason) {
    try {
      await invoke('request_sync', { reason });
    } catch (error) {
      setMessage(String(error), true);
    }
    void refreshSyncStatus();
  }
  $('#sync-now').addEventListener('click', () => {
    void requestSync('manual');
  });
  // 窗口回到前台时，距上次同步超过 1 分钟就立即同步一次，打开就能看到最新数据。
  const FOCUS_SYNC_AFTER_MS = 60000;
  window.addEventListener('focus', () => {
    if (app.lastCompletedSyncAt && Date.now() - app.lastCompletedSyncAt > FOCUS_SYNC_AFTER_MS) void requestSync('window_focus');
  });
  async function refreshSyncStatus() {
    try {
      const status = await invoke('get_sync_status');
      renderSyncStatus(status);
      const completedAt = status?.completed_at_ms || null;
      if (status?.state === 'completed' && completedAt && completedAt !== app.lastCompletedSyncAt) {
        app.lastCompletedSyncAt = completedAt;
        await refreshAccounts();
        // 有新记录或已有记录更新过才重新读取面板数据；读取失败过、或者过了零点（“今天”“近 7 天”跟着日期变）也读一次。
        const added = (status.added_battle_count || 0) + (status.added_coop_count || 0);
        if (added > 0 || status.records_updated) await loadFacts({ force: true });
        else if (app.facts.status !== 'ready' || new Date(app.facts.loadedAt).toDateString() !== new Date().toDateString()) await loadFacts();
        refreshOverlayStates();
      }
      // 同步失败或停下等待权限时，账号可能刚被标为需要重新获取权限；重新读取账号列表，「获取权限」才会出现。
      const stopped = ['error', 'idle'].includes(status?.state)
        ? [status.state, status.account_id, status.message, status.error].join('|')
        : null;
      if (stopped && stopped !== app.lastStoppedSync) {
        app.lastStoppedSync = stopped;
        await refreshAccounts();
      }
    } catch (error) {
      renderSyncStatus({ state: 'error', message: String(error), error: String(error) });
    }
  }

  // OBS 服务
  const serviceToggle = $('#obs-service-toggle');
  function renderServiceStatus(status) {
    const running = status?.state === 'running';
    const starting = status?.state === 'starting';
    const failed = status?.state === 'error';
    app.serviceRunning = running;
    if (status?.url) {
      try {
        app.serviceOrigin = new URL(status.url).origin;
      } catch {}
    }
    const host = app.serviceOrigin ? new URL(app.serviceOrigin).host : '';
    const label = running ? '运行中' : starting ? '启动中…' : failed ? '出错' : '已停止';
    const tone = running ? '' : starting ? 'is-busy' : failed ? 'is-error' : 'is-idle';

    const foot = $('#sidebar-obs');
    foot.replaceChildren(document.createTextNode(running ? 'OBS 服务运行中' : starting ? 'OBS 服务启动中…' : failed ? 'OBS 服务出错' : 'OBS 服务已停止'));
    if (running && host) foot.append(document.createTextNode(' · '), el('span', { class: 'font-number', text: host }));
    foot.classList.toggle('is-error', failed);

    $('#obs-service-state').replaceChildren(...present(
      el('span', { class: `status-dot ${tone}`.trim(), 'aria-hidden': 'true' }),
      document.createTextNode(label),
      host ? el('span', { class: 'service-addr font-number', text: host }) : null,
    ));
    serviceToggle.hidden = false;
    serviceToggle.textContent = running || starting ? '停止服务' : '启动服务';
    serviceToggle.className = running || starting ? 'ghost' : 'button-primary';
    serviceToggle.disabled = starting;
    const warning = $('#obs-url-warning');
    warning.hidden = running || starting;
    warning.textContent = failed ? (status?.error || status?.message || 'OBS 服务出错') : '服务已停止，OBS 里的卡片不会更新。';
    warning.classList.toggle('is-error', failed);
    renderObsUrl();
  }
  async function refreshServiceStatus() {
    try {
      renderServiceStatus(await invoke('get_overlay_service_status'));
    } catch (error) {
      renderServiceStatus({ state: 'error', error: String(error) });
    }
  }
  serviceToggle.addEventListener('click', async () => {
    serviceToggle.disabled = true;
    try {
      renderServiceStatus(await invoke(app.serviceRunning ? 'stop_overlay_service' : 'start_overlay_service'));
    } catch (error) {
      renderServiceStatus({ state: 'error', error: String(error) });
    }
  });

  // OBS 数据卡片
  const obsCardOf = (id) => app.obs.catalog.find((card) => card.id === id);
  const obsOrdered = (ids) => app.obs.catalog.map((card) => card.id).filter((id) => ids.includes(id));
  // 地址带参数的数据卡片能合进同一个地址；自动、时钟这类地址独立的卡片只能单选。
  const obsStandalone = (id) => !obsCardOf(id)?.route.includes('?');

  function normalizeSelection(ids) {
    const known = obsOrdered(ids || []);
    if (!known.length || known.includes('overlay')) return ['overlay'];
    const standalone = known.find(obsStandalone);
    return standalone ? [standalone] : known;
  }
  function saveSelection() {
    try {
      localStorage.setItem(OBS_SELECTION_KEY, JSON.stringify(app.obs.selected));
    } catch {}
  }
  async function loadObsCatalog() {
    const response = await fetch('./obs-cards.json', { cache: 'no-store' });
    if (!response.ok) throw new Error(`读取 OBS 卡片配置失败（HTTP ${response.status}）`);
    const catalog = await response.json();
    if (!Array.isArray(catalog) || catalog.some((card) => !card.id || !card.optionLabel || !card.route || !card.width || !card.height)) {
      throw new Error('OBS 卡片配置无效');
    }
    app.obs.catalog = catalog;
    let saved = null;
    try {
      saved = JSON.parse(localStorage.getItem(OBS_SELECTION_KEY) || 'null');
    } catch {}
    app.obs.selected = normalizeSelection(Array.isArray(saved) ? saved : null);
    app.obs.active = app.obs.selected[app.obs.selected.length - 1];
    renderObsOptions();
    renderObsUrl();
    renderObsPreview();
  }

  function renderObsOptions() {
    const box = $('#obs-options');
    if (box.childElementCount !== app.obs.catalog.length) {
      box.replaceChildren(...app.obs.catalog.map((card) => el('label', { class: 'obs-option', 'data-obs-card': card.id }, [
        el('input', { type: 'checkbox', value: card.id, 'data-obs-card-option': '' }),
        el('span', { text: card.optionLabel }),
      ])));
    }
    for (const option of box.querySelectorAll('.obs-option')) {
      const selected = app.obs.selected.includes(option.getAttribute('data-obs-card'));
      option.classList.toggle('is-selected', selected);
      option.querySelector('input').checked = selected;
    }
  }

  // 自动、时钟与其他类型互斥；至少保留一项，全部取消时回到自动；勾选哪种就预览哪种。
  function toggleObsCard(id, checked) {
    if (checked) {
      app.obs.selected = obsStandalone(id) ? [id] : obsOrdered([...app.obs.selected.filter((item) => !obsStandalone(item)), id]);
      app.obs.active = id;
    } else {
      app.obs.selected = normalizeSelection(app.obs.selected.filter((item) => item !== id));
      if (!app.obs.selected.includes(app.obs.active)) app.obs.active = app.obs.selected[0];
    }
    app.obs.copiedUrl = null;
    saveSelection();
    renderObsOptions();
    renderObsUrl();
    renderObsPreview();
  }
  $('#obs-options').addEventListener('change', (event) => {
    const input = event.target.closest('input[type="checkbox"]');
    if (input) toggleObsCard(input.value, input.checked);
  });

  function selectedObsRoute() {
    const standalone = app.obs.selected.find(obsStandalone);
    if (standalone) return obsCardOf(standalone).route;
    const keys = app.obs.selected
      .flatMap((id) => Array.from(new URL(obsCardOf(id).route, 'http://localhost').searchParams.keys()));
    return `/overlay?${keys.join('&')}`;
  }
  // 建议尺寸与卡片目录中的实际尺寸一致。
  const obsSourceSize = (card) => [card.width, card.height];
  function renderObsUrl() {
    const box = $('#obs-url');
    const card = obsCardOf(app.obs.active) || app.obs.catalog[0];
    if (!card) {
      box.replaceChildren();
      return;
    }
    const route = selectedObsRoute();
    const url = app.serviceOrigin ? new URL(route, app.serviceOrigin).href : route;
    const copy = el('button', { type: 'button', class: 'button-primary', 'data-copy-overlay-url': '', text: app.obs.copiedUrl === url ? '已复制' : '复制' });
    copy.disabled = !app.serviceOrigin;
    copy.addEventListener('click', async () => {
      if (await copyText(url, copy, '复制 OBS 地址失败')) app.obs.copiedUrl = url;
    });
    box.replaceChildren(el('code', { text: url }), copy);
    const [width, height] = obsSourceSize(card);
    $('#obs-size-hint').textContent = `在浏览器来源属性里把宽高设为 ${width} × ${height}，与卡片实际尺寸一致。`;
  }
  async function copyText(text, button, failure) {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const input = document.createElement('textarea');
        input.value = text;
        input.style.position = 'fixed';
        input.style.opacity = '0';
        document.body.appendChild(input);
        input.focus();
        input.select();
        document.execCommand('copy');
        input.remove();
      }
      button.textContent = '已复制';
      return true;
    } catch (error) {
      button.textContent = '复制失败';
      setMessage(`${failure}：${String(error)}`, true);
      return false;
    }
  }

  // 卡片背景透明度：生成一行给 OBS “自定义 CSS” 用的样式，预览同步显示效果。
  const obsBackground = (transparency) => `rgba(${OBS_CARD_RGB}, ${(100 - transparency) / 100})`;
  const obsBackgroundCss = (transparency) => `:root {\n  --obs-card-background: ${obsBackground(transparency)};\n}`;
  function loadObsTransparency() {
    let saved = 0;
    try {
      saved = Number(localStorage.getItem(OBS_TRANSPARENCY_KEY) ?? 0);
    } catch {}
    app.obs.transparency = Number.isFinite(saved) ? Math.min(100, Math.max(0, Math.round(saved))) : 0;
  }
  function renderObsTransparency() {
    const value = app.obs.transparency;
    const css = obsBackgroundCss(value);
    $('#obs-transparency').value = String(value);
    $('#obs-transparency-value').textContent = `${value}%`;
    $('#obs-preview').style.setProperty('--obs-card-background', obsBackground(value));
    const copy = el('button', { type: 'button', class: 'button-primary', 'data-copy-overlay-css': '', text: app.obs.copiedCss === css ? '已复制' : '复制' });
    copy.addEventListener('click', async () => {
      if (await copyText(css, copy, '复制 CSS 失败')) app.obs.copiedCss = css;
    });
    $('#obs-css').replaceChildren(el('code', { text: css }), copy);
  }
  $('#obs-transparency').addEventListener('input', (event) => {
    app.obs.transparency = Number(event.target.value);
    renderObsTransparency();
  });
  $('#obs-transparency').addEventListener('change', () => {
    try {
      localStorage.setItem(OBS_TRANSPARENCY_KEY, String(app.obs.transparency));
    } catch {}
  });

  function obsLatestTimes(states) {
    const time = (value) => {
      const parsed = Date.parse(value ?? '');
      return Number.isFinite(parsed) ? parsed : null;
    };
    return {
      'bankara-match': time(states?.bankaraMatch?.selected?.played_time),
      'x-match': time(states?.xMatch?.selected?.played_time),
      'salmon-run': time(states?.overlay?.salmon_run?.played_time),
    };
  }
  async function refreshOverlayStates() {
    const accountId = app.activeAccountId;
    if (!accountId) {
      app.obs.states = null;
      dashboard.setSalmonRun(null);
      renderObsPreview();
      return;
    }
    const requestId = ++overlayRequest;
    try {
      const [overlay, xMatch, bankaraMatch] = await Promise.all([
        invoke('get_overlay_state', { accountId }),
        invoke('get_overlay_state', { accountId, matchType: 'xmatch' }),
        invoke('get_overlay_state', { accountId, matchType: 'bankaramatch' }),
      ]);
      if (requestId !== overlayRequest) return;
      const states = { overlay, xMatch, bankaraMatch };
      const latest = obsLatestTimes(states);
      // 多选时，所选类型里有新记录就把预览切过去，和 OBS 里的切换一致。
      if (app.obs.latest && !app.obs.selected.includes('overlay')) {
        const fresh = app.obs.selected
          .filter((id) => latest[id] != null && (app.obs.latest[id] == null || latest[id] > app.obs.latest[id]))
          .sort((a, b) => latest[b] - latest[a]);
        if (fresh.length) app.obs.active = fresh[0];
      }
      app.obs.latest = latest;
      app.obs.states = states;
      dashboard.setSalmonRun(overlay?.salmon_run);
    } catch {
      // 保留上一次成功的预览，避免偶发失败让卡片闪烁。
      return;
    }
    renderObsPreview();
  }

  function renderObsPreview() {
    const card = obsCardOf(app.obs.active);
    const box = $('#obs-preview');
    if (!card) {
      box.replaceChildren();
      return;
    }
    $('#obs-preview-title').textContent = `预览：${card.optionLabel}`;
    const [width, height] = obsSourceSize(card);
    $('#obs-preview-size').textContent = `建议 ${width} × ${height}`;
    let preview = box.querySelector('.overlay-preview');
    if (!preview || preview.dataset.card !== card.id) {
      preview = el('div', { class: 'overlay-preview', 'data-card': card.id }, el('div', { class: 'overlay-preview__viewport' }, el('div', { class: 'overlay-preview__scale' })));
      box.replaceChildren(preview);
    }
    const scale = preview.querySelector('.overlay-preview__scale');
    const states = app.obs.states;
    let hasData = false;
    if (card.id === 'clock') {
      // 时钟不需要账号数据，直接显示本机时间。
      hasData = Boolean(cards?.updateClock(scale));
    } else if (card.id === 'defeat-counts') {
      // 击倒数量用数据面板读到的「鲑鱼跑记录」，图片走本地图片缓存；换账号后等新账号的数据读完再显示。
      const factsReady = app.facts.status === 'ready' && app.facts.accountId === app.activeAccountId;
      hasData = Boolean(factsReady && cards?.updateDefeatCounts(scale, dashboard.coopRecord(), records.imageSrc));
    } else if (cards && states) {
      if (card.id === 'overlay') {
        const type = cards.updateAuto(scale, states.overlay);
        hasData = type === 'salmon-run' ? Boolean(states.overlay?.salmon_run) : Boolean(states.overlay?.selected);
      } else if (card.id === 'salmon-run') {
        hasData = cards.updateSalmonRun(scale, states.overlay);
      } else if (card.id === 'x-match') {
        hasData = cards.update(scale, states.xMatch);
      } else if (card.id === 'bankara-match') {
        hasData = cards.update(scale, states.bankaraMatch);
      }
    }
    let empty = box.querySelector('.obs-empty');
    if (!hasData) {
      if (!empty) {
        empty = el('div', { class: 'obs-empty' });
        box.append(empty);
      }
      empty.textContent = app.activeAccountId ? OBS_EMPTY[card.id] : '添加账号并同步后显示卡片预览。';
      preview.hidden = true;
      return;
    }
    empty?.remove();
    preview.hidden = false;
    fitObsPreview();
    if (card.id === 'clock') scheduleObsClock();
  }
  // 时钟预览每到整秒刷新；离开这一页或换成其他卡片后停下。
  let obsClockTimer = 0;
  function scheduleObsClock() {
    clearTimeout(obsClockTimer);
    obsClockTimer = setTimeout(() => {
      const scale = $('#obs-preview .overlay-preview[data-card="clock"] .overlay-preview__scale');
      if (app.view !== 'obs' || !scale) return;
      cards.updateClock(scale);
      scheduleObsClock();
    }, 1000 - (Date.now() % 1000));
  }
  function fitObsPreview() {
    const preview = $('#obs-preview .overlay-preview');
    const viewport = preview?.querySelector('.overlay-preview__viewport');
    const scale = preview?.querySelector('.overlay-preview__scale');
    const card = preview?.querySelector('.overlay-card, .salmon-run-card, .clock-card, .defeat-count-card');
    if (!viewport || !scale || !card) return;
    const ratio = scale.offsetWidth > 0 && preview.clientWidth > 0 ? Math.min(1, preview.clientWidth / scale.offsetWidth) : 1;
    scale.style.transform = `scale(${ratio})`;
    viewport.style.height = `${Math.ceil(scale.offsetHeight * ratio)}px`;
  }

  // 标题栏贴住窗口顶部时（top: -1px，露出的部分少了一像素）加上底色和分隔线。
  const topbar = $('.topbar');
  new IntersectionObserver(([entry]) => topbar.classList.toggle('is-stuck', entry.intersectionRatio < 1), { threshold: [1] }).observe(topbar);

  // 窗口宽度变化时重新绘制图表
  let lastWidth = 0;
  let resizeFrame = 0;
  new ResizeObserver((entries) => {
    const width = Math.round(entries[0].contentRect.width);
    if (width === lastWidth) return;
    lastWidth = width;
    cancelAnimationFrame(resizeFrame);
    resizeFrame = requestAnimationFrame(() => keepScroll(() => {
      if (app.view === 'overview') renderOverview();
      else if (analysis.VIEWS.includes(app.view)) renderAnalysis(app.view);
      else if (app.view === 'obs') fitObsPreview();
    }));
  }).observe($('.main'));

  let lastView = null;
  try {
    lastView = localStorage.getItem(VIEW_KEY);
  } catch {}
  showView(location.hash.slice(1) || lastView || 'overview');
  renderSyncStatus(null);
  loadObsTransparency();
  renderObsTransparency();
  // 字体加载完再画一次当前页面，场地名等标签的宽度按实际字体计算。
  document.fonts?.ready.then(() => {
    renderDataView();
  });
  loadObsCatalog().catch((error) => setMessage(String(error), true));
  refreshAccounts().catch((error) => {
    app.accountsLoaded = true;
    setMessage(String(error), true);
    renderDataView();
  });
  refreshSyncStatus();
  refreshServiceStatus();
  window.setInterval(refreshSyncStatus, 1000);
  window.setInterval(refreshServiceStatus, 2000);
  window.setInterval(() => {
    if (app.view === 'obs') refreshOverlayStates();
  }, 5000);
})();
