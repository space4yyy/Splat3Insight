(() => {
  'use strict';

  const NS = 'http://www.w3.org/2000/svg';
  const DAY = 86400000;
  const token = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const readColors = () => ({
    ink: token('--ink'), ink2: token('--ink-2'), ink3: token('--ink-3'),
    line: token('--line'), lineStrong: token('--line-strong'), panel: token('--panel'),
    accent: token('--accent'), brandBlue: token('--brand-blue'), winLine: token('--win-line'),
    meterWin: token('--meter-win'), meterLose: token('--meter-lose'),
    salmon: token('--mode-salmon'), context: token('--context'), seq0: token('--seq-0'),
    seq: ['--seq-1', '--seq-2', '--seq-3', '--seq-4', '--seq-5'].map(token),
    div: ['--div-neg-3', '--div-neg-2', '--div-neg-1', '--div-mid', '--div-pos-1', '--div-pos-2', '--div-pos-3'].map(token),
    salmonDiv: ['--salmon-div-neg-3', '--salmon-div-neg-2', '--salmon-div-neg-1', '--div-mid', '--salmon-div-pos-1', '--salmon-div-pos-2', '--salmon-div-pos-3'].map(token),
    modes: {
      x: token('--mode-x'), anarchy: token('--mode-anarchy'), event: token('--mode-event'),
      regular: token('--mode-regular'), private: token('--mode-private'), other: token('--context'),
    },
  });
  let COLOR = readColors();

  const MODES = [
    { key: 'x', label: 'X 比赛', short: 'X' },
    { key: 'anarchy', label: '蛮颓比赛', short: '真格' },
    { key: 'event', label: '活动比赛', short: '活动' },
    { key: 'regular', label: '一般比赛', short: '一般' },
    { key: 'private', label: '私人比赛', short: '私人' },
    { key: 'other', label: '祭典及其他', short: '其他', optional: true },
  ];
  const MODE = Object.fromEntries(MODES.map((mode) => [mode.key, mode]));
  const modeColor = (key) => COLOR.modes[key] || COLOR.context;
  const data = window.Splat3DashboardData;
  const { WATER, isNum, round1, seasonStart, summarize, clearRate, average } = data;
  const RANKED_RULES = ['AREA', 'LOFT', 'GOAL', 'CLAM'];
  const RULE_LABEL = { AREA: '真格区域', LOFT: '真格塔楼', GOAL: '真格鱼虎', CLAM: '真格蛤蜊', TURF_WAR: '占地对战' };
  const RULE_SHORT = { AREA: '区域', LOFT: '塔楼', GOAL: '鱼虎', CLAM: '蛤蜊' };
  const WEEKDAYS = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];
  const STAGE_ROWS = 12;

  let battles = [];
  let shifts = [];
  let xSettlements = [];
  let coopRecord = null;
  let xRules = new Set();
  let salmonRun = null;
  let filterListener = null;
  let openRecordListener = null;
  const state = { range: '30d', mode: 'all', coopRule: 'all', rule: 'AREA', heatMetric: 'count', tables: {} };
  // 右上角筛选的选择记在本机，重新打开应用时沿用。state 里是实际生效的值：对战、武器、场地页没有
  // 「打工」，打工模式也只能选记录里出现过的，这时先按全部模式显示，选择本身不变。
  // period 是时间范围第三格显示的自然周期：当前选着的，或者上次选的。
  const FILTERS_KEY = 'splat3-filters';
  const chosen = { mode: 'all', coopRule: 'all', period: 'season' };
  try {
    const saved = JSON.parse(localStorage.getItem(FILTERS_KEY) || 'null') || {};
    if (typeof saved.range === 'string') state.range = saved.range;
    if (typeof saved.mode === 'string') chosen.mode = state.mode = saved.mode;
    if (typeof saved.coopRule === 'string') chosen.coopRule = saved.coopRule;
    if (typeof saved.period === 'string') chosen.period = saved.period;
  } catch {}
  function saveFilters() {
    try {
      localStorage.setItem(FILTERS_KEY, JSON.stringify({ range: state.range, mode: chosen.mode, coopRule: chosen.coopRule, period: chosen.period }));
    } catch {}
  }
  // 打工模式（SplatNet 的 rule），名称取游戏里的说法。
  const COOP_RULES = [
    { key: 'REGULAR', label: '鲑鱼跑' },
    { key: 'BIG_RUN', label: '大型跑' },
    { key: 'TEAM_CONTEST', label: '团队打工竞赛' },
  ];
  const COOP_RULE_LABEL = Object.fromEntries(COOP_RULES.map((rule) => [rule.key, rule.label]));

  const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
  const ruleLabel = (battle) => RULE_LABEL[battle.rule] || battle.ruleName || '未知规则';

  function dayStartOf(t) { const date = new Date(t); date.setHours(0, 0, 0, 0); return date.getTime(); }
  function addDays(t, days) { const date = new Date(t); date.setDate(date.getDate() + days); return date.getTime(); }
  function weekStartOf(t) { const day = dayStartOf(t); return addDays(day, -((new Date(day).getDay() + 6) % 7)); }
  function monthStartOf(t) { const date = new Date(t); return new Date(date.getFullYear(), date.getMonth(), 1).getTime(); }
  function dataStart(nowMs) {
    const first = Math.min(battles[0]?.t ?? Infinity, shifts[0]?.t ?? Infinity);
    return Number.isFinite(first) ? dayStartOf(first) : addDays(dayStartOf(nowMs), -29);
  }
  function rangeBounds(key) {
    const now = Date.now();
    const today = dayStartOf(now);
    if (key === '7d') return [addDays(today, -6), now];
    if (key === '30d') return [addDays(today, -29), now];
    if (key === 'week') return [weekStartOf(now), now];
    if (key === 'month') return [monthStartOf(now), now];
    if (key === 'season') return [seasonStart(now), now];
    return [dataStart(now), now];
  }
  // 本周、本月、本赛季和上一周、月、赛季的同一段比：从开头起经过同样长的时间，上一段没这么长时到它结束为止。
  function previousBounds(key) {
    if (key === 'all') return null;
    const [start, end] = rangeBounds(key);
    if (key === '7d') return [addDays(start, -7), start];
    if (key === '30d') return [addDays(start, -30), start];
    const before = { week: weekStartOf, month: monthStartOf, season: seasonStart }[key](start - 1);
    return [before, Math.min(before + (end - start), start)];
  }
  const inMode = (battle) => state.mode === 'all' || battle.mode === state.mode;
  const battlesIn = (start, end) => battles.filter((battle) => battle.t >= start && battle.t <= end && inMode(battle));
  const shiftsIn = (start, end) => shifts.filter((shift) => shift.t >= start && shift.t <= end);

  const pct = (value, digits = 1) => (value == null ? '—' : `${(value * 100).toFixed(digits)}%`);
  const fixed = (value, digits = 1) => (value == null ? '—' : value.toFixed(digits));
  const count = (value) => (value == null ? '—' : Math.round(value).toLocaleString('zh-CN'));
  const pad2 = (value) => String(value).padStart(2, '0');
  const fmtDate = (t) => { const d = new Date(t); return `${d.getMonth() + 1}月${d.getDate()}日`; };
  const fmtDateTime = (t) => { const d = new Date(t); return `${d.getMonth() + 1}/${d.getDate()} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`; };
  function fmtSince(t) {
    if (t == null) return '—';
    const days = Math.floor((dayStartOf(Date.now()) - dayStartOf(t)) / DAY);
    if (days <= 0) return '今天';
    if (days === 1) return '昨天';
    return `${days} 天前`;
  }
  function fmtDuration(seconds) {
    if (seconds == null) return '—';
    const minutes = Math.round(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    if (!hours) return `${rest} 分钟`;
    return rest ? `${hours} 小时 ${rest} 分` : `${hours} 小时`;
  }
  const fmtShortDate = (t) => { const d = new Date(t); return `${d.getMonth() + 1}/${d.getDate()}`; };
  const fmtDayLabel = (t) => `${fmtDate(t)} ${WEEKDAYS[(new Date(t).getDay() + 6) % 7]}`;
  function signed(value, digits = 1, unit = '') {
    if (value == null || Number.isNaN(value)) return '';
    const rounded = Number(value.toFixed(digits));
    if (rounded === 0) return `±0${digits ? `.${'0'.repeat(digits)}` : ''}${unit}`;
    return `${rounded > 0 ? '+' : '-'}${Math.abs(rounded).toFixed(digits)}${unit}`;
  }
  function deltaNode(value, digits, unit) {
    const span = document.createElement('span');
    if (value == null || Number.isNaN(value)) { span.className = 'muted'; span.textContent = '无对比'; return span; }
    const rounded = Number(value.toFixed(digits));
    if (rounded === 0) {
      span.className = 'delta-flat';
      span.textContent = signed(value, digits, unit);
      return span;
    }
    const up = rounded > 0;
    span.className = up ? 'delta-up' : 'delta-down';
    span.textContent = `${up ? '▲' : '▼'} ${signed(value, digits, unit)}`;
    return span;
  }

  function el(tag, props = {}, children = []) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (key === 'text') node.textContent = value;
      else if (key === 'class') node.className = value;
      else if (key === 'style') node.style.cssText = value;
      else node.setAttribute(key, value);
    }
    for (const child of [].concat(children)) if (child != null) node.append(child);
    return node;
  }
  function svgEl(tag, attrs = {}, parent) {
    const node = document.createElementNS(NS, tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    if (parent) parent.appendChild(node);
    return node;
  }
  function svgText(parent, x, y, text, attrs = {}) {
    const node = svgEl('text', { x, y, fill: COLOR.ink3, 'font-size': 12, ...attrs }, parent);
    node.textContent = text;
    return node;
  }
  function niceScale(min, max, steps = 4) {
    if (min === max) { min -= 1; max += 1; }
    const raw = (max - min) / steps;
    const magnitude = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 2.5, 5, 10].map((factor) => factor * magnitude).find((candidate) => candidate >= raw);
    const low = Math.floor(min / step) * step;
    const high = Math.ceil(max / step) * step;
    const ticks = [];
    for (let value = low; value <= high + step / 2; value += step) ticks.push(Math.round(value * 1000) / 1000);
    return { min: low, max: high, ticks };
  }
  function columnPath(left, right, top, bottom, rounded) {
    if (bottom - top <= 0) return '';
    const radius = rounded ? Math.min(4, (bottom - top) / 2, (right - left) / 2) : 0;
    if (!radius) return `M${left},${bottom} L${left},${top} L${right},${top} L${right},${bottom} Z`;
    return `M${left},${bottom} L${left},${top + radius} Q${left},${top} ${left + radius},${top} L${right - radius},${top} Q${right},${top} ${right},${top + radius} L${right},${bottom} Z`;
  }
  function emptyState(container, title, body) {
    container.textContent = '';
    container.append(el('div', { class: 'empty-state' }, [el('strong', { text: title }), el('span', { text: body })]));
  }
  // onRow：点一行（或在行上按回车 / 空格）时调用，用来打开这条记录的详情。
  function clickableRow(tr, open) {
    tr.classList.add('is-clickable');
    tr.tabIndex = 0;
    tr.addEventListener('click', open);
    tr.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      event.preventDefault();
      open();
    });
  }
  function renderTable(container, columns, rows, { onRow } = {}) {
    container.textContent = '';
    const table = el('table', { class: 'data' });
    const head = el('tr');
    for (const column of columns) head.append(el('th', { class: column.num ? 'num' : '', text: column.label }));
    table.append(el('thead', {}, head));
    const body = el('tbody');
    for (const [rowIndex, row] of rows.entries()) {
      const tr = el('tr');
      if (onRow) clickableRow(tr, () => onRow(rowIndex));
      row.forEach((cell, index) => {
        const column = columns[index];
        const td = el('td', { class: [column.num ? 'num' : '', column.number ? 'font-number' : '', column.wrap ? 'wrap' : '', cell === '—' ? 'placeholder' : ''].filter(Boolean).join(' ') });
        if (cell instanceof Node) td.append(cell); else td.textContent = cell;
        tr.append(td);
      });
      body.append(tr);
    }
    table.append(body);
    container.append(container.classList.contains('table-wrap') ? table : el('div', { class: 'table-wrap' }, table));
  }
  function textOn(hex) {
    const n = parseInt(hex.slice(1), 16);
    const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((channel) => {
      const c = channel / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.22 ? '#0b0b0b' : '#ffffff';
  }
  function divergingColor(winRate, colors = COLOR.div) {
    const delta = winRate - 0.5;
    const distance = Math.abs(delta);
    const step = distance < 0.03 ? 0 : distance < 0.08 ? 1 : distance < 0.15 ? 2 : 3;
    if (step === 0) return colors[3];
    return delta > 0 ? colors[3 + step] : colors[3 - step];
  }

  const tip = document.getElementById('tip');
  function showTip(point, title, rows) {
    tip.textContent = '';
    if (title) tip.append(el('div', { class: 'tip-title', text: title }));
    for (const row of rows) {
      const line = el('div', { class: 'tip-row' });
      if (row.color) line.append(el('span', { class: row.box ? 'tip-box' : 'tip-key', style: `background:${row.color}` }));
      line.append(el('strong', { text: row.value }));
      if (row.label) line.append(el('span', { text: row.label }));
      tip.append(line);
    }
    tip.hidden = false;
    const rect = tip.getBoundingClientRect();
    let left = point.clientX + 14;
    let top = point.clientY + 14;
    if (left + rect.width > window.innerWidth - 8) left = point.clientX - rect.width - 14;
    if (top + rect.height > window.innerHeight - 8) top = point.clientY - rect.height - 14;
    tip.style.left = `${Math.max(8, left)}px`;
    tip.style.top = `${Math.max(8, top)}px`;
  }
  function hideTip() { tip.hidden = true; }
  function bindTip(node, content) {
    node.addEventListener('pointermove', (event) => { const [title, rows] = content(); showTip(event, title, rows); });
    node.addEventListener('pointerleave', hideTip);
    node.addEventListener('focus', () => {
      const rect = node.getBoundingClientRect();
      const [title, rows] = content();
      showTip({ clientX: rect.right, clientY: rect.bottom }, title, rows);
    });
    node.addEventListener('blur', hideTip);
  }

  function xValueAt(rule, t) {
    let value = null;
    for (const settlement of xSettlements) {
      if (settlement.rule !== rule) continue;
      if (settlement.t <= t) value = settlement.value; else break;
    }
    return value;
  }
  function bucketEdges(start, end, buckets = 12) {
    return Array.from({ length: buckets }, (_, index) => [start + ((end - start) * index) / buckets, start + ((end - start) * (index + 1)) / buckets]);
  }
  function sparkline(values, width) {
    const w = Math.max(80, Math.round(width));
    const h = 32;
    const pad = 7;
    const svg = svgEl('svg', { width: w, height: h, viewBox: `0 0 ${w} ${h}`, 'aria-hidden': 'true' });
    const known = values.map((value, index) => [index, value]).filter(([, value]) => value != null);
    if (known.length < 2) return svg;
    const min = Math.min(...known.map(([, value]) => value));
    const max = Math.max(...known.map(([, value]) => value));
    const x = (index) => pad + (index / (values.length - 1)) * (w - pad * 2);
    const y = (value) => (max === min ? h / 2 : h - pad - ((value - min) / (max - min)) * (h - pad * 2));
    svgEl('polyline', { points: known.map(([index, value]) => `${x(index).toFixed(1)},${y(value).toFixed(1)}`).join(' '), fill: 'none', stroke: COLOR.context, 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round', 'vector-effect': 'non-scaling-stroke' }, svg);
    const [lastIndex, lastValue] = known[known.length - 1];
    svgEl('circle', { cx: x(lastIndex), cy: y(lastValue), r: 4.5, fill: COLOR.brandBlue, stroke: COLOR.panel, 'stroke-width': 2 }, svg);
    return svg;
  }
  const relativeChange = (now, before) => (before ? ((now - before) / before) * 100 : null);

  function renderKpis() {
    const container = document.getElementById('kpis');
    container.textContent = '';
    const [start, end] = rangeBounds(state.range);
    const previous = previousBounds(state.range);
    const buckets = bucketEdges(start, end);
    const versus = { all: '', week: '较上周同期', month: '较上月同期', season: '较上赛季同期' }[state.range] ?? '较上期';
    const tiles = [];
    if (state.mode === 'salmon') {
      const shiftStats = (from, to) => {
        const list = shiftsIn(from, to);
        const cleared = list.filter((shift) => shift.cleared).length;
        return {
          n: list.length,
          cleared,
          seconds: list.reduce((sum, shift) => sum + shift.duration, 0),
          clear: clearRate(list),
          eggs: average(list, (shift) => shift.goldenEggs),
        };
      };
      const now = shiftStats(start, end);
      const prev = previous ? shiftStats(previous[0], previous[1]) : null;
      const trend = buckets.map(([from, to]) => shiftStats(from, to));
      tiles.push({ label: '游戏时长', value: (now.seconds / 3600).toFixed(1), valueSmall: '小时', meta: [versus && deltaNode(prev ? relativeChange(now.seconds, prev.seconds) : null, 0, '%'), el('span', { text: '仅打工，按波次估算' })], spark: trend.map((s) => s.seconds / 3600) });
      tiles.push({ label: '打工场次', value: count(now.n), meta: [versus && deltaNode(prev ? relativeChange(now.n, prev.n) : null, 0, '%'), el('span', { text: `通关 ${now.cleared} 场` })], spark: trend.map((s) => s.n) });
      tiles.push({ label: '通关率', value: pct(now.clear), meta: [versus && deltaNode(prev && prev.clear != null && now.clear != null ? (now.clear - prev.clear) * 100 : null, 1, ' pt'), el('span', { text: versus || '整个归档' })], spark: trend.map((s) => s.clear) });
      tiles.push({ label: '场均金鲑鱼卵', value: fixed(now.eggs), meta: [versus && deltaNode(prev && prev.eggs != null && now.eggs != null ? now.eggs - prev.eggs : null, 1, ''), el('span', { text: '个人交付' })], spark: trend.map((s) => s.eggs) });
    } else {
      const current = summarize(battlesIn(start, end));
      const before = previous ? summarize(battlesIn(previous[0], previous[1])) : null;
      const bucketStats = buckets.map(([from, to]) => summarize(battlesIn(from, to)));
      const includeSalmon = state.mode === 'all';
      const battleSecondsIn = (from, to) => battlesIn(from, to).reduce((sum, battle) => sum + (battle.duration || 0), 0);
      const playSecondsIn = (from, to) => battleSecondsIn(from, to) + (includeSalmon ? shiftsIn(from, to).reduce((sum, shift) => sum + shift.duration, 0) : 0);
      const timeNow = playSecondsIn(start, end);
      const timeBefore = previous ? playSecondsIn(previous[0], previous[1]) : null;
      const battleHours = battleSecondsIn(start, end) / 3600;
      tiles.push({ label: '游戏时长', value: (timeNow / 3600).toFixed(1), valueSmall: '小时', meta: [versus && deltaNode(timeBefore != null ? relativeChange(timeNow, timeBefore) : null, 0, '%'), el('span', { text: includeSalmon ? `对战 ${battleHours.toFixed(1)} · 打工 ${(timeNow / 3600 - battleHours).toFixed(1)}` : `仅${MODE[state.mode].label}` })], spark: buckets.map(([from, to]) => playSecondsIn(from, to) / 3600) });
      tiles.push({ label: '对战场次', value: count(current.n), meta: [versus && deltaNode(before ? relativeChange(current.n, before.n) : null, 0, '%'), el('span', { text: `${current.wins} 胜 ${current.losses} 负` })], spark: bucketStats.map((s) => s.n) });
      tiles.push({ label: '胜率', value: pct(current.wr), meta: [versus && deltaNode(before && before.wr != null && current.wr != null ? (current.wr - before.wr) * 100 : null, 1, ' pt'), el('span', { text: versus || '整个归档' })], spark: bucketStats.map((s) => s.wr) });
      tiles.push({ label: 'K/D', value: fixed(current.kd, 2), meta: [versus && deltaNode(before && before.kd != null && current.kd != null ? current.kd - before.kd : null, 2, ''), el('span', { text: `场均 ${fixed(current.avgK)} 击倒 / ${fixed(current.avgD)} 死亡` })], spark: bucketStats.map((s) => s.kd) });
    }
    const placed = tiles.map((tile) => {
      const value = el('div', { class: tile.value === '—' ? 'kpi-value placeholder' : 'kpi-value', text: tile.value });
      if (tile.valueSmall) value.append(el('small', { text: tile.valueSmall }));
      const node = el('div', { class: 'kpi' }, [el('div', { class: 'kpi-label', text: tile.label }), value, el('div', { class: 'kpi-meta' }, tile.meta.filter(Boolean))]);
      container.append(node);
      return [node, tile];
    });
    for (const [node, tile] of placed) {
      if (tile.spark.length) node.append(sparkline(tile.spark, node.clientWidth - 32));
    }
  }

  function renderXPower() {
    const tabs = document.getElementById('x-tabs');
    const box = document.getElementById('x-chart');
    const [start, end] = rangeBounds(state.range);
    tabs.textContent = '';
    for (const rule of RANKED_RULES) {
      const current = xValueAt(rule, end);
      const before = xValueAt(rule, start);
      const selected = state.rule === rule;
      const button = el('button', { type: 'button', class: 'rule-tab', 'aria-pressed': String(selected) }, [
        el('span', { class: 'rule-key', style: `background:${selected ? COLOR.modes.x : COLOR.context}` }),
        el('span', { class: 'rule-name', text: RULE_LABEL[rule] }),
        el('span', { class: 'rule-value', text: current == null ? '未定级' : current.toFixed(1) }),
      ]);
      if (current != null && before != null && round1(current - before) !== 0) button.append(deltaNode(current - before, 1, ''));
      button.addEventListener('click', () => { state.rule = rule; renderXPower(); });
      tabs.append(button);
    }

    if (!(state.mode === 'all' || state.mode === 'x')) {
      emptyState(box, '当前筛选不包含 X 比赛', 'X 战力只在 X 比赛中计算。切换到“全部模式”或“X”后查看。');
      return;
    }
    if (!xRules.has(state.rule)) {
      emptyState(box, `${RULE_LABEL[state.rule]}还没有定级`, '这个规则还没有 X 战力记录，战力保持为空，也不会借用其他规则的分数。');
      return;
    }
    const series = [...xRules].map((rule) => ({
      rule,
      points: xSettlements.filter((item) => item.rule === rule && item.t >= start && item.t <= end),
      anchor: xValueAt(rule, start),
    }));
    if (state.tables.x) {
      const rows = xSettlements.filter((item) => item.t >= start && item.t <= end).slice().reverse().map((item) => {
        const previous = xSettlements.filter((other) => other.rule === item.rule && other.t < item.t).slice(-1)[0];
        return [fmtDateTime(item.t), RULE_LABEL[item.rule] || item.rule, item.value.toFixed(1), previous ? signed(item.value - previous.value) : '—'];
      });
      if (!rows.length) { emptyState(box, '这段时间 X 战力没有变化', '完成一次测定后会出现新的分数点。'); return; }
      renderTable(box, [{ label: '时间', number: true }, { label: '规则' }, { label: 'X 战力', num: true }, { label: '变化', num: true }], rows);
      return;
    }
    const selectedSeries = series.find((item) => item.rule === state.rule);
    if (!selectedSeries.points.length && selectedSeries.anchor == null) {
      emptyState(box, '这段时间没有 X 战力记录', '完成一次测定后会出现新的分数点。');
      return;
    }
    const values = series.flatMap((item) => item.points.map((point) => point.value).concat(item.anchor == null ? [] : [item.anchor]));
    box.textContent = '';
    const width = Math.max(300, Math.floor(box.clientWidth));
    const height = 236;
    const margin = { top: 14, right: 64, bottom: 28, left: 46 };
    const scale = niceScale(Math.min(...values) - 10, Math.max(...values) + 10, 4);
    const x = (t) => margin.left + ((t - start) / Math.max(1, end - start)) * (width - margin.left - margin.right);
    const y = (value) => margin.top + (1 - (value - scale.min) / (scale.max - scale.min)) * (height - margin.top - margin.bottom);
    const currentValue = xValueAt(state.rule, end);
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `${RULE_LABEL[state.rule]} X 战力走势，当前 ${currentValue == null ? '未定级' : currentValue.toFixed(1)}` }, box);
    for (const tick of scale.ticks) {
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: COLOR.line, 'stroke-width': 1 }, svg);
      svgText(svg, margin.left - 8, y(tick) + 4, String(tick), { 'text-anchor': 'end', class: 'font-number' });
    }
    for (let index = 0; index <= 4; index += 1) {
      const t = start + ((end - start) * index) / 4;
      svgText(svg, x(t), height - 8, fmtDate(t), { 'text-anchor': index === 0 ? 'start' : index === 4 ? 'end' : 'middle', class: 'font-number' });
    }
    const drawOrder = series.filter((item) => item.rule !== state.rule).concat(selectedSeries);
    for (const item of drawOrder) {
      const coords = [];
      if (item.anchor != null) coords.push([x(start), y(item.anchor)]);
      for (const point of item.points) coords.push([x(point.t), y(point.value)]);
      if (coords.length && coords[coords.length - 1][0] < x(end)) coords.push([x(end), coords[coords.length - 1][1]]);
      const selected = item.rule === state.rule;
      const path = coords.map(([px, py], index) => `${index ? 'L' : 'M'}${px.toFixed(1)},${py.toFixed(1)}`).join(' ');
      svgEl('path', { d: path, fill: 'none', stroke: selected ? COLOR.modes.x : COLOR.context, 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round', opacity: selected ? 1 : 0.7 }, svg);
      if (selected) {
        for (const point of item.points) svgEl('circle', { cx: x(point.t), cy: y(point.value), r: 4, fill: COLOR.modes.x, stroke: COLOR.panel, 'stroke-width': 2 }, svg);
        const last = xValueAt(item.rule, end);
        if (last != null) svgText(svg, width - margin.right + 8, y(last) + 5, last.toFixed(1), { fill: COLOR.ink, 'font-size': 15, class: 'font-number' });
      }
    }
    const marks = series.flatMap((item) => item.points.map((point) => point.t)).sort((a, b) => a - b);
    const crosshair = svgEl('line', { y1: margin.top, y2: height - margin.bottom, stroke: COLOR.ink3, 'stroke-width': 1, visibility: 'hidden' }, svg);
    const overlay = svgEl('rect', { x: margin.left, y: margin.top, width: width - margin.left - margin.right, height: height - margin.top - margin.bottom, fill: 'transparent' }, svg);
    overlay.addEventListener('pointermove', (event) => {
      if (!marks.length) return;
      const bounds = svg.getBoundingClientRect();
      const t = start + ((event.clientX - bounds.left - margin.left) / (width - margin.left - margin.right)) * (end - start);
      const nearest = marks.reduce((best, mark) => (Math.abs(mark - t) < Math.abs(best - t) ? mark : best), marks[0]);
      crosshair.setAttribute('x1', x(nearest));
      crosshair.setAttribute('x2', x(nearest));
      crosshair.setAttribute('visibility', 'visible');
      showTip(event, fmtDateTime(nearest), RANKED_RULES.map((rule) => {
        const value = xValueAt(rule, nearest);
        return { value: value == null ? '未定级' : value.toFixed(1), label: RULE_LABEL[rule], color: rule === state.rule ? COLOR.modes.x : COLOR.context };
      }));
    });
    overlay.addEventListener('pointerleave', () => { crosshair.setAttribute('visibility', 'hidden'); hideTip(); });
  }

  function renderForm() {
    const strip = document.getElementById('form-strip');
    const summary = document.getElementById('form-summary');
    const box = document.getElementById('form-chart');
    const [start, end] = rangeBounds(state.range);
    const salmon = state.mode === 'salmon';
    const word = salmon ? { rate: '通关率', win: '通关', lose: '失败' } : { rate: '胜率', win: '胜', lose: '负' };
    document.getElementById('form-sub').textContent = salmon ? '最近 20 场打工结果与 20 场滚动通关率' : '最近 20 场结果与 20 场滚动胜率';
    const list = salmon
      ? shiftsIn(start, end).filter((shift) => !shift.disconnected).map((shift) => ({ t: shift.t, win: shift.cleared, shift }))
      : battlesIn(start, end).filter((battle) => battle.win || battle.lose);
    const lastTwenty = list.slice(-20);
    strip.textContent = '';
    for (let index = 0; index < 20; index += 1) {
      const item = lastTwenty[index - (20 - lastTwenty.length)];
      if (!item) { strip.append(el('div', { class: 'form-cell none', 'aria-hidden': 'true' })); continue; }
      const label = salmon ? (item.win ? '通' : `W${item.shift.failedWave ?? '?'}`) : (item.win ? '胜' : '负');
      const cell = el('div', { class: `form-cell ${item.win ? 'win' : 'lose'}`, tabindex: '0', 'aria-label': `${fmtDateTime(item.t)} ${salmon ? shiftResult(item.shift) : item.win ? word.win : word.lose}`, text: label });
      bindTip(cell, () => [fmtDateTime(item.t), salmon ? [
        { value: shiftResult(item.shift), label: item.shift.stage, color: COLOR.salmon, box: true },
        { value: `${count(item.shift.goldenEggs)} 个`, label: '金鲑鱼卵' },
        { value: `${count(item.shift.rescues)} / ${count(item.shift.rescued)}`, label: '救援 / 被救' },
      ] : [
        { value: item.win ? '胜利' : '失败', label: `${MODE[item.mode].label} · ${ruleLabel(item)}`, color: modeColor(item.mode), box: true },
        { value: `${count(item.kills)} / ${count(item.assists)} / ${count(item.deaths)}`, label: `击倒 / 协助 / 死亡 · ${item.weapon}` },
        { value: item.stage, label: '场地' },
      ]]);
      strip.append(cell);
    }
    let streak = 0;
    for (let index = list.length - 1; index >= 0 && list[index].win === list[list.length - 1].win; index -= 1) streak += 1;
    const lastWin = list.length ? list[list.length - 1].win : null;
    const streakText = !list.length ? '—' : salmon ? `${streak} 场${lastWin ? '连续通关' : '连续失败'}` : `${streak} ${lastWin ? '连胜' : '连败'}`;
    const recentRate = lastTwenty.length ? lastTwenty.filter((item) => item.win).length / lastTwenty.length : null;
    summary.textContent = '';
    summary.append(
      el('span', {}, [el('strong', { text: streakText }), document.createTextNode(' 当前')]),
      el('span', {}, [el('strong', { text: pct(recentRate, 0) }), document.createTextNode(` 最近 ${lastTwenty.length || 20} 场${word.rate}`)]),
    );

    const points = [];
    let wins = 0;
    for (let index = 0; index < list.length; index += 1) {
      wins += list[index].win ? 1 : 0;
      if (index >= 20) wins -= list[index - 20].win ? 1 : 0;
      if (index >= 19) points.push({ index: index + 1, t: list[index].t, wr: wins / 20, win: list[index].win });
    }
    if (points.length < 2) { emptyState(box, `至少 20 场后显示滚动${word.rate}`, '扩大时间范围，或切换到场次更多的模式。'); return; }
    if (state.tables.form) {
      renderTable(box, [{ label: '场次', number: true }, { label: '时间', number: true }, { label: '本场' }, { label: `20 场${word.rate}`, num: true }], points.slice().reverse().map((point) => [`第 ${point.index} 场`, fmtDateTime(point.t), point.win ? word.win : word.lose, pct(point.wr, 0)]));
      return;
    }
    box.textContent = '';
    const width = Math.max(240, Math.floor(box.clientWidth));
    const height = 150;
    const margin = { top: 10, right: 40, bottom: 22, left: 34 };
    const first = points[0].index;
    const last = points[points.length - 1].index;
    const x = (index) => margin.left + ((index - first) / Math.max(1, last - first)) * (width - margin.left - margin.right);
    const y = (value) => margin.top + (1 - value) * (height - margin.top - margin.bottom);
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `20 场滚动${word.rate}，最新 ${pct(points[points.length - 1].wr, 0)}` }, box);
    for (const tick of [0, 0.25, 0.5, 0.75, 1]) {
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: tick === 0.5 ? COLOR.lineStrong : COLOR.line, 'stroke-width': 1 }, svg);
      svgText(svg, margin.left - 6, y(tick) + 4, `${tick * 100}%`, { 'text-anchor': 'end', class: 'font-number' });
    }
    svgText(svg, margin.left, height - 6, `第 ${first} 场`, { 'text-anchor': 'start', class: 'font-number' });
    svgText(svg, width - margin.right, height - 6, `第 ${last} 场`, { 'text-anchor': 'end', class: 'font-number' });
    const line = points.map((point, index) => `${index ? 'L' : 'M'}${x(point.index).toFixed(1)},${y(point.wr).toFixed(1)}`).join(' ');
    svgEl('path', { d: `${line} L${x(last).toFixed(1)},${y(0)} L${x(first).toFixed(1)},${y(0)} Z`, fill: COLOR.winLine, opacity: 0.1 }, svg);
    svgEl('path', { d: line, fill: 'none', stroke: COLOR.winLine, 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }, svg);
    const lastPoint = points[points.length - 1];
    svgEl('circle', { cx: x(lastPoint.index), cy: y(lastPoint.wr), r: 4, fill: COLOR.winLine, stroke: COLOR.panel, 'stroke-width': 2 }, svg);
    svgText(svg, x(lastPoint.index) + 8, y(lastPoint.wr) + 5, pct(lastPoint.wr, 0), { fill: COLOR.ink, 'font-size': 14, class: 'font-number' });
    const crosshair = svgEl('line', { y1: margin.top, y2: height - margin.bottom, stroke: COLOR.ink3, 'stroke-width': 1, visibility: 'hidden' }, svg);
    const overlay = svgEl('rect', { x: margin.left, y: margin.top, width: width - margin.left - margin.right, height: height - margin.top - margin.bottom, fill: 'transparent' }, svg);
    overlay.addEventListener('pointermove', (event) => {
      const bounds = svg.getBoundingClientRect();
      const ratio = clamp((event.clientX - bounds.left - margin.left) / (width - margin.left - margin.right), 0, 1);
      const point = points[Math.round(ratio * (points.length - 1))];
      crosshair.setAttribute('x1', x(point.index));
      crosshair.setAttribute('x2', x(point.index));
      crosshair.setAttribute('visibility', 'visible');
      showTip(event, `第 ${point.index} 场 · ${fmtDateTime(point.t)}`, [
        { value: pct(point.wr, 0), label: `20 场滚动${word.rate}`, color: COLOR.winLine },
        { value: point.win ? word.win : word.lose, label: '本场结果' },
      ]);
    });
    overlay.addEventListener('pointerleave', () => { crosshair.setAttribute('visibility', 'hidden'); hideTip(); });
  }

  function renderPlaytime() {
    const statsBox = document.getElementById('time-stats');
    const legend = document.getElementById('time-legend');
    const box = document.getElementById('time-chart');
    const [start, end] = rangeBounds(state.range);
    const includeSalmon = state.mode === 'all' || state.mode === 'salmon';
    const rangeBattles = battlesIn(start, end);
    const series = [
      ...MODES
        .filter((mode) => (state.mode === 'all' ? !mode.optional || rangeBattles.some((battle) => battle.mode === mode.key) : mode.key === state.mode))
        .map((mode) => ({ key: mode.key, label: mode.label, color: modeColor(mode.key) })),
      ...(includeSalmon ? [{ key: 'salmon', label: '鲑鱼跑', color: COLOR.salmon, estimated: true, hatch: true }] : []),
    ];
    const swatchBackground = (item) => (item.hatch ? `repeating-linear-gradient(45deg, ${item.color} 0 3px, ${COLOR.panel} 3px 5px)` : item.color);
    document.getElementById('time-sub').textContent = state.mode === 'salmon'
      ? '只统计打工，时长按波次估算'
      : includeSalmon
        ? '对战按每场对局时长累加，鲑鱼跑按波次估算'
        : `只统计${MODE[state.mode].label}的对局时长`;
    const emptyEntry = () => ({
      seconds: Object.fromEntries(series.map((item) => [item.key, 0])),
      counts: Object.fromEntries(series.map((item) => [item.key, 0])),
    });
    const days = new Map();
    const add = (t, key, seconds) => {
      const day = dayStartOf(t);
      if (!days.has(day)) days.set(day, emptyEntry());
      const entry = days.get(day);
      if (!(key in entry.seconds)) return;
      entry.seconds[key] += seconds;
      entry.counts[key] += 1;
    };
    for (const battle of rangeBattles) add(battle.t, battle.mode, battle.duration || 0);
    if (includeSalmon) for (const shift of shiftsIn(start, end)) add(shift.t, 'salmon', shift.duration);
    const sumOf = (entry) => series.reduce((sum, item) => sum + entry.seconds[item.key], 0);
    const total = [...days.values()].reduce((sum, entry) => sum + sumOf(entry), 0);
    let longest = null;
    for (const [day, entry] of days) {
      const seconds = sumOf(entry);
      if (!longest || seconds > longest.seconds) longest = { day, seconds };
    }
    statsBox.textContent = '';
    // 默认窗口下四格要排在一行：日期放进标签，数值里的「小时」「分」「天」用小字。
    const stats = [
      ['总计', total ? fmtDuration(total) : '—'],
      ['游玩天数', `${days.size} 天`],
      ['每个游玩日', days.size ? fmtDuration(total / days.size) : '—'],
      [longest ? `最长一天 · ${fmtShortDate(longest.day)}` : '最长一天', longest ? fmtDuration(longest.seconds) : '—'],
    ];
    const withUnits = (text) => text.split(/(\d+)/).filter(Boolean).map((part) => (/^\d+$/.test(part) ? part : el('small', { text: part })));
    for (const [label, value] of stats) {
      statsBox.append(el('div', { class: 'mini-stat' }, [
        el('span', { text: label }),
        value === '—' ? el('strong', { class: 'placeholder', text: value }) : el('strong', {}, withUnits(value)),
      ]));
    }
    legend.textContent = '';
    if (series.length > 1) {
      for (const item of series) {
        legend.append(el('span', {}, [el('span', { class: 'swatch', style: `background:${swatchBackground(item)}` }), document.createTextNode(item.estimated ? `${item.label}（估算）` : item.label)]));
      }
    }
    if (!total) { emptyState(box, '这段时间没有游玩记录', '换一个时间范围看看。'); return; }

    const weekly = state.range === 'all';
    const buckets = [];
    for (let from = weekly ? weekStartOf(start) : dayStartOf(start); from <= end; from = addDays(from, weekly ? 7 : 1)) {
      buckets.push({ from, to: addDays(from, weekly ? 7 : 1), ...emptyEntry() });
    }
    const byStart = new Map(buckets.map((bucket) => [bucket.from, bucket]));
    for (const [day, entry] of days) {
      const bucket = byStart.get(weekly ? weekStartOf(day) : day);
      if (!bucket) continue;
      for (const item of series) {
        bucket.seconds[item.key] += entry.seconds[item.key];
        bucket.counts[item.key] += entry.counts[item.key];
      }
    }
    const bucketLabel = (bucket) => (weekly ? `${fmtDate(bucket.from)} 至 ${fmtDate(addDays(bucket.to, -1))}` : fmtDayLabel(bucket.from));
    if (state.tables.time) {
      const columns = [{ label: weekly ? '周' : '日期', number: true }, ...series.map((item) => ({ label: item.estimated ? `${item.label}（估算）` : item.label, num: true })), { label: '合计', num: true }];
      renderTable(box, columns, buckets.filter((bucket) => sumOf(bucket) > 0).reverse().map((bucket) => [
        bucketLabel(bucket),
        ...series.map((item) => (bucket.counts[item.key] ? `${fmtDuration(bucket.seconds[item.key])} · ${bucket.counts[item.key]} 场` : '—')),
        fmtDuration(sumOf(bucket)),
      ]));
      return;
    }
    box.textContent = '';
    const width = Math.max(280, Math.floor(box.clientWidth));
    const height = 210;
    const margin = { top: 24, right: 8, bottom: 24, left: 34 };
    const scale = niceScale(0, Math.max(...buckets.map((bucket) => sumOf(bucket) / 3600)), 4);
    const plotWidth = width - margin.left - margin.right;
    const band = plotWidth / buckets.length;
    const barWidth = Math.max(2, Math.min(24, band * 0.62));
    const y = (hours) => margin.top + (1 - hours / scale.max) * (height - margin.top - margin.bottom);
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `游戏时长，${weekly ? '按周' : '按天'}统计，合计 ${fmtDuration(total)}` }, box);
    const hatchId = `salmon-hatch-${Math.random().toString(36).slice(2)}`;
    if (series.some((item) => item.hatch)) {
      const pattern = svgEl('pattern', { id: hatchId, patternUnits: 'userSpaceOnUse', width: 5, height: 5, patternTransform: 'rotate(45)' }, svgEl('defs', {}, svg));
      svgEl('rect', { width: 5, height: 5, fill: COLOR.salmon }, pattern);
      svgEl('rect', { width: 2, height: 5, fill: COLOR.panel }, pattern);
    }
    svgText(svg, 0, 12, '小时');
    for (const tick of scale.ticks) {
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: tick === 0 ? COLOR.lineStrong : COLOR.line, 'stroke-width': 1 }, svg);
      svgText(svg, margin.left - 6, y(tick) + 4, String(tick), { 'text-anchor': 'end', class: 'font-number' });
    }
    const labelEvery = Math.max(1, Math.ceil(buckets.length / Math.max(2, Math.floor(plotWidth / 52))));
    buckets.forEach((bucket, index) => {
      const cx = margin.left + band * index + band / 2;
      if (index % labelEvery === 0) svgText(svg, cx, height - 6, fmtShortDate(bucket.from), { 'text-anchor': 'middle', class: 'font-number' });
      const bucketTotal = sumOf(bucket);
      if (bucketTotal <= 0) return;
      const left = cx - barWidth / 2;
      const right = cx + barWidth / 2;
      const pieces = [];
      let cumulative = 0;
      let previousTop = y(0);
      for (const item of series) {
        const seconds = bucket.seconds[item.key];
        if (seconds <= 0) continue;
        cumulative += seconds / 3600;
        const top = y(cumulative);
        const bottom = pieces.length ? previousTop - 2 : previousTop;
        if (bottom - top > 0.5) pieces.push({ item, top, bottom });
        previousTop = top;
      }
      pieces.forEach((piece, pieceIndex) => {
        svgEl('path', { d: columnPath(left, right, piece.top, piece.bottom, pieceIndex === pieces.length - 1), fill: piece.item.hatch ? `url(#${hatchId})` : piece.item.color }, svg);
      });
      const hit = svgEl('rect', { x: margin.left + band * index, y: margin.top, width: band, height: height - margin.top - margin.bottom, class: 'col-hit', tabindex: '0' }, svg);
      bindTip(hit, () => [bucketLabel(bucket), [
        { value: fmtDuration(bucketTotal), label: '合计' },
        ...series.filter((item) => bucket.counts[item.key]).map((item) => ({
          value: fmtDuration(bucket.seconds[item.key]),
          label: `${item.label} · ${bucket.counts[item.key]} 场${item.estimated ? '（估算）' : ''}`,
          color: swatchBackground(item),
          box: true,
        })),
      ]]);
    });
  }

  function renderModes() {
    const bar = document.getElementById('mode-bar');
    const tableBox = document.getElementById('mode-table');
    const [start, end] = rangeBounds(state.range);
    const list = battlesIn(start, end);
    const total = list.length;
    bar.textContent = '';
    if (!total) { emptyState(bar, '这段时间没有对战', '换一个时间范围看看。'); tableBox.textContent = ''; return; }
    const stats = MODES
      .map((mode) => ({ mode, ...summarize(list.filter((battle) => battle.mode === mode.key)) }))
      .filter((stat) => !stat.mode.optional || stat.n);
    const width = Math.max(240, Math.floor(bar.clientWidth));
    const height = 18;
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `模式占比：${stats.filter((s) => s.n).map((s) => `${s.mode.label} ${pct(s.n / total, 0)}`).join('，')}` }, bar);
    const clipId = `mode-clip-${Math.random().toString(36).slice(2)}`;
    const clip = svgEl('clipPath', { id: clipId }, svgEl('defs', {}, svg));
    svgEl('rect', { x: 0, y: 0, width, height, rx: 4 }, clip);
    const group = svgEl('g', { 'clip-path': `url(#${clipId})` }, svg);
    const visible = stats.filter((s) => s.n);
    const gaps = (visible.length - 1) * 2;
    let cursor = 0;
    for (const stat of visible) {
      const segment = ((width - gaps) * stat.n) / total;
      const rect = svgEl('rect', { x: cursor, y: 0, width: Math.max(1, segment), height, fill: modeColor(stat.mode.key), class: 'cell', tabindex: '0' }, group);
      bindTip(rect, () => [stat.mode.label, [
        { value: `${stat.n} 场`, label: `占比 ${pct(stat.n / total)}`, color: modeColor(stat.mode.key), box: true },
        { value: pct(stat.wr), label: '胜率' },
      ]]);
      cursor += segment + 2;
    }
    renderTable(tableBox, [{ label: '模式' }, { label: '场次', num: true }, { label: '占比', num: true }, { label: '胜率', num: true }, { label: 'K/D', num: true }], stats.map((stat) => [
      el('span', {}, [el('span', { class: 'swatch', style: `background:${modeColor(stat.mode.key)}` }), document.createTextNode(stat.mode.label)]),
      String(stat.n), stat.n ? pct(stat.n / total, 0) : '—', pct(stat.wr), fixed(stat.kd, 2),
    ]));
  }

  function renderHeat() {
    const box = document.getElementById('heat-chart');
    const legend = document.getElementById('heat-legend');
    const [start, end] = rangeBounds(state.range);
    const salmon = state.mode === 'salmon';
    const rateWord = salmon ? '通关率' : '胜率';
    document.querySelector('#heat-metric [data-metric="winrate"]').textContent = rateWord;
    const list = salmon
      ? shiftsIn(start, end).map((shift) => ({ t: shift.t, win: shift.cleared, decided: !shift.disconnected }))
      : battlesIn(start, end).map((battle) => ({ t: battle.t, win: battle.win, decided: battle.win || battle.lose }));
    const cells = Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => ({ n: 0, decided: 0, wins: 0 })));
    for (const item of list) {
      const date = new Date(item.t);
      const cell = cells[(date.getDay() + 6) % 7][date.getHours()];
      cell.n += 1;
      if (item.decided) cell.decided += 1;
      if (item.win) cell.wins += 1;
    }
    const max = Math.max(0, ...cells.flat().map((cell) => cell.n));
    const metric = state.heatMetric;
    const rateOf = (cell) => (cell.decided >= 3 ? cell.wins / cell.decided : null);
    const colorFor = (cell) => {
      if (metric === 'count') return cell.n ? COLOR.seq[Math.min(4, Math.ceil((cell.n / max) * 5) - 1)] : COLOR.seq0;
      const rate = rateOf(cell);
      return rate == null ? COLOR.seq0 : divergingColor(rate);
    };
    if (state.tables.heat) {
      const rows = [];
      cells.forEach((row, day) => row.forEach((cell, hour) => { if (cell.n) rows.push([WEEKDAYS[day], `${pad2(hour)}:00`, String(cell.n), rateOf(cell) == null ? '样本不足' : pct(rateOf(cell), 0)]); }));
      if (!rows.length) emptyState(box, salmon ? '这段时间没有打工' : '这段时间没有对战', '换一个时间范围看看。');
      else renderTable(box, [{ label: '星期' }, { label: '时段', number: true }, { label: '场次', num: true }, { label: rateWord, num: true }], rows);
    } else {
      box.textContent = '';
      const labelWidth = 34;
      const available = Math.max(0, box.clientWidth - labelWidth);
      const size = Math.max(14, Math.floor(available / 24));
      const width = labelWidth + size * 24;
      const height = 16 + size * 7;
      const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `按星期与小时的${metric === 'count' ? '场次' : rateWord}热力图` }, box);
      for (let hour = 0; hour < 24; hour += 3) svgText(svg, labelWidth + hour * size + 1, 11, `${hour}时`, { class: 'font-number' });
      cells.forEach((row, day) => {
        svgText(svg, 0, 16 + day * size + size / 2 + 4, WEEKDAYS[day]);
        row.forEach((cell, hour) => {
          const rect = svgEl('rect', { x: labelWidth + hour * size + 1, y: 16 + day * size + 1, width: size - 2, height: size - 2, rx: 3, fill: colorFor(cell), class: 'cell', tabindex: cell.n ? '0' : '-1' }, svg);
          bindTip(rect, () => [`${WEEKDAYS[day]} ${hour} 点`, [
            { value: `${cell.n} 场`, label: '场次', color: colorFor(cell), box: true },
            { value: rateOf(cell) != null ? pct(rateOf(cell), 0) : cell.n ? '样本不足' : '—', label: rateWord },
          ]]);
        });
      });
    }
    legend.textContent = '';
    if (metric === 'count') {
      legend.append(
        el('span', {}, [el('span', { class: 'legend-scale' }, [el('span', { style: `background:${COLOR.seq0}` })]), document.createTextNode(salmon ? ' 没有打工' : ' 没有对战')]),
        el('span', {}, [document.createTextNode('少 '), el('span', { class: 'legend-scale' }, COLOR.seq.map((color) => el('span', { style: `background:${color}` }))), document.createTextNode(` 多（最多 ${max} 场）`)]),
      );
    } else {
      legend.append(
        el('span', {}, [document.createTextNode('低于 50% '), el('span', { class: 'legend-scale' }, COLOR.div.map((color) => el('span', { style: `background:${color}` }))), document.createTextNode(' 高于 50%')]),
        el('span', {}, [el('span', { class: 'legend-scale' }, [el('span', { style: `background:${COLOR.seq0}` })]), document.createTextNode(' 少于 3 场')]),
      );
    }
  }

  function renderStageRule() {
    const box = document.getElementById('stage-chart');
    const legend = document.getElementById('stage-legend');
    const [start, end] = rangeBounds(state.range);
    const list = battlesIn(start, end).filter((battle) => RANKED_RULES.includes(battle.rule));
    const stageCounts = new Map();
    for (const battle of list) stageCounts.set(battle.stage, (stageCounts.get(battle.stage) || 0) + 1);
    const stages = [...stageCounts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'zh-CN')).map(([stage]) => stage);
    legend.textContent = '';
    legend.append(
      el('span', {}, [document.createTextNode('低于 50% '), el('span', { class: 'legend-scale' }, COLOR.div.map((color) => el('span', { style: `background:${color}` }))), document.createTextNode(' 高于 50%')]),
      el('span', {}, [el('span', { class: 'legend-scale' }, [el('span', { style: `background:${COLOR.seq0}` })]), document.createTextNode(' 少于 3 场')]),
    );
    if (stages.length > STAGE_ROWS && !state.tables.stage) legend.append(el('span', { text: `只列场次最多的 ${STAGE_ROWS} 张图` }));
    if (!list.length) { emptyState(box, '这段时间没有真格规则的对战', '占地对战不计入这个矩阵。'); return; }
    const cellOf = (stage, rule) => summarize(list.filter((battle) => battle.stage === stage && battle.rule === rule));
    const decided = (cell) => cell.wins + cell.losses;
    if (state.tables.stage) {
      const rows = [];
      for (const stage of stages) {
        for (const rule of RANKED_RULES) {
          const cell = cellOf(stage, rule);
          if (cell.n) rows.push([stage, RULE_LABEL[rule], String(cell.n), decided(cell) >= 3 ? pct(cell.wr, 0) : '样本不足']);
        }
      }
      renderTable(box, [{ label: '场地' }, { label: '规则' }, { label: '场次', num: true }, { label: '胜率', num: true }], rows);
      return;
    }
    const shown = stages.slice(0, STAGE_ROWS);
    const grid = shown.map((stage) => RANKED_RULES.map((rule) => cellOf(stage, rule)));
    box.textContent = '';
    const width = Math.max(280, Math.floor(box.clientWidth));
    const cellHeight = 24;
    const height = 18 + cellHeight * shown.length;
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': '场地与规则的胜率矩阵' }, box);
    // 场地名完整显示：标签列按最长的名字留宽度；只有卡片窄到放不下时才截断。
    const labelGap = 10;
    const maxLabel = width - RANKED_RULES.length * 36 - labelGap;
    const labels = shown.map((stage, stageIndex) => {
      const label = svgText(svg, 0, 18 + stageIndex * cellHeight + cellHeight / 2 + 4, stage, { fill: COLOR.ink2 });
      for (let length = stage.length - 1; length > 0 && label.getComputedTextLength() > maxLabel; length -= 1) label.textContent = `${stage.slice(0, length)}…`;
      return label;
    });
    const labelWidth = Math.ceil(Math.max(0, ...labels.map((label) => label.getComputedTextLength()))) + labelGap;
    const cellWidth = (width - labelWidth) / RANKED_RULES.length;
    RANKED_RULES.forEach((rule, index) => svgText(svg, labelWidth + index * cellWidth + cellWidth / 2, 12, RULE_SHORT[rule], { 'text-anchor': 'middle' }));
    grid.forEach((row, stageIndex) => {
      row.forEach((cell, ruleIndex) => {
        const fill = decided(cell) >= 3 ? divergingColor(cell.wr) : COLOR.seq0;
        const cx = labelWidth + ruleIndex * cellWidth;
        const cy = 18 + stageIndex * cellHeight;
        const rect = svgEl('rect', { x: cx + 1, y: cy + 1, width: cellWidth - 2, height: cellHeight - 2, rx: 3, fill, class: 'cell', tabindex: cell.n ? '0' : '-1' }, svg);
        bindTip(rect, () => [`${shown[stageIndex]} · ${RULE_LABEL[RANKED_RULES[ruleIndex]]}`, [
          { value: decided(cell) >= 3 ? pct(cell.wr, 0) : cell.n ? '样本不足' : '没有对战', label: '胜率', color: fill, box: true },
          { value: `${cell.wins} 胜 ${cell.losses} 负`, label: `${cell.n} 场` },
        ]]);
        if (decided(cell) >= 3 && cellWidth >= 40) svgText(svg, cx + cellWidth / 2, cy + cellHeight / 2 + 4, pct(cell.wr, 0), { 'text-anchor': 'middle', fill: textOn(fill), 'pointer-events': 'none', class: 'font-number' });
      });
    });
  }

  function renderWeapons() {
    const box = document.getElementById('weapon-table');
    const [start, end] = rangeBounds(state.range);
    const list = battlesIn(start, end);
    const byWeapon = new Map();
    for (const battle of list) {
      if (!byWeapon.has(battle.weapon)) byWeapon.set(battle.weapon, []);
      byWeapon.get(battle.weapon).push(battle);
    }
    const rows = [...byWeapon.entries()]
      .map(([name, used]) => ({ name, last: used[used.length - 1].t, ...summarize(used) }))
      .sort((a, b) => b.n - a.n)
      .slice(0, 6);
    if (!rows.length) { emptyState(box, '这段时间没有对战', '换一个时间范围看看。'); return; }
    renderTable(box, [{ label: '武器' }, { label: '场次', num: true }, { label: '胜率', number: true }, { label: 'K/D', num: true }, { label: '场均涂地', num: true }, { label: '最近使用', num: true }], rows.map((row) => [
      row.name, String(row.n),
      row.wr == null ? '—' : el('span', {}, [el('span', { class: 'meter', 'aria-hidden': 'true' }, el('span', { style: `width:${(row.wr * 100).toFixed(1)}%` })), document.createTextNode(pct(row.wr))]),
      fixed(row.kd, 2), count(row.avgPaint), fmtSince(row.last),
    ]));
  }

  function renderSalmon() {
    const stats = document.getElementById('salmon-stats');
    const box = document.getElementById('salmon-chart');
    const [start, end] = rangeBounds(state.range);
    const list = shiftsIn(start, end);
    const graded = shifts.filter((shift) => shift.gradeName);
    const lastGraded = graded[graded.length - 1];
    const kingAppeared = list.filter((shift) => shift.kingAppeared).length;
    const kingDefeated = list.filter((shift) => shift.kingDefeated).length;
    const rescues = average(list, (shift) => shift.rescues);
    const rescued = average(list, (shift) => shift.rescued);
    stats.textContent = '';
    const items = [
      ['评级', lastGraded ? `${lastGraded.gradeName} ${lastGraded.gradePoint ?? ''}`.trim() : '—'],
      ['打工场次', [String(list.length), el('small', { text: ' 场' })]],
      ['通关率', pct(clearRate(list), 0)],
      ['场均金鲑鱼卵', fixed(average(list, (shift) => shift.goldenEggs))],
      ['场均救援 / 被救', rescues == null && rescued == null ? '—' : `${fixed(rescues)} / ${fixed(rescued)}`],
      ['头目鲑鱼击倒', kingAppeared ? `${kingDefeated} / ${kingAppeared}` : '—'],
    ];
    for (const [label, value] of items) stats.append(el('div', { class: 'mini-stat' }, [el('span', { text: label }), el('strong', { class: value === '—' ? 'placeholder' : '' }, value)]));

    const waves = WATER.map((water) => {
      const attempted = list.filter((shift) => !shift.disconnected).flatMap((shift) => shift.waves).filter((wave) => wave.water === water);
      const cleared = attempted.filter((wave) => wave.cleared).length;
      return { water, attempted: attempted.length, cleared, rate: attempted.length ? cleared / attempted.length : null };
    });
    if (!list.length) emptyState(box, '这段时间没有打工记录', '换一个时间范围看看。');
    else if (state.tables.salmon) renderTable(box, [{ label: '潮位' }, { label: '波次', num: true }, { label: '通关', num: true }, { label: '通关率', num: true }], waves.map((wave) => [wave.water, String(wave.attempted), String(wave.cleared), pct(wave.rate, 0)]));
    else {
      box.textContent = '';
      const width = Math.max(220, Math.floor(box.clientWidth));
      const height = 150;
      const margin = { top: 18, bottom: 22, left: 34, right: 8 };
      const y = (value) => margin.top + (1 - value) * (height - margin.top - margin.bottom);
      const band = (width - margin.left - margin.right) / waves.length;
      const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `各潮位通关率：${waves.map((wave) => `${wave.water} ${pct(wave.rate, 0)}`).join('，')}` }, box);
      for (const tick of [0, 0.5, 1]) {
        svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: COLOR.line, 'stroke-width': 1 }, svg);
        svgText(svg, margin.left - 6, y(tick) + 4, `${tick * 100}%`, { 'text-anchor': 'end', class: 'font-number' });
      }
      waves.forEach((wave, index) => {
        const barWidth = Math.min(24, band * 0.5);
        const cx = margin.left + band * index + band / 2;
        svgText(svg, cx, height - 6, wave.water, { 'text-anchor': 'middle' });
        if (wave.rate == null) return;
        const top = y(wave.rate);
        const bar = svgEl('path', { d: columnPath(cx - barWidth / 2, cx + barWidth / 2, top, y(0), true), fill: COLOR.salmon, class: 'cell', tabindex: '0' }, svg);
        bindTip(bar, () => [wave.water, [{ value: pct(wave.rate, 0), label: `${wave.cleared} / ${wave.attempted} 波`, color: COLOR.salmon, box: true }]]);
        svgText(svg, cx, top - 6, pct(wave.rate, 0), { 'text-anchor': 'middle', fill: COLOR.ink, 'font-size': 14, class: 'font-number' });
      });
    }

    renderRotation();
  }

  // 当前打工：直接用 OBS 鲑鱼跑卡片的数据（同一期工由后端按场地和发放的武器判断），
  // 第三行的鳞片、头目和场次总计沿用卡片的样式。
  function renderRotation() {
    const box = document.getElementById('salmon-rotation');
    box.textContent = '';
    if (!salmonRun) {
      box.append(el('span', { text: '还没有打工记录' }));
      return;
    }
    const playedAt = Date.parse(salmonRun.played_time ?? '');
    box.append(el('div', {}, [
      el('strong', { text: data.gameName('coopStages', salmonRun.stage) || '未知场地' }),
      Number.isFinite(playedAt) ? document.createTextNode(' · 最近一场 ') : null,
      Number.isFinite(playedAt) ? el('span', { class: 'font-number', text: fmtDateTime(playedAt) }) : null,
    ]));
    const totals = el('div', { class: 'rotation-totals' });
    box.append(totals);
    window.Splat3OverlayCard?.updateSalmonTotals(totals, { salmon_run: salmonRun });
  }

  function powerNode(battle) {
    if (battle.power == null) return el('span', { class: 'placeholder', text: '—' });
    const value = document.createTextNode(`${battle.power.toFixed(1)} `);
    const tracked = (battle.mode === 'x' && battle.powerKind === 'x') || (battle.mode === 'anarchy' && battle.bankara === 'challenge');
    return el('span', {}, tracked ? [value, deltaNode(battle.delta, 1, '')] : [value]);
  }
  const modeLabelOf = (battle) => (battle.mode === 'anarchy' && battle.bankara ? `蛮颓 · ${battle.bankara === 'challenge' ? '挑战' : '开放'}` : MODE[battle.mode].label);
  function resultChip(battle) {
    if (battle.win) return el('span', { class: 'chip win', text: '胜' });
    if (battle.lose) return el('span', { class: 'chip lose', text: '负' });
    return el('span', { class: 'chip other', text: '—' });
  }
  // 打工的结果：通关，或者失败的那一波；掉线没有结果，和无效对战一样用灰色。
  const salmonChip = (ok, text) => el('span', { class: `chip ${ok ? 'clear' : 'fail'}`, text });
  const shiftResult = (shift) => (shift.cleared ? '通关' : shift.disconnected ? '掉线' : `第 ${shift.failedWave ?? '?'} 波失败`);
  const shiftChip = (shift) => (shift.disconnected
    ? el('span', { class: 'chip other', text: '掉线' })
    : salmonChip(shift.cleared, shift.cleared ? '通关' : `第 ${shift.failedWave ?? '?'} 波`));

  function renderRecent() {
    const box = document.getElementById('recent-table');
    const [start, end] = rangeBounds(state.range);
    const salmon = state.mode === 'salmon';
    box.classList.add('is-full');
    document.getElementById('recent-more').hidden = false;
    document.getElementById('recent-title').textContent = salmon ? '最近打工' : '最近对战';
    document.getElementById('recent-sub').textContent = salmon
      ? '每场打工的结果、金鲑鱼卵、救援和评级。'
      : '战力变化只和同一赛道的上一场比较：X 赛按规则，挑战按武器；开放和活动赛只显示战力。';
    if (salmon) {
      const recentShifts = shiftsIn(start, end).slice(-10).reverse();
      if (!recentShifts.length) { emptyState(box, '这段时间没有打工', '换一个时间范围看看。'); return; }
      renderTable(box, [{ label: '时间', number: true }, { label: '场地' }, { label: '结果' }, { label: '金鲑鱼卵', num: true }, { label: '救援 / 被救', num: true }, { label: '头目鲑鱼' }, { label: '评级', num: true }], recentShifts.map((shift) => [
        fmtDateTime(shift.t),
        shift.stage,
        shiftChip(shift),
        count(shift.goldenEggs),
        `${count(shift.rescues)} / ${count(shift.rescued)}`,
        shift.kingAppeared ? (shift.kingDefeated ? '已击倒' : '未击倒') : '—',
        shift.gradeName ? `${shift.gradeName} ${shift.gradePoint ?? ''}`.trim() : '—',
      ]), { onRow: (index) => openRecordListener?.('coop', recentShifts[index].id) });
      return;
    }
    const list = battlesIn(start, end).slice(-10).reverse();
    if (!list.length) { emptyState(box, '这段时间没有对战', '换一个时间范围看看。'); return; }
    renderTable(box, [{ label: '时间', number: true }, { label: '模式' }, { label: '规则 · 场地' }, { label: '武器' }, { label: '结果' }, { label: '击倒 / 协助 / 死亡', num: true }, { label: '战力', num: true }], list.map((battle) => [
      fmtDateTime(battle.t),
      el('span', {}, [el('span', { class: 'swatch', style: `background:${modeColor(battle.mode)}` }), document.createTextNode(modeLabelOf(battle))]),
      `${ruleLabel(battle)} · ${battle.stage}`,
      battle.weapon,
      resultChip(battle),
      `${count(battle.kills)} / ${count(battle.assists)} / ${count(battle.deaths)}`,
      powerNode(battle),
    ]), { onRow: (index) => openRecordListener?.('battle', list[index].id) });
  }

  function render() {
    const salmon = state.mode === 'salmon';
    document.getElementById('overview-grid').classList.toggle('is-salmon', salmon);
    for (const card of document.querySelectorAll('[data-battle-only]')) card.hidden = salmon;
    hideTip();
    renderKpis();
    renderForm();
    renderPlaytime();
    renderHeat();
    renderSalmon();
    renderRecent();
    if (!salmon) {
      renderXPower();
      renderModes();
      renderStageRule();
      renderWeapons();
    }
  }

  function setData(facts) {
    ({ battles, shifts, xSettlements, coopRecord } = data.prepare(facts));
    xRules = new Set(xSettlements.map((item) => item.rule));
    if (!xRules.has(state.rule)) state.rule = RANKED_RULES.find((rule) => xRules.has(rule)) || RANKED_RULES[0];
    coopRules = COOP_RULES.filter((rule) => shifts.some((shift) => shift.rule === rule.key));
    state.coopRule = coopRules.some((rule) => rule.key === chosen.coopRule) ? chosen.coopRule : 'all';
    renderCoopRuleFilter();
  }

  const modeFilter = document.getElementById('mode-filter');
  modeFilter.append(el('button', { type: 'button', 'data-mode': 'all', 'aria-pressed': 'true', text: '全部模式' }));
  for (const option of [
    { key: 'x', label: MODE.x.short, color: () => COLOR.modes.x },
    { key: 'anarchy', label: MODE.anarchy.short, color: () => COLOR.modes.anarchy },
    { key: 'salmon', label: '打工', color: () => COLOR.salmon },
  ]) {
    modeFilter.append(el('button', { type: 'button', 'data-mode': option.key, 'aria-pressed': 'false' }, [el('span', { class: 'mode-dot', style: `background:${option.color()}`, 'aria-hidden': 'true' }), document.createTextNode(option.label)]));
  }
  // 鲑鱼跑页的打工模式筛选：只在记录里有不止一种打工模式时显示。
  const coopRuleFilter = document.getElementById('coop-rule-filter');
  let coopRules = [];
  let modeScope = 'all';
  function renderCoopRuleFilter() {
    coopRuleFilter.replaceChildren(
      el('button', { type: 'button', 'data-coop-rule': 'all', 'aria-pressed': String(state.coopRule === 'all'), text: '全部模式' }),
      ...coopRules.map((rule) => el('button', { type: 'button', 'data-coop-rule': rule.key, 'aria-pressed': String(state.coopRule === rule.key), text: rule.label })),
    );
    coopRuleFilter.hidden = modeScope !== 'salmon' || coopRules.length < 2;
  }
  function pressSegmented(container, attribute, value) {
    for (const button of container.querySelectorAll(`button[${attribute}]`)) button.setAttribute('aria-pressed', String(button.getAttribute(attribute) === value));
  }
  function bindSegmented(container, attribute, apply) {
    container.addEventListener('click', (event) => {
      const button = event.target.closest(`button[${attribute}]`);
      if (!button) return;
      pressSegmented(container, attribute, button.getAttribute(attribute));
      apply(button.getAttribute(attribute));
    });
  }
  // 时间范围：近 7 天、近 30 天、全部是普通分段；第三格是菜单按钮，按下状态和文字跟着自然周期走。
  function pressRange() {
    for (const button of rangeFilter.querySelectorAll(':scope > button[data-range]')) button.setAttribute('aria-pressed', String(button.getAttribute('data-range') === state.range));
    for (const item of periodMenu.querySelectorAll('button[data-range]')) item.setAttribute('aria-checked', String(item.getAttribute('data-range') === state.range));
    const label = periodMenu.querySelector(`[data-range="${chosen.period}"] .range-item-name`).textContent;
    periodButton.querySelector('.range-period-label').textContent = label;
    periodButton.setAttribute('aria-label', `自然周期：${label}`);
    periodButton.setAttribute('aria-pressed', String(periods.includes(state.range)));
  }
  // 和账号菜单一样：打开时焦点放在选中的一项，点外面或按 Esc 收起。每次打开时重算各项从哪天开始。
  function setPeriodMenu(open) {
    if (open) {
      for (const item of periodMenu.querySelectorAll('button[data-range]')) {
        item.querySelector('.range-item-from').textContent = `${fmtShortDate(rangeBounds(item.getAttribute('data-range'))[0])} 起`;
      }
    }
    periodMenu.hidden = !open;
    periodButton.setAttribute('aria-expanded', String(open));
    if (open) (periodMenu.querySelector('[aria-checked="true"]') || periodMenu.querySelector('button'))?.focus();
  }
  // 记下的选择已经不在按钮里时回到默认。
  const rangeFilter = document.getElementById('range-filter');
  const periodButton = document.getElementById('range-period-button');
  const periodMenu = document.getElementById('range-period-menu');
  const optionsOf = (container, attribute) => [...container.querySelectorAll(`button[${attribute}]`)].map((button) => button.getAttribute(attribute));
  const periods = optionsOf(periodMenu, 'data-range');
  if (!optionsOf(rangeFilter, 'data-range').includes(state.range)) state.range = '30d';
  if (periods.includes(state.range)) chosen.period = state.range;
  else if (!periods.includes(chosen.period)) chosen.period = 'season';
  if (!optionsOf(modeFilter, 'data-mode').includes(chosen.mode)) chosen.mode = state.mode = 'all';
  pressRange();
  pressSegmented(modeFilter, 'data-mode', state.mode);
  // 时间范围和模式筛选由总览和对战记录共用，改动后通知当前页面重新绘制。
  const filtersChanged = () => (filterListener ? filterListener() : render());
  rangeFilter.addEventListener('click', (event) => {
    const button = event.target.closest('button[data-range]');
    if (!button) return;
    state.range = button.getAttribute('data-range');
    if (periods.includes(state.range)) chosen.period = state.range;
    if (periodMenu.contains(button)) {
      setPeriodMenu(false);
      periodButton.focus();
    }
    pressRange();
    saveFilters();
    filtersChanged();
  });
  periodButton.addEventListener('click', () => setPeriodMenu(periodMenu.hidden));
  document.addEventListener('click', (event) => {
    if (!periodMenu.hidden && !event.target.closest('.range-period')) setPeriodMenu(false);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !periodMenu.hidden) {
      setPeriodMenu(false);
      periodButton.focus();
    }
  });
  bindSegmented(modeFilter, 'data-mode', (value) => { chosen.mode = state.mode = value; saveFilters(); filtersChanged(); });
  bindSegmented(coopRuleFilter, 'data-coop-rule', (value) => { chosen.coopRule = state.coopRule = value; saveFilters(); filtersChanged(); });
  bindSegmented(document.getElementById('heat-metric'), 'data-metric', (value) => { state.heatMetric = value; renderHeat(); });
  for (const button of document.querySelectorAll('[data-table-toggle]')) {
    button.addEventListener('click', () => {
      const key = button.getAttribute('data-table-toggle');
      state.tables[key] = !state.tables[key];
      button.textContent = state.tables[key] ? '查看图表' : '查看表格';
      // 由当前页面重新绘制（总览或分析页）。
      filtersChanged();
    });
  }

  window.Splat3Dashboard = Object.freeze({
    setData,
    // 返回已整理的数据引用，供分享图按完整时间段统计；调用方不修改这些数组。
    records: () => ({ battles, shifts, xSettlements, mode: state.mode }),
    // SplatNet「鲑鱼跑记录」的累计击倒数量，不随时间范围变化；还没查到时为 null。
    coopRecord: () => coopRecord,
    setSalmonRun(run) {
      salmonRun = run || null;
      renderRotation();
    },
    render,
    hasRecords: () => battles.length > 0 || shifts.length > 0,
    onFiltersChange(listener) {
      filterListener = listener;
    },
    onOpenRecord(listener) {
      openRecordListener = listener;
    },
    // 对战记录页的数据：当前时间范围和模式下的对战，或者选“打工”时的打工记录。
    logSource() {
      const [start, end] = rangeBounds(state.range);
      return state.mode === 'salmon' ? { salmon: true, rows: shiftsIn(start, end) } : { salmon: false, rows: battlesIn(start, end) };
    },
    // 分析页的数据：当前时间范围和模式下的对战，以及时间范围内、所选打工模式的打工。
    analysisSource() {
      const [start, end] = rangeBounds(state.range);
      const inCoopRule = (shift) => state.coopRule === 'all' || shift.rule === state.coopRule;
      return { start, end, mode: state.mode, battles: battlesIn(start, end), shifts: shiftsIn(start, end).filter(inCoopRule) };
    },
    tableOn: (key) => Boolean(state.tables[key]),
    // 对战、武器、场地页不提供“打工”筛选；鲑鱼跑页把对战模式筛选换成打工模式筛选。
    setModeScope(scope) {
      modeScope = scope;
      coopRuleFilter.hidden = scope !== 'salmon' || coopRules.length < 2;
      modeFilter.hidden = scope === 'salmon';
      modeFilter.querySelector('[data-mode="salmon"]').hidden = scope === 'battles';
      // 选着「打工」时，这几页先按全部模式显示，回到其他页面再恢复。
      state.mode = scope === 'battles' && chosen.mode === 'salmon' ? 'all' : chosen.mode;
      pressSegmented(modeFilter, 'data-mode', state.mode);
    },
    ui: Object.freeze({
      el, fmtDateTime, fmtDate, fmtShortDate, fmtSince, count, pct, fixed, signed, deltaNode,
      MODE, MODES, RULE_LABEL, RANKED_RULES, COOP_RULE_LABEL, ruleLabel, modeLabelOf, modeColor, powerNode, emptyState, clickableRow,
      salmonChip, shiftChip, shiftResult,
      svgEl, svgText, niceScale, columnPath, renderTable, textOn, divergingColor, showTip, hideTip, bindTip,
      colors: () => COLOR,
    }),
    onThemeChange() {
      COLOR = readColors();
    },
    hideTip,
  });
})();
