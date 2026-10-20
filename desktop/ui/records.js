(() => {
  'use strict';

  // 对战记录页：当前时间范围和模式下的全部记录，可以筛选、排序、翻页；点一行在右侧抽屉里看详情。
  // 选“打工”模式时列出打工记录。列表用已加载的面板数据，详情按需读取归档的原始 JSON。
  const dashboard = window.Splat3Dashboard;
  const data = window.Splat3DashboardData;
  const detail = window.Splat3RecordDetail;
  const { el, fmtDateTime, count, pct, fixed, RULE_LABEL, ruleLabel, modeLabelOf, modeColor, powerNode, emptyState, clickableRow } = dashboard.ui;
  const invoke = (...args) => window.__TAURI__.core.invoke(...args);

  const pad2 = (value) => String(value).padStart(2, '0');
  const fmtClock = (seconds) => (seconds == null ? '—' : `${Math.floor(seconds / 60)}:${pad2(Math.round(seconds % 60))}`);
  const fmtFullTime = (t) => {
    const date = new Date(t);
    return `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()} ${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
  };
  const dangerPercent = (rate) => (rate == null ? null : Math.round(rate < 10 ? rate * 100 : rate));
  const chip = (ok, text) => el('span', { class: `chip ${ok ? 'win' : 'lose'}`, text });
  const battleChip = (battle) => (battle.win ? chip(true, '胜') : battle.lose ? chip(false, '负') : el('span', { class: 'chip other', text: '—' }));
  const shiftChip = (shift) => chip(shift.cleared, shift.cleared ? '通关' : `第 ${shift.failedWave ?? '?'} 波`);
  const swatch = (color) => el('span', { class: 'swatch', style: `background:${color}` });

  // 图片走桌面端的本地缓存：splat3img://localhost/<路径>?src=<原链接>（Windows 上是 http://splat3img.localhost/）。
  // 同一张图只下载一次，原链接过期后也能显示；加载失败时不显示图片，只留文字。
  const IMAGE_BASE = window.__SPLAT3_IMAGE_BASE__ || (/Windows|Android/i.test(navigator.userAgent) ? 'http://splat3img.localhost/' : 'splat3img://localhost/');
  function imageSrc(url) {
    const key = detail.imageKey(url);
    return key ? `${IMAGE_BASE}${key}?src=${encodeURIComponent(url)}` : null;
  }
  function img(url, className, label) {
    const src = imageSrc(url);
    if (!src) return null;
    const node = el('img', { class: className, src, alt: label || '', loading: 'lazy', decoding: 'async' });
    if (label) node.title = label;
    node.addEventListener('error', () => node.remove());
    return node;
  }
  // 鳞片统一写成「鳞片图 x数量」，和 OBS 鲑鱼跑卡片一致。
  const scaleCounts = (scales) => [['gold', '金'], ['silver', '银'], ['bronze', '铜']].map(([key, label]) => el('span', { class: 'scale-count', title: `${label}鳞片` }, [
    el('img', { src: `images/scale-${key}.png`, alt: `${label}鳞片`, width: '18', height: '18' }),
    document.createTextNode(`x${scales[key] ?? '—'}`),
  ]));
  const iconWithName = (url, name, className = 'weapon-icon') => el('span', { class: 'icon-name' }, [img(url, className, name), document.createTextNode(name ?? '—')]);
  // 团队颜色上的文字：按亮度选深色或白色。
  function inkOn(color) {
    const match = /rgba?\((\d+), (\d+), (\d+)/.exec(color || '');
    if (!match) return null;
    const [r, g, b] = match.slice(1, 4).map(Number);
    return 0.299 * r + 0.587 * g + 0.114 * b > 150 ? '#171716' : '#ffffff';
  }

  const state = {
    filters: {
      battle: { rule: 'all', stage: 'all', weapon: 'all', result: 'all' },
      coop: { stage: 'all', result: 'all' },
    },
    sort: { key: 't', dir: 'desc' },
    page: 0,
    salmon: false,
    rows: [],
    selected: null,
  };
  const kindOf = (salmon) => (salmon ? 'coop' : 'battle');

  // 筛选
  function choicesFrom(rows, pick, label) {
    const counts = new Map();
    for (const row of rows) {
      const value = pick(row);
      if (value != null) counts.set(value, (counts.get(value) || 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || String(a[0]).localeCompare(String(b[0]), 'zh-CN'))
      .map(([value]) => [value, label ? label(value) : value]);
  }
  function renderFilters(source, filters) {
    const bar = document.getElementById('log-filters');
    const focused = document.activeElement?.closest?.('#log-filters') ? document.activeElement.getAttribute('data-filter') : null;
    bar.textContent = '';
    const field = (label, key, choices, allLabel) => {
      const values = [['all', allLabel], ...choices];
      // 换了时间范围后，已选的值可能不在列表里了，仍然保留它，结果为空时一眼能看出原因。
      if (filters[key] !== 'all' && !values.some(([value]) => value === filters[key])) values.push([filters[key], filters[key]]);
      const select = el('select', { 'data-filter': key });
      for (const [value, text] of values) {
        const option = el('option', { value, text });
        option.selected = filters[key] === value;
        select.append(option);
      }
      select.addEventListener('change', () => {
        filters[key] = select.value;
        state.page = 0;
        render();
      });
      bar.append(el('label', {}, [document.createTextNode(label), select]));
    };
    if (source.salmon) {
      field('场地', 'stage', choicesFrom(source.rows, (row) => row.stage), '全部场地');
      field('结果', 'result', [['win', '通关'], ['lose', '失败']], '全部结果');
    } else {
      field('规则', 'rule', choicesFrom(source.rows, (row) => row.rule, (rule) => RULE_LABEL[rule] || rule), '全部规则');
      field('场地', 'stage', choicesFrom(source.rows, (row) => row.stage), '全部场地');
      field('武器', 'weapon', choicesFrom(source.rows, (row) => row.weapon), '全部武器');
      field('结果', 'result', [['win', '胜'], ['lose', '负']], '全部结果');
    }
    if (Object.values(filters).some((value) => value !== 'all')) {
      const reset = el('button', { type: 'button', class: 'ghost', text: '清除筛选' });
      reset.addEventListener('click', () => {
        for (const key of Object.keys(filters)) filters[key] = 'all';
        state.page = 0;
        render();
      });
      bar.append(reset);
    }
    if (focused) bar.querySelector(`[data-filter="${focused}"]`)?.focus();
  }

  function renderSummary() {
    const stat = (value, label) => el('span', {}, [el('strong', { class: 'font-number', text: value }), document.createTextNode(` ${label}`)]);
    const box = document.getElementById('log-summary');
    if (state.salmon) {
      const cleared = state.rows.filter((shift) => shift.cleared).length;
      box.replaceChildren(
        stat(`${state.rows.length} 场`, '打工'),
        stat(pct(state.rows.length ? cleared / state.rows.length : null, 0), '通关率'),
        stat(fixed(data.average(state.rows, (shift) => shift.goldenEggs)), '场均金鲑鱼卵'),
      );
      return;
    }
    const totals = data.summarize(state.rows);
    box.replaceChildren(stat(`${state.rows.length} 场`, '对战'), stat(pct(totals.wr, 0), '胜率'), stat(fixed(totals.kd, 2), 'K/D'));
  }

  // 表格
  const BATTLE_COLUMNS = [
    { label: '时间', sort: 't', number: true },
    { label: '模式' },
    { label: '规则 · 场地' },
    { label: '武器' },
    { label: '结果' },
    { label: '击倒 / 协助 / 死亡', sort: 'kills', num: true },
    { label: '特殊', num: true },
    { label: '涂地', sort: 'paint', num: true },
    { label: '用时', sort: 'duration', num: true },
    { label: '战力', num: true },
  ];
  const SHIFT_COLUMNS = [
    { label: '时间', sort: 't', number: true },
    { label: '场地' },
    { label: '结果' },
    { label: '潮位' },
    { label: '金鲑鱼卵', sort: 'goldenEggs', num: true },
    { label: '救援 / 被救', num: true },
    { label: '头目鲑鱼' },
    { label: '评级', num: true },
    { label: '用时（估算）', sort: 'duration', num: true },
  ];
  const battleCells = (battle) => [
    fmtDateTime(battle.t),
    el('span', {}, [swatch(modeColor(battle.mode)), document.createTextNode(modeLabelOf(battle))]),
    `${ruleLabel(battle)} · ${battle.stage}`,
    iconWithName(battle.weaponImage, battle.weapon),
    battleChip(battle),
    `${count(battle.kills)} / ${count(battle.assists)} / ${count(battle.deaths)}`,
    count(battle.specials),
    count(battle.paint),
    fmtClock(battle.duration),
    powerNode(battle),
  ];
  const shiftCells = (shift) => [
    fmtDateTime(shift.t),
    shift.stage,
    shiftChip(shift),
    shift.waves.map((wave) => wave.water).join(' · ') || '—',
    count(shift.goldenEggs),
    `${count(shift.rescues)} / ${count(shift.rescued)}`,
    shift.kingAppeared ? (shift.kingDefeated ? '已击倒' : '未击倒') : '—',
    shift.gradeName ? `${shift.gradeName} ${shift.gradePoint ?? ''}`.trim() : '—',
    fmtClock(shift.duration),
  ];
  function cellsInto(tr, columns, cells) {
    cells.forEach((cell, index) => {
      const column = columns[index];
      const td = el('td', { class: [column.num ? 'num' : '', column.number ? 'font-number' : '', cell === '—' ? 'placeholder' : ''].filter(Boolean).join(' ') });
      if (cell instanceof Node) td.append(cell);
      else td.textContent = cell ?? '—';
      tr.append(td);
    });
  }
  function headerCell(column) {
    const th = el('th', { class: column.num ? 'num' : '', scope: 'col' });
    if (!column.sort) {
      th.textContent = column.label;
      return th;
    }
    const active = state.sort.key === column.sort;
    th.setAttribute('aria-sort', active ? (state.sort.dir === 'asc' ? 'ascending' : 'descending') : 'none');
    const button = el('button', { type: 'button', class: 'sort-button', 'aria-pressed': String(active), text: active ? `${column.label} ${state.sort.dir === 'asc' ? '↑' : '↓'}` : column.label });
    button.addEventListener('click', () => {
      state.sort = active ? { key: column.sort, dir: state.sort.dir === 'asc' ? 'desc' : 'asc' } : { key: column.sort, dir: 'desc' };
      state.page = 0;
      render();
      document.querySelector(`#log-table th .sort-button[aria-pressed="true"]`)?.focus();
    });
    th.append(button);
    return th;
  }
  function renderList() {
    const box = document.getElementById('log-table');
    const pager = document.getElementById('log-pager');
    box.textContent = '';
    if (!state.rows.length) {
      emptyState(box, state.salmon ? '没有符合条件的打工' : '没有符合条件的对战', '换一个时间范围或筛选条件看看。');
      pager.textContent = '';
      return;
    }
    const { page, pages, first, items } = data.pageOf(state.rows, state.page);
    state.page = page;
    const columns = state.salmon ? SHIFT_COLUMNS : BATTLE_COLUMNS;
    const table = el('table', { class: 'data' });
    table.append(el('thead', {}, el('tr', {}, columns.map(headerCell))));
    const body = el('tbody');
    for (const item of items) {
      const tr = el('tr', { 'data-record': item.id });
      if (item.id === state.selected) tr.classList.add('is-selected');
      clickableRow(tr, () => open(item.id, tr));
      cellsInto(tr, columns, state.salmon ? shiftCells(item) : battleCells(item));
      body.append(tr);
    }
    table.append(body);
    box.append(el('div', { class: 'scroll-x' }, table));

    const previous = el('button', { type: 'button', class: 'ghost', text: '上一页' });
    const next = el('button', { type: 'button', class: 'ghost', text: '下一页' });
    previous.disabled = page === 0;
    next.disabled = page >= pages - 1;
    previous.addEventListener('click', () => { state.page = page - 1; renderList(); });
    next.addEventListener('click', () => { state.page = page + 1; renderList(); });
    pager.replaceChildren(
      el('span', { class: 'font-number', text: `第 ${first + 1}-${first + items.length} 条，共 ${state.rows.length} 条` }),
      el('span', { class: 'log-pager-buttons' }, [previous, el('span', { class: 'font-number', text: `${page + 1} / ${pages}` }), next]),
    );
  }

  function render() {
    const source = dashboard.logSource();
    state.salmon = source.salmon;
    document.getElementById('page-sub').textContent = source.salmon ? '所有打工，点一行查看详情。' : '所有对局，点一行查看详情。';
    const filters = state.filters[kindOf(source.salmon)];
    const sortable = source.salmon ? ['t', 'goldenEggs', 'duration'] : ['t', 'kills', 'paint', 'duration'];
    if (!sortable.includes(state.sort.key)) state.sort = { key: 't', dir: 'desc' };
    renderFilters(source, filters);
    state.rows = data.sortRecords(data.filterRecords(source.rows, filters, source.salmon), state.sort);
    renderSummary();
    renderList();
    // 抽屉开着时只更新上一条 / 下一条，不打断正在看的内容。
    if (!drawer.hidden && !overviewList) drawer.querySelector('.drawer-nav')?.replaceWith(drawerNav(selectedIndex()));
  }

  // 详情抽屉
  const drawer = document.getElementById('record-drawer');
  const backdrop = document.getElementById('drawer-backdrop');
  const rawCache = new Map();
  const RAW_CACHE_LIMIT = 50;
  let returnFocus = null;
  // 上一条 / 下一条走的列表：从对战记录页打开时是表格里筛选、排序后的记录；从总览打开时是同一时间
  // 范围和模式下的全部记录，从新到旧，对战记录页的筛选、翻页和选中都不动。
  let overviewList = null;
  const drawerRows = () => overviewList?.rows ?? state.rows;
  const selectedIndex = () => drawerRows().findIndex((row) => row.id === state.selected);

  const section = (title, content) => el('section', { class: 'drawer-section' }, [el('h3', { text: title }), ...[].concat(content)]);
  function stats(items) {
    return el('div', { class: 'drawer-stats' }, items.map(([label, value]) => el('div', { class: 'mini-stat' }, [
      el('span', { text: label }),
      el('strong', { class: value === '—' ? 'placeholder' : '' }, value instanceof Node ? value : document.createTextNode(value)),
    ])));
  }
  function simpleTable(columns, rows, rowClass) {
    const table = el('table', { class: 'data' });
    table.append(el('thead', {}, el('tr', {}, columns.map((column) => el('th', { class: column.num ? 'num' : '', scope: 'col', text: column.label })))));
    const body = el('tbody');
    rows.forEach((row, index) => {
      const tr = el('tr', { class: rowClass ? rowClass(index) : '' });
      cellsInto(tr, columns, row);
      body.append(tr);
    });
    table.append(body);
    return el('div', { class: 'scroll-x' }, table);
  }
  function scoreBar(battle) {
    const score = battle.score;
    if (!score) return el('p', { class: 'muted', text: '—' });
    const values = [score.mine, ...score.theirs];
    const format = (value) => (value == null ? '—' : score.kind === 'paint' ? `${value.toFixed(1)}%` : String(value));
    const total = values.reduce((sum, value) => sum + (value || 0), 0) || 1;
    const bar = el('div', { class: 'score-bar', role: 'img', 'aria-label': `我方 ${format(score.mine)}，对方 ${score.theirs.map(format).join(' / ')}` });
    values.forEach((value, index) => {
      const share = Math.min(90, Math.max(10, ((value || 0) / total) * 100));
      // 有队伍颜色时用这场的墨水颜色。
      const color = battle.teams[index]?.color;
      const style = `flex: ${share.toFixed(1)} 1 0${color ? `; background: ${color}; color: ${inkOn(color)}` : ''}`;
      bar.append(el('span', { class: index === 0 ? 'mine' : 'theirs', style, text: format(value) }));
    });
    // 提前打满计数的比赛，游戏里显示“完胜！”；输的一方记作完败。
    const caption = score.kind === 'paint' ? '涂地比例' : battle.knockout === 'WIN' ? '完胜' : battle.knockout === 'LOSE' ? '完败' : '计数';
    return el('div', {}, [bar, el('div', { class: 'score-labels' }, [el('span', { text: '我方' }), el('span', { text: caption }), el('span', { text: '对方' })])]);
  }
  // 游戏里的名牌：背景图、玩家名字和徽章。
  // 名字里有假名时标成日文，换用日文字体显示。
  const KANA = /[\u3040-\u30ff\u31f0-\u31ff\uff66-\uff9f]/;
  function nameplate(name, plate, me) {
    const label = `${name ?? '—'}${me ? '（我）' : ''}`;
    const background = plate ? imageSrc(plate.background) : null;
    const node = el('span', { class: `nameplate${background ? '' : ' is-plain'}`, title: label }, [
      el('span', KANA.test(name ?? '') ? { class: 'nameplate-name', lang: 'ja', text: label } : { class: 'nameplate-name', text: label }),
      plate?.badges?.length ? el('span', { class: 'nameplate-badges' }, plate.badges.map((badge) => img(badge, 'badge-icon', '徽章'))) : null,
    ]);
    if (background) node.style.backgroundImage = `url("${background}")`;
    if (background && plate.textColor) node.style.color = plate.textColor;
    return node;
  }
  function weaponSet(player) {
    return el('span', { class: 'weapon-set' }, [
      img(player.weaponImage, 'weapon-icon', player.weapon),
      el('span', { text: player.weapon ?? '—' }),
      player.sub ? img(player.sub.image, 'kit-icon', player.sub.name) : null,
      player.special ? img(player.special.image, 'kit-icon', player.special.name) : null,
    ]);
  }
  function teamBlock(team, index, total) {
    const label = team.mine ? '我方' : total > 2 ? `对方 ${index}` : '对方';
    return el('div', { class: 'team' }, [
      el('div', { class: 'team-head' }, [el('span', { class: 'team-dot', style: `background: ${team.color || `var(${team.mine ? '--win' : '--lose'})`}`, 'aria-hidden': 'true' }), document.createTextNode(label)]),
      simpleTable(
        [{ label: '玩家' }, { label: '武器' }, { label: '击倒/协助/死亡', num: true }, { label: '特殊', num: true }, { label: '涂地', num: true }],
        team.players.map((player) => [
          nameplate(player.name, player.nameplate, player.me),
          weaponSet(player),
          `${count(player.kills)} / ${count(player.assists)} / ${count(player.deaths)}`,
          count(player.specials),
          count(player.paint),
        ]),
        (rowIndex) => (team.players[rowIndex].me ? 'is-me' : ''),
      ),
    ]);
  }
  // 我这场的装备：装备图、名称和品牌，主能力大图标，副能力小图标（悬停看名称）。
  function gearCard(gear) {
    return el('div', { class: 'gear-card' }, [
      img(gear.image, 'gear-image', gear.name),
      el('div', { class: 'gear-text' }, [el('strong', { text: gear.name ?? '—' }), el('small', { text: [gear.slot, gear.brand].filter(Boolean).join(' · ') })]),
      el('div', { class: 'gear-abilities' }, [
        gear.primary ? img(gear.primary.image, 'ability-main', gear.primary.name) : null,
        ...gear.additional.map((ability) => img(ability.image, 'ability-sub', ability.name)),
      ]),
    ]);
  }
  function rawSection(raw) {
    const pre = el('pre');
    const details = el('details', { class: 'raw-json' }, [el('summary', { text: '查看原始 JSON' }), pre]);
    // 原始 JSON 较大，展开时才生成文本。
    details.addEventListener('toggle', () => {
      if (details.open && !pre.textContent) pre.textContent = JSON.stringify(raw, null, 2);
    });
    return section('原始数据', details);
  }
  function battleSections(item, battle, raw) {
    const me = battle.teams[0]?.players.find((player) => player.me);
    return [
      section('比分', scoreBar(battle)),
      section('我的数据', stats([
        ['武器', me ? weaponSet({ ...me, weapon: me.weapon ?? item.weapon }) : item.weapon],
        ['击倒 / 协助 / 死亡', `${count(item.kills)} / ${count(item.assists)} / ${count(item.deaths)}`],
        ['特殊武器', item.specials == null ? '—' : `${item.specials} 次`],
        ['涂地', item.paint == null ? '—' : `${count(item.paint)} 点`],
        ['战力', powerNode(item)],
      ])),
      battle.gear.length ? section('装备', el('div', { class: 'gear-list' }, battle.gear.map(gearCard))) : null,
      section('队伍', battle.teams.map((team, index) => teamBlock(team, index, battle.teams.length))),
      battle.awards.length ? section('奖牌', el('div', { class: 'award-list' }, battle.awards.map((award) => el('span', { class: 'award', text: award })))) : null,
      rawSection(raw),
    ];
  }
  const weaponIcons = (weapons) => el('span', { class: 'weapon-icons' }, weapons.length ? weapons.map((weapon) => img(weapon.image, 'weapon-icon', weapon.name) || el('span', { class: 'muted', text: weapon.name ?? '—' })) : [document.createTextNode('—')]);
  function shiftSections(item, shift, raw) {
    const scale = shift.scale && [shift.scale.gold, shift.scale.silver, shift.scale.bronze].some((value) => value != null)
      ? el('span', { class: 'scale-inline' }, scaleCounts(shift.scale))
      : '—';
    const waveRows = shift.waves.map((wave) => [
      wave.number == null ? '—' : `第 ${wave.number} 波`,
      wave.water ?? '—',
      wave.event ?? '—',
      wave.specials.length ? el('span', { class: 'weapon-icons' }, wave.specials.map((special) => img(special.image, 'kit-icon', special.name))) : '—',
      count(wave.quota),
      count(wave.delivered),
      chip(wave.cleared, wave.cleared ? '通过' : '失败'),
    ]);
    if (shift.boss) {
      waveRows.push(['EX', '—', iconWithName(shift.boss.image, shift.boss.name ?? '头目鲑鱼', 'enemy-icon'), '—', '—', '—', chip(shift.boss.defeated, shift.boss.defeated ? '击倒' : '未击倒')]);
    }
    const golden = (member) => (member.goldenEggs == null ? '—' : member.goldenAssists ? `${member.goldenEggs} (+${member.goldenAssists})` : String(member.goldenEggs));
    return [
      section('我的数据', stats([
        ['金鲑鱼卵', count(shift.mine.goldenEggs ?? item.goldenEggs)],
        ['红鲑鱼卵', count(shift.mine.powerEggs ?? item.powerEggs)],
        ['救援 / 被救', `${count(shift.mine.rescues ?? item.rescues)} / ${count(shift.mine.rescued ?? item.rescued)}`],
        ['巨大鲑鱼击倒', count(shift.mine.defeated)],
        ['评级', shift.grade ? `${shift.grade} ${shift.gradePoint ?? ''}`.trim() : item.gradeName ? `${item.gradeName} ${item.gradePoint ?? ''}`.trim() : '—'],
        ['鳞片', scale],
      ])),
      shift.supplied.length ? section('本期武器', el('div', { class: 'weapon-row' }, shift.supplied.map((weapon) => iconWithName(weapon.image, weapon.name)))) : null,
      shift.members.length ? section('队员', simpleTable(
        [{ label: '队员' }, { label: '各波武器' }, { label: '特殊' }, { label: '金鲑鱼卵', num: true }, { label: '红鲑鱼卵', num: true }, { label: '救援/被救', num: true }, { label: '巨大鲑鱼', num: true }],
        shift.members.map((member) => [
          nameplate(member.name, member.nameplate, member.me),
          weaponIcons(member.weapons),
          member.special ? img(member.special.image, 'kit-icon', member.special.name) || member.special.name : '—',
          golden(member),
          count(member.powerEggs),
          `${count(member.rescues)} / ${count(member.rescued)}`,
          count(member.defeated),
        ]),
        (rowIndex) => (shift.members[rowIndex].me ? 'is-me' : ''),
      )) : null,
      section('波次', waveRows.length
        ? simpleTable([{ label: '波次' }, { label: '潮位' }, { label: '特殊波次' }, { label: '特殊武器' }, { label: '配额', num: true }, { label: '交付', num: true }, { label: '结果' }], waveRows)
        : el('p', { class: 'muted', text: '—' })),
      shift.enemies.length ? section('巨大鲑鱼', simpleTable(
        [{ label: '鲑鱼' }, { label: '我击倒', num: true }, { label: '全队击倒', num: true }, { label: '出现', num: true }],
        shift.enemies.map((enemy) => [iconWithName(enemy.image, enemy.name, 'enemy-icon'), count(enemy.defeated), count(enemy.teamDefeated), count(enemy.appeared)]),
      )) : null,
      rawSection(raw),
    ];
  }
  function drawerHead(item, kind) {
    const closeButton = el('button', { type: 'button', class: 'ghost', 'data-drawer-close': '', text: '关闭' });
    closeButton.addEventListener('click', close);
    let kicker;
    let title;
    let sub;
    if (kind === 'coop') {
      const danger = dangerPercent(item.danger);
      kicker = [shiftChip(item), swatch('var(--mode-salmon)'), document.createTextNode('鲑鱼跑')];
      title = item.stage;
      sub = [fmtFullTime(item.t), danger == null ? null : `危险度 ${danger}%`, `用时约 ${fmtClock(item.duration)}`].filter(Boolean).join(' · ');
    } else {
      kicker = [battleChip(item), swatch(modeColor(item.mode)), document.createTextNode(modeLabelOf(item))];
      title = `${ruleLabel(item)} · ${item.stage}`;
      sub = [fmtFullTime(item.t), `用时 ${fmtClock(item.duration)}`, item.knockout ? (item.win ? '完胜' : '完败') : null].filter(Boolean).join(' · ');
    }
    return el('div', { class: 'drawer-head' }, [
      el('div', {}, [el('div', { class: 'drawer-kicker' }, kicker), el('h2', { class: 'drawer-title', id: 'drawer-title', text: title }), el('p', { class: 'drawer-sub', text: sub })]),
      closeButton,
    ]);
  }
  function drawerNav(index) {
    const previous = el('button', { type: 'button', class: 'ghost', 'data-nav': 'previous', text: '上一条' });
    const next = el('button', { type: 'button', class: 'ghost', 'data-nav': 'next', text: '下一条' });
    previous.disabled = index <= 0;
    next.disabled = index < 0 || index >= drawerRows().length - 1;
    previous.addEventListener('click', () => select(index - 1, 'previous'));
    next.addEventListener('click', () => select(index + 1, 'next'));
    return el('div', { class: 'drawer-nav' }, [previous, el('span', { class: 'muted font-number', text: index < 0 ? '' : `${index + 1} / ${drawerRows().length}` }), next]);
  }
  function fillDrawer(banner, body, item, kind, raw) {
    body.textContent = '';
    if (raw == null) {
      body.append(el('p', { class: 'muted', text: '归档里没有找到这条记录的原始数据。' }));
      return;
    }
    const parsed = kind === 'coop' ? detail.shiftDetail(raw) : detail.battleDetail(raw);
    banner.replaceChildren(...[img(parsed.stageImage, 'drawer-banner', item.stage)].filter(Boolean));
    for (const node of kind === 'coop' ? shiftSections(item, parsed, raw) : battleSections(item, parsed, raw)) if (node) body.append(node);
  }
  function renderDrawer() {
    const index = selectedIndex();
    const item = drawerRows()[index];
    if (!item) {
      close();
      return;
    }
    const kind = kindOf(overviewList?.salmon ?? state.salmon);
    const banner = el('div', { class: 'drawer-banner-slot' });
    const body = el('div', { class: 'drawer-body' });
    drawer.replaceChildren(banner, drawerHead(item, kind), body, drawerNav(index));
    const key = `${kind}:${item.id}`;
    if (rawCache.has(key)) {
      fillDrawer(banner, body, item, kind, rawCache.get(key));
      return;
    }
    body.append(el('p', { class: 'muted', text: '正在读取详情…' }));
    invoke('get_history_record', { recordType: kind, recordId: item.id })
      .then((raw) => {
        rawCache.set(key, raw ?? null);
        if (rawCache.size > RAW_CACHE_LIMIT) rawCache.delete(rawCache.keys().next().value);
        if (state.selected === item.id && body.isConnected) fillDrawer(banner, body, item, kind, raw ?? null);
      })
      .catch((error) => {
        if (state.selected === item.id && body.isConnected) body.replaceChildren(el('p', { class: 'drawer-error', text: `读取详情失败：${String(error)}` }));
      });
  }
  function select(index, focusNav) {
    const item = drawerRows()[index];
    if (!item) return;
    state.selected = item.id;
    if (!overviewList) {
      state.page = Math.floor(index / data.LOG_PAGE_SIZE);
      renderList();
    }
    renderDrawer();
    const target = drawer.querySelector(`[data-nav="${focusNav}"]`);
    (target && !target.disabled ? target : drawer.querySelector('[data-drawer-close]'))?.focus();
  }
  function openDrawer(index, origin) {
    if (drawer.hidden) returnFocus = origin || document.activeElement;
    drawer.hidden = false;
    backdrop.hidden = false;
    select(index, null);
  }
  // 总览里点最近的一场：就在当前页面打开详情。
  function show(kind, id, origin) {
    const source = dashboard.logSource();
    if (source.salmon !== (kind === 'coop')) return;
    const rows = data.sortRecords(source.rows, { key: 't', dir: 'desc' });
    const index = rows.findIndex((row) => row.id === id);
    if (index < 0) return;
    overviewList = { rows, salmon: source.salmon };
    openDrawer(index, origin);
  }
  // 对战记录页里点一行。
  function open(id, origin) {
    overviewList = null;
    const index = state.rows.findIndex((row) => row.id === id);
    if (index >= 0) openDrawer(index, origin);
  }
  function close() {
    if (drawer.hidden) return;
    drawer.hidden = true;
    backdrop.hidden = true;
    drawer.replaceChildren();
    const row = state.selected ? document.querySelector(`#log-table tr[data-record="${CSS.escape(state.selected)}"]`) : null;
    state.selected = null;
    overviewList = null;
    row?.classList.remove('is-selected');
    const target = returnFocus?.isConnected ? returnFocus : row;
    returnFocus = null;
    target?.focus?.();
  }
  backdrop.addEventListener('click', close);
  document.addEventListener('keydown', (event) => {
    if (drawer.hidden) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== 'Tab') return;
    // 焦点留在抽屉里。
    const focusable = [...drawer.querySelectorAll('button, summary, [tabindex="0"]')].filter((node) => !node.disabled && node.offsetParent !== null);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !drawer.contains(document.activeElement))) {
      event.preventDefault();
      first.focus();
    }
  });

  window.Splat3Records = Object.freeze({ render, show, close, img, iconWithName, scaleCounts });
})();
