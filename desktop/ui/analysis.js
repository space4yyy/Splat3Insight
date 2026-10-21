(() => {
  'use strict';

  // 分析页（对战、武器、场地、鲑鱼跑）的绘制。数据来自 Splat3Dashboard.analysisSource()，
  // 汇总用 Splat3AnalysisData 的纯函数，图表的样式和总览一致。
  const dashboard = window.Splat3Dashboard;
  const records = window.Splat3Records;
  const analysis = window.Splat3AnalysisData;
  const { WATER } = window.Splat3DashboardData;
  const {
    el, fmtDateTime, fmtDate, fmtShortDate, fmtSince, count, pct, fixed, signed,
    MODE, RULE_LABEL, COOP_RULE_LABEL, modeColor, emptyState, clickableRow,
    svgEl, svgText, niceScale, columnPath, renderTable, textOn, divergingColor, showTip, hideTip, bindTip,
  } = dashboard.ui;
  let COLOR = dashboard.ui.colors();
  const $ = (id) => document.getElementById(id);
  const decided = (stat) => stat.wins + stat.losses;
  const enough = (stat) => decided(stat) >= analysis.MIN_DECIDED;
  const ruleName = (rule, fallback) => RULE_LABEL[rule] || fallback || '未知规则';
  const noBattles = (box) => emptyState(box, '这段时间没有对战', '换一个时间范围或模式看看。');
  const noShifts = (box) => emptyState(box, '这段时间没有打工记录', '换一个时间范围看看。');
  function fmtClock(seconds) {
    const total = Math.round(seconds);
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
  }

  // ---------- 通用图形 ----------
  const measureContext = document.createElement('canvas').getContext('2d');
  function textWidth(text, size = 13) {
    measureContext.font = `${size}px ${getComputedStyle(document.body).fontFamily}`;
    return measureContext.measureText(text).width + text.length * size * 0.02;
  }
  const hatchCss = (color, angle) => `repeating-linear-gradient(${angle}deg, ${color} 0 3px, ${COLOR.panel} 3px 5px)`;
  function hatchFill(svg, color, angle) {
    const id = `hatch-${Math.random().toString(36).slice(2)}`;
    const defs = svg.querySelector('defs') || svgEl('defs', {}, svg);
    const pattern = svgEl('pattern', { id, patternUnits: 'userSpaceOnUse', width: 5, height: 5, patternTransform: `rotate(${angle})` }, defs);
    svgEl('rect', { width: 5, height: 5, fill: color }, pattern);
    svgEl('rect', { width: 2, height: 5, fill: COLOR.panel }, pattern);
    return `url(#${id})`;
  }
  const meterCell = (rate, digits = 1) => (rate == null ? '—' : el('span', {}, [
    el('span', { class: 'meter', 'aria-hidden': 'true' }, el('span', { style: `width:${(rate * 100).toFixed(1)}%` })),
    document.createTextNode(pct(rate, digits)),
  ]));
  const legendSwatch = (background, text) => el('span', {}, [el('span', { class: 'swatch', style: `background:${background}` }), document.createTextNode(text)]);
  const divergingLegend = (low, high, thin, colors = COLOR.div) => [
    el('span', {}, [document.createTextNode(`${low} `), el('span', { class: 'legend-scale' }, colors.map((color) => el('span', { style: `background:${color}` }))), document.createTextNode(` ${high}`)]),
    el('span', {}, [el('span', { class: 'legend-scale' }, [el('span', { class: 'thin-swatch' })]), document.createTextNode(` ${thin}`)]),
  ];
  const miniStat = (label, value) => el('div', { class: 'mini-stat' }, [el('span', { text: label }), el('strong', { class: value === '—' ? 'placeholder' : '', text: value })]);
  // 横条从 from 画到 to，只有 to 那一端是圆角；竖条同理。差不多为零时画一条细线。
  function hbarPath(from, to, top, bottom) {
    const left = Math.min(from, to);
    const right = Math.max(from, to);
    if (right - left < 2) return `M${from - 1},${top} H${from + 1} V${bottom} H${from - 1} Z`;
    const r = Math.min(4, (bottom - top) / 2, right - left);
    if (to >= from) return `M${left},${top} H${right - r} Q${right},${top} ${right},${top + r} V${bottom - r} Q${right},${bottom} ${right - r},${bottom} H${left} Z`;
    return `M${right},${top} H${left + r} Q${left},${top} ${left},${top + r} V${bottom - r} Q${left},${bottom} ${left + r},${bottom} H${right} Z`;
  }
  function vbarPath(from, to, left, right) {
    const top = Math.min(from, to);
    const bottom = Math.max(from, to);
    if (bottom - top < 2) return `M${left},${from - 1} H${right} V${from + 1} H${left} Z`;
    const r = Math.min(4, (right - left) / 2, bottom - top);
    if (to <= from) return `M${left},${bottom} V${top + r} Q${left},${top} ${left + r},${top} H${right - r} Q${right},${top} ${right},${top + r} V${bottom} Z`;
    return `M${left},${top} H${right} V${bottom - r} Q${right},${bottom} ${right - r},${bottom} H${left + r} Q${left},${bottom} ${left},${bottom - r} Z`;
  }
  const rowHit = (svg, top, width, height) => svgEl('rect', { x: 0, y: top, width, height, class: 'col-hit', tabindex: '0' }, svg);

  // 直方图：bins 是 { value, tick, title }；marker 是中位数竖线，position 以格子为单位（0 是第一格的中心）。
  function drawHistogram(box, { bins, color, height = 150, labelEvery = 5, marker, aria, unit = '场' }) {
    box.textContent = '';
    const width = Math.max(200, Math.floor(box.clientWidth));
    const margin = { top: 24, right: 8, bottom: 22, left: 32 };
    const scale = niceScale(0, Math.max(1, ...bins.map((bin) => bin.value)), 3);
    const band = (width - margin.left - margin.right) / bins.length;
    const barWidth = Math.max(2, Math.min(band - 2, 22));
    const total = bins.reduce((sum, bin) => sum + bin.value, 0);
    const y = (value) => margin.top + (1 - value / scale.max) * (height - margin.top - margin.bottom);
    const center = (position) => margin.left + band * (position + 0.5);
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': aria }, box);
    for (const tick of scale.ticks) {
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: tick === 0 ? COLOR.lineStrong : COLOR.line, 'stroke-width': 1 }, svg);
      svgText(svg, margin.left - 6, y(tick) + 4, String(tick), { 'text-anchor': 'end', class: 'font-number' });
    }
    bins.forEach((bin, index) => {
      const cx = center(index);
      if (index % labelEvery === 0) svgText(svg, cx, height - 6, bin.tick, { 'text-anchor': 'middle', class: 'font-number' });
      if (bin.value > 0) svgEl('path', { d: columnPath(cx - barWidth / 2, cx + barWidth / 2, y(bin.value), y(0), true), fill: color }, svg);
      const hit = svgEl('rect', { x: margin.left + band * index, y: margin.top, width: band, height: height - margin.top - margin.bottom, class: 'col-hit', tabindex: bin.value ? '0' : '-1' }, svg);
      bindTip(hit, () => [bin.title, [{ value: `${bin.value} ${unit}`, label: total ? `占 ${pct(bin.value / total, 0)}` : '', color, box: true }]]);
    });
    if (marker) {
      const mx = center(marker.position);
      svgEl('line', { x1: mx, x2: mx, y1: margin.top - 6, y2: y(0), stroke: COLOR.ink, 'stroke-width': 1, 'stroke-dasharray': '3 3', 'pointer-events': 'none' }, svg);
      const anchor = mx > width - 56 ? 'end' : mx < margin.left + 56 ? 'start' : 'middle';
      svgText(svg, mx + (anchor === 'start' ? -2 : anchor === 'end' ? 2 : 0), margin.top - 10, marker.label, { 'text-anchor': anchor, fill: COLOR.ink, class: 'font-number', 'pointer-events': 'none' });
    }
  }
  function histogramBins(result, { step, label, unit }) {
    return result.bins.map((bin) => {
      const range = step === 1 ? `${bin.low}` : `${bin.low}–${bin.low + step - 1}`;
      return { value: bin.value, tick: bin.last ? `${bin.low}+` : String(bin.low), title: `${label} ${bin.last ? `${bin.low} ${unit}以上` : `${range} ${unit}`}` };
    });
  }
  const fmtMedian = (value) => (value == null ? '—' : Number.isInteger(value) ? String(value) : value.toFixed(1));

  // 折线：x 是时间或序号；指针靠近哪个点就提示哪个点。
  function drawLine(box, { points, color, height = 170, yMin, yMax, ticks, tickLabel = String, strongTick = () => false, xMin, xMax, xLabels = [], refValue = null, area = false, endLabel, aria, marginLeft = 40, marginRight = 48 }) {
    box.textContent = '';
    const width = Math.max(220, Math.floor(box.clientWidth));
    const margin = { top: 12, right: marginRight, bottom: 22, left: marginLeft };
    const span = xMax - xMin || 1;
    const x = (value) => margin.left + ((value - xMin) / span) * (width - margin.left - margin.right);
    const y = (value) => margin.top + (1 - (value - yMin) / (yMax - yMin || 1)) * (height - margin.top - margin.bottom);
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': aria }, box);
    for (const tick of ticks) {
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: strongTick(tick) ? COLOR.lineStrong : COLOR.line, 'stroke-width': 1 }, svg);
      svgText(svg, margin.left - 6, y(tick) + 4, tickLabel(tick), { 'text-anchor': 'end', class: 'font-number' });
    }
    for (const [value, text] of xLabels) {
      const px = x(value);
      const anchor = px <= margin.left + 1 ? 'start' : px >= width - margin.right - 1 ? 'end' : 'middle';
      svgText(svg, px, height - 6, text, { 'text-anchor': anchor, class: 'font-number' });
    }
    if (refValue != null) svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(refValue), y2: y(refValue), stroke: COLOR.ink3, 'stroke-width': 1, 'stroke-dasharray': '3 3' }, svg);
    const line = points.map((point, index) => `${index ? 'L' : 'M'}${x(point.x).toFixed(1)},${y(point.y).toFixed(1)}`).join(' ');
    if (area && points.length > 1) svgEl('path', { d: `${line} L${x(points[points.length - 1].x).toFixed(1)},${y(yMin)} L${x(points[0].x).toFixed(1)},${y(yMin)} Z`, fill: color, opacity: 0.1 }, svg);
    if (points.length > 1) svgEl('path', { d: line, fill: 'none', stroke: color, 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }, svg);
    const last = points[points.length - 1];
    svgEl('circle', { cx: x(last.x), cy: y(last.y), r: 4, fill: color, stroke: COLOR.panel, 'stroke-width': 2 }, svg);
    if (endLabel) svgText(svg, x(last.x) + 8, y(last.y) + 5, endLabel, { fill: COLOR.ink, 'font-size': 13, class: 'font-number' });
    const crosshair = svgEl('line', { y1: margin.top, y2: height - margin.bottom, stroke: COLOR.ink3, 'stroke-width': 1, visibility: 'hidden' }, svg);
    const dot = svgEl('circle', { r: 4, fill: color, stroke: COLOR.panel, 'stroke-width': 2, visibility: 'hidden' }, svg);
    const overlay = svgEl('rect', { x: margin.left, y: margin.top, width: width - margin.left - margin.right, height: height - margin.top - margin.bottom, fill: 'transparent' }, svg);
    overlay.addEventListener('pointermove', (event) => {
      const bounds = svg.getBoundingClientRect();
      const value = xMin + ((event.clientX - bounds.left - margin.left) / (width - margin.left - margin.right)) * span;
      let best = points[0];
      for (const point of points) if (Math.abs(point.x - value) < Math.abs(best.x - value)) best = point;
      crosshair.setAttribute('x1', x(best.x));
      crosshair.setAttribute('x2', x(best.x));
      crosshair.setAttribute('visibility', 'visible');
      dot.setAttribute('cx', x(best.x));
      dot.setAttribute('cy', y(best.y));
      dot.setAttribute('visibility', 'visible');
      showTip(event, best.title, best.rows);
    });
    overlay.addEventListener('pointerleave', () => {
      crosshair.setAttribute('visibility', 'hidden');
      dot.setAttribute('visibility', 'hidden');
      hideTip();
    });
  }

  // 矩阵单元格：至少 3 个样本才着色并显示百分比；样本不足只描边，没有样本显示“—”。
  function matrixCell({ n, rate, fill, unit, title, detail }) {
    const filled = fill != null;
    const chip = el('span', {
      class: filled ? 'rate-chip' : n ? 'rate-chip is-thin' : 'rate-chip is-empty',
      style: filled ? `background:${fill};color:${textOn(fill)}` : '',
      text: filled ? pct(rate, 0) : n ? `${n} ${unit}` : '—',
    });
    const td = el('td', { class: 'rate-cell', tabindex: n ? '0' : '-1' }, chip);
    if (n) bindTip(td, () => [title, [filled ? { value: pct(rate, 0), label: detail, color: fill, box: true } : { value: '样本不足', label: detail }]]);
    return td;
  }
  function sortHeader(columns, sort, onSort) {
    const row = el('tr');
    for (const column of columns) {
      const th = el('th', { class: column.num ? 'num' : '', scope: 'col' });
      if (column.sort) {
        const active = sort.key === column.sort;
        th.setAttribute('aria-sort', active ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none');
        const button = el('button', { type: 'button', class: 'sort-button', 'aria-pressed': String(active), text: `${column.label}${active ? (sort.dir === 'asc' ? ' ▲' : ' ▼') : ''}` });
        button.addEventListener('click', () => onSort(active ? { key: column.sort, dir: sort.dir === 'asc' ? 'desc' : 'asc' } : { key: column.sort, dir: 'desc' }));
        th.append(button);
      } else {
        th.textContent = column.label;
      }
      row.append(th);
    }
    return row;
  }

  // ---------- 对战 ----------
  function renderRuleChart(list) {
    const box = $('rule-chart');
    const legend = $('rule-legend');
    const rows = analysis.ruleBreakdown(list);
    legend.replaceChildren(
      legendSwatch(hatchCss(COLOR.meterWin, 45), '完胜'),
      legendSwatch(COLOR.meterWin, '胜'),
      legendSwatch(COLOR.meterLose, '负'),
      legendSwatch(hatchCss(COLOR.meterLose, -45), '完败'),
    );
    if (!rows.length) { noBattles(box); return; }
    if (dashboard.tableOn('rules')) {
      renderTable(box, [{ label: '规则' }, { label: '场次', num: true }, { label: '胜率', num: true }, { label: '完胜', num: true }, { label: '完败', num: true }, { label: 'K/D', num: true }], rows.map((row) => [
        ruleName(row.rule, row.ruleName), String(row.n), pct(row.wr), String(row.koWins), String(row.koLosses), fixed(row.kd, 2),
      ]));
      return;
    }
    box.textContent = '';
    const width = Math.max(280, Math.floor(box.clientWidth));
    const labelWidth = Math.ceil(Math.max(...rows.map((row) => textWidth(ruleName(row.rule, row.ruleName), 13)))) + 14;
    const rightWidth = 92;
    const rowHeight = 34;
    const barHeight = 18;
    const top = 4;
    const plotWidth = width - labelWidth - rightWidth;
    const height = top + rows.length * rowHeight + 20;
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `各规则胜率：${rows.map((row) => `${ruleName(row.rule, row.ruleName)} ${pct(row.wr, 0)}`).join('，')}` }, box);
    const winHatch = hatchFill(svg, COLOR.meterWin, 45);
    const loseHatch = hatchFill(svg, COLOR.meterLose, -45);
    const defs = svg.querySelector('defs');
    rows.forEach((row, index) => {
      const cy = top + index * rowHeight + rowHeight / 2;
      svgText(svg, 0, cy + 5, ruleName(row.rule, row.ruleName), { fill: COLOR.ink2, 'font-size': 13 });
      const segments = [
        { count: row.koWins, fill: winHatch },
        { count: row.wins - row.koWins, fill: COLOR.meterWin },
        { count: row.losses - row.koLosses, fill: COLOR.meterLose },
        { count: row.koLosses, fill: loseHatch },
      ].filter((segment) => segment.count > 0);
      if (segments.length) {
        const clipId = `rule-clip-${index}-${Math.random().toString(36).slice(2)}`;
        svgEl('rect', { x: labelWidth, y: cy - barHeight / 2, width: plotWidth, height: barHeight, rx: 4 }, svgEl('clipPath', { id: clipId }, defs));
        const group = svgEl('g', { 'clip-path': `url(#${clipId})` }, svg);
        const available = plotWidth - (segments.length - 1) * 2;
        let cursor = labelWidth;
        for (const segment of segments) {
          const segmentWidth = (available * segment.count) / decided(row);
          svgEl('rect', { x: cursor, y: cy - barHeight / 2, width: Math.max(1, segmentWidth), height: barHeight, fill: segment.fill }, group);
          cursor += segmentWidth + 2;
        }
      }
      svgText(svg, width - rightWidth + 10, cy + 5, pct(row.wr, 0), { fill: row.wr == null ? COLOR.ink3 : COLOR.ink, 'font-size': 14, class: 'font-number' });
      svgText(svg, width, cy + 5, `${row.n} 场`, { 'text-anchor': 'end', class: 'font-number' });
      bindTip(rowHit(svg, cy - rowHeight / 2, width, rowHeight), () => [ruleName(row.rule, row.ruleName), [
        { value: pct(row.wr), label: `${row.wins} 胜 ${row.losses} 负`, color: COLOR.meterWin, box: true },
        { value: `${row.koWins} 场`, label: '完胜', color: hatchCss(COLOR.meterWin, 45), box: true },
        { value: `${row.koLosses} 场`, label: '完败', color: hatchCss(COLOR.meterLose, -45), box: true },
        ...(row.modes.length > 1 ? row.modes.map((item) => ({ value: pct(item.wr, 0), label: `${MODE[item.mode]?.label ?? item.mode} · ${item.n} 场`, color: modeColor(item.mode), box: true })) : []),
      ]]);
    });
    const half = labelWidth + plotWidth / 2;
    svgEl('line', { x1: half, x2: half, y1: top, y2: top + rows.length * rowHeight, stroke: COLOR.ink, 'stroke-width': 1, 'stroke-dasharray': '3 3', 'pointer-events': 'none' }, svg);
    svgText(svg, half, height - 4, '50%', { 'text-anchor': 'middle', class: 'font-number' });
  }

  const streakTick = (key) => (key === 0 ? '开局' : `${Math.abs(key)}${Math.abs(key) === 4 ? '+' : ''}`);
  const streakName = (key) => (key === 0 ? '开局' : `之前${Math.abs(key)}${Math.abs(key) === 4 ? ' 场以上' : ' 场'}${key < 0 ? '连败' : '连胜'}`);
  function renderStreakChart(list) {
    const box = $('streak-chart');
    const rows = analysis.streakBuckets(list);
    if (!rows.some((row) => row.n)) { noBattles(box); return; }
    if (dashboard.tableOn('streak')) {
      renderTable(box, [{ label: '之前' }, { label: '场次', num: true }, { label: '胜', num: true }, { label: '负', num: true }, { label: '胜率', num: true }], rows.filter((row) => row.n).map((row) => [
        streakName(row.key).replace(/^之前/, ''), String(row.n), String(row.wins), String(row.losses), enough(row) ? pct(row.wr) : '样本不足',
      ]));
      return;
    }
    box.textContent = '';
    const width = Math.max(260, Math.floor(box.clientWidth));
    const height = 236;
    const margin = { top: 20, right: 4, bottom: 58, left: 38 };
    const shown = rows.filter(enough);
    // 纵轴以 50% 为中心，上下留出放数字的空间。
    const deviation = Math.min(0.5, Math.max(0.2, Math.ceil((Math.max(0, ...shown.map((row) => Math.abs(row.wr - 0.5))) + 0.06) * 10 - 1e-9) / 10));
    const low = 0.5 - deviation;
    const high = 0.5 + deviation;
    const y = (value) => margin.top + (1 - (Math.max(low, Math.min(high, value)) - low) / (high - low)) * (height - margin.top - margin.bottom);
    const band = (width - margin.left - margin.right) / rows.length;
    const barWidth = Math.max(6, Math.min(26, band * 0.56));
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `之前连胜或连败后的胜率：${shown.map((row) => `${streakName(row.key)} ${pct(row.wr, 0)}`).join('，')}` }, box);
    const step = deviation > 0.25 ? 0.2 : 0.1;
    const steps = Math.floor(deviation / step + 1e-9);
    for (let index = -steps; index <= steps; index += 1) {
      const tick = 0.5 + index * step;
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: index ? COLOR.line : COLOR.ink3, 'stroke-width': 1, 'stroke-dasharray': index ? 'none' : '3 3' }, svg);
      svgText(svg, margin.left - 6, y(tick) + 4, `${Math.round(tick * 100)}%`, { 'text-anchor': 'end', class: 'font-number' });
    }
    const axisY = height - margin.bottom;
    rows.forEach((row, index) => {
      const cx = margin.left + band * index + band / 2;
      svgText(svg, cx, axisY + 18, streakTick(row.key), { 'text-anchor': 'middle', fill: COLOR.ink2, class: 'font-number' });
      svgText(svg, cx, axisY + 34, row.n ? String(row.n) : '', { 'text-anchor': 'middle', 'font-size': 11, class: 'font-number' });
      if (enough(row)) {
        const up = row.wr >= 0.5;
        svgEl('path', { d: vbarPath(y(0.5), y(row.wr), cx - barWidth / 2, cx + barWidth / 2), fill: up ? COLOR.meterWin : COLOR.meterLose }, svg);
        svgText(svg, cx, up ? y(row.wr) - 6 : y(row.wr) + 15, pct(row.wr, 0), { 'text-anchor': 'middle', fill: COLOR.ink, 'font-size': 12, class: 'font-number' });
      }
      const hit = svgEl('rect', { x: margin.left + band * index, y: margin.top, width: band, height: axisY + 38 - margin.top, class: 'col-hit', tabindex: row.n ? '0' : '-1' }, svg);
      bindTip(hit, () => [streakName(row.key), [
        { value: enough(row) ? pct(row.wr) : row.n ? '样本不足' : '—', label: `${row.wins} 胜 ${row.losses} 负`, color: enough(row) ? (row.wr >= 0.5 ? COLOR.meterWin : COLOR.meterLose) : COLOR.seq0, box: true },
      ]]);
    });
    svgText(svg, margin.left + band * 2, height - 4, '← 之前连败', { 'text-anchor': 'middle' });
    svgText(svg, margin.left + band * 7, height - 4, '之前连胜 →', { 'text-anchor': 'middle' });
    svgText(svg, margin.left - 6, axisY + 34, '场', { 'text-anchor': 'end', 'font-size': 11 });
  }

  function renderKdChart(list) {
    const box = $('kd-chart');
    const games = list.filter((battle) => battle.kills != null && battle.deaths != null);
    if (!games.length) { noBattles(box); return; }
    const CAP = 15;
    const series = [
      { key: 'kills', label: '击倒', color: COLOR.meterWin },
      { key: 'deaths', label: '死亡', color: COLOR.meterLose },
    ].map((item) => ({ ...item, result: analysis.histogram(games.map((battle) => battle[item.key]), { step: 1, cap: CAP }) }));
    if (dashboard.tableOn('kd')) {
      renderTable(box, [{ label: '次数', number: true }, { label: '击倒', num: true }, { label: '死亡', num: true }], series[0].result.bins.map((bin, index) => [
        bin.last ? `${bin.low}+` : String(bin.low), String(bin.value), String(series[1].result.bins[index].value),
      ]));
      return;
    }
    box.textContent = '';
    // 先把两块都放进网格，再按各自的宽度绘制。
    const charts = series.map(() => el('div', { class: 'chart' }));
    box.append(el('div', { class: 'chart-pair' }, series.map((item, index) => el('div', { class: 'chart-block' }, [el('p', { class: 'subhead', text: item.label }), charts[index]]))));
    // 两张图并排时加高，和同一行的对局时长卡片对齐；上下排列时矮一些。
    const sideBySide = charts[0].getBoundingClientRect().top === charts[1].getBoundingClientRect().top;
    series.forEach((item, index) => {
      drawHistogram(charts[index], {
        height: sideBySide ? 226 : 150,
        bins: histogramBins(item.result, { step: 1, label: item.label, unit: '次' }),
        color: item.color,
        marker: { position: item.result.medianPosition, label: `中位数 ${fmtMedian(item.result.median)}` },
        aria: `每场${item.label}次数分布，中位数 ${fmtMedian(item.result.median)}`,
      });
    });
  }

  function durationLabel(bin) {
    if (bin.fullTime) return '5:00 时间到';
    if (bin.overtime) return bin.to == null ? `加时 ${fmtClock(bin.from)} 以上` : `加时 ${fmtClock(bin.from)}–${fmtClock(bin.to)}`;
    return `${fmtClock(bin.from)}–${fmtClock(bin.to)}`;
  }
  function renderDurationChart(list) {
    const box = $('duration-chart');
    const statsBox = $('duration-stats');
    const legend = $('duration-legend');
    const summary = analysis.durationSummary(list);
    statsBox.textContent = '';
    legend.textContent = '';
    if (!summary.n) { emptyState(box, '这段时间没有真格规则的对战', '换一个时间范围或模式看看。'); return; }
    statsBox.append(
      miniStat('平均用时', summary.average == null ? '—' : fmtClock(summary.average)),
      miniStat('加时', `${summary.overtime.n} 场`),
      miniStat('加时胜率', pct(summary.overtime.wr, 0)),
    );
    legend.append(legendSwatch(COLOR.meterWin, '胜'), legendSwatch(COLOR.meterLose, '负'));
    const { bins } = summary;
    if (dashboard.tableOn('duration')) {
      renderTable(box, [{ label: '用时', number: true }, { label: '胜', num: true }, { label: '负', num: true }, { label: '胜率', num: true }], bins
        .filter((bin) => bin.n)
        .map((bin) => [durationLabel(bin), String(bin.wins), String(bin.losses), decided(bin) ? pct(bin.wins / decided(bin), 0) : '—']));
      return;
    }
    box.textContent = '';
    const width = Math.max(240, Math.floor(box.clientWidth));
    const height = 164;
    const margin = { top: 22, right: 8, bottom: 22, left: 32 };
    const scale = niceScale(0, Math.max(1, ...bins.map((bin) => decided(bin))), 3);
    const band = (width - margin.left - margin.right) / bins.length;
    const barWidth = Math.max(2, Math.min(band - 3, 22));
    const y = (value) => margin.top + (1 - value / scale.max) * (height - margin.top - margin.bottom);
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `真格规则对局时长分布，加时 ${summary.overtime.n} 场` }, box);
    const overtimeX = margin.left + band * 11;
    svgEl('rect', { x: overtimeX, y: margin.top, width: width - margin.right - overtimeX, height: height - margin.top - margin.bottom, fill: COLOR.seq0 }, svg);
    svgText(svg, overtimeX + 4, margin.top - 8, '加时', { fill: COLOR.ink2 });
    for (const tick of scale.ticks) {
      svgEl('line', { x1: margin.left, x2: width - margin.right, y1: y(tick), y2: y(tick), stroke: tick === 0 ? COLOR.lineStrong : COLOR.line, 'stroke-width': 1 }, svg);
      svgText(svg, margin.left - 6, y(tick) + 4, String(tick), { 'text-anchor': 'end', class: 'font-number' });
    }
    const labelStep = band * 2 >= 40 ? 2 : 4;
    for (let edge = 0; edge <= 8; edge += labelStep) svgText(svg, margin.left + band * edge, height - 6, fmtClock(edge * 30), { 'text-anchor': 'middle', class: 'font-number' });
    svgText(svg, margin.left + band * 10.5, height - 6, '5:00', { 'text-anchor': 'middle', class: 'font-number' });
    bins.forEach((bin, index) => {
      const cx = margin.left + band * index + band / 2;
      const pieces = [{ value: bin.wins, fill: COLOR.meterWin }, { value: bin.losses, fill: COLOR.meterLose }].filter((piece) => piece.value > 0);
      let cumulative = 0;
      let previousTop = y(0);
      pieces.forEach((piece, pieceIndex) => {
        cumulative += piece.value;
        const top = y(cumulative);
        const bottom = pieceIndex ? previousTop - 2 : previousTop;
        if (bottom - top > 0.5) svgEl('path', { d: columnPath(cx - barWidth / 2, cx + barWidth / 2, top, bottom, pieceIndex === pieces.length - 1), fill: piece.fill }, svg);
        previousTop = top;
      });
      const hit = svgEl('rect', { x: margin.left + band * index, y: margin.top, width: band, height: height - margin.top - margin.bottom, class: 'col-hit', tabindex: bin.n ? '0' : '-1' }, svg);
      bindTip(hit, () => [durationLabel(bin), [
        { value: `${bin.wins} 场`, label: '胜', color: COLOR.meterWin, box: true },
        { value: `${bin.losses} 场`, label: '负', color: COLOR.meterLose, box: true },
      ]]);
    });
  }

  function renderBattles() {
    const { battles } = dashboard.analysisSource();
    renderRuleChart(battles);
    renderStreakChart(battles);
    renderKdChart(battles);
    renderDurationChart(battles);
  }

  // ---------- 武器 ----------
  const weaponView = { sort: { key: 'n', dir: 'desc' }, selected: null, rows: [], showPower: true };
  function renderWeapons() {
    const { battles, mode } = dashboard.analysisSource();
    weaponView.showPower = mode === 'all' || mode === 'anarchy';
    weaponView.rows = analysis.weaponRows(battles);
    if (!weaponView.rows.some((row) => row.key === weaponView.selected)) {
      weaponView.selected = analysis.sortRows(weaponView.rows, { key: 'n', dir: 'desc' })[0]?.key ?? null;
    }
    renderWeaponRanking();
    renderWeaponDetail();
    renderKitTable('sub-table', 'sub', '副武器', battles);
    renderKitTable('special-table', 'special', '特殊武器', battles);
  }
  function renderWeaponRanking() {
    const box = $('weapon-ranking');
    box.textContent = '';
    const { rows, showPower } = weaponView;
    if (!rows.length) { noBattles(box); return; }
    if (!showPower && weaponView.sort.key === 'power') weaponView.sort = { key: 'n', dir: 'desc' };
    const columns = [
      { label: '武器' },
      { label: '场次', sort: 'n', num: true },
      { label: '胜率', sort: 'wr' },
      { label: 'K/D', sort: 'kd', num: true },
      { label: '场均击倒', sort: 'avgK', num: true },
      { label: '场均死亡', sort: 'avgD', num: true },
      { label: '场均特殊', sort: 'avgSpecial', num: true },
      { label: '场均涂地', sort: 'avgPaint', num: true },
      ...(showPower ? [{ label: '挑战战力', sort: 'power', num: true }] : []),
      { label: '最近使用', sort: 'last', num: true },
    ];
    const table = el('table', { class: 'data' });
    table.append(el('thead', {}, sortHeader(columns, weaponView.sort, (next) => { weaponView.sort = next; renderWeaponRanking(); })));
    const body = el('tbody');
    for (const row of analysis.sortRows(rows, weaponView.sort)) {
      const cells = [
        records.iconWithName(row.image, row.name), String(row.n), meterCell(row.wr), fixed(row.kd, 2), fixed(row.avgK), fixed(row.avgD), fixed(row.avgSpecial), count(row.avgPaint),
        ...(showPower ? [row.power == null ? '—' : row.power.toFixed(1)] : []),
        fmtSince(row.last),
      ];
      const tr = el('tr', { class: row.key === weaponView.selected ? 'is-selected' : '', 'data-weapon': row.key, 'aria-label': `${row.name}，查看详情` });
      clickableRow(tr, () => selectWeapon(row.key));
      cells.forEach((cell, index) => {
        const td = el('td', { class: [columns[index].num ? 'num' : '', cell === '—' ? 'placeholder' : ''].filter(Boolean).join(' ') });
        if (cell instanceof Node) td.append(cell); else td.textContent = cell;
        tr.append(td);
      });
      body.append(tr);
    }
    table.append(body);
    box.append(table);
  }
  function selectWeapon(key) {
    weaponView.selected = key;
    for (const tr of document.querySelectorAll('#weapon-ranking tr[data-weapon]')) tr.classList.toggle('is-selected', tr.getAttribute('data-weapon') === key);
    renderWeaponDetail();
  }
  $('weapon-select').addEventListener('change', (event) => selectWeapon(event.target.value));
  function stageRanks(label, items) {
    const body = el('tbody', {}, items.map((item) => el('tr', {}, [
      el('td', { text: item.stage }),
      el('td', {}, meterCell(item.wr, 0)),
      el('td', { class: 'num font-number', text: `${item.n} 场` }),
    ])));
    return el('div', { class: 'stage-split' }, [el('p', { class: 'mini-label', text: label }), el('div', { class: 'scroll-x' }, el('table', { class: 'data compact' }, body))]);
  }
  function renderWeaponDetail() {
    const box = $('weapon-detail');
    const title = $('weapon-detail-title');
    const sub = $('weapon-detail-sub');
    const select = $('weapon-select');
    const rows = analysis.sortRows(weaponView.rows, { key: 'n', dir: 'desc' });
    const row = rows.find((item) => item.key === weaponView.selected);
    select.replaceChildren(...rows.map((item) => el('option', { value: item.key, text: item.name })));
    select.hidden = !row;
    box.textContent = '';
    if (!row) {
      title.textContent = '武器详情';
      sub.textContent = '';
      noBattles(box);
      return;
    }
    select.value = row.key;
    title.textContent = row.name;
    sub.textContent = [row.sub, row.special].filter(Boolean).join(' · ');
    const block = (label) => {
      const chart = el('div', { class: 'chart' });
      box.append(el('div', { class: 'chart-block' }, [el('p', { class: 'subhead', text: label }), chart]));
      return chart;
    };
    const trendBox = block('胜率（10 场滚动）');
    const powerBox = block('挑战战力');
    const stageBox = block('场地');

    const WINDOW = 10;
    const rolling = analysis.rollingWinRate(row.battles, WINDOW);
    if (!rolling.length) {
      emptyState(trendBox, `至少 ${WINDOW} 场后显示`, '扩大时间范围看看。');
    } else {
      const first = rolling[0].index;
      const last = rolling[rolling.length - 1].index;
      const xMax = last > first ? last : first + 1;
      drawLine(trendBox, {
        points: rolling.map((point) => ({ x: point.index, y: point.rate, title: `第 ${point.index} 场 · ${fmtDateTime(point.t)}`, rows: [
          { value: pct(point.rate, 0), label: `${WINDOW} 场胜率`, color: COLOR.winLine },
          { value: point.battle.win ? '胜' : '负', label: `${ruleName(point.battle.rule, point.battle.ruleName)} · ${point.battle.stage}` },
        ] })),
        color: COLOR.winLine, height: 168, yMin: 0, yMax: 1, ticks: [0, 0.5, 1], tickLabel: (tick) => `${tick * 100}%`, strongTick: (tick) => tick === 0.5,
        xMin: first, xMax, xLabels: [[first, `第 ${first} 场`], [xMax, `第 ${last} 场`]],
        area: true, endLabel: pct(rolling[rolling.length - 1].rate, 0), aria: `${row.name} ${WINDOW} 场滚动胜率，最新 ${pct(rolling[rolling.length - 1].rate, 0)}`,
      });
    }

    // 挑战战力只和同一把武器比较；按场次等距排列，连着打的几场不会挤成一条竖线。
    const challenge = row.battles.filter((battle) => battle.mode === 'anarchy' && battle.bankara === 'challenge' && battle.power != null);
    if (!challenge.length) {
      emptyState(powerBox, '没有蛮颓挑战记录', weaponView.showPower ? '这段时间没有用它打蛮颓挑战。' : '当前只看 X 比赛。');
    } else {
      const values = challenge.map((battle) => battle.power);
      const scale = niceScale(Math.min(...values) - 5, Math.max(...values) + 5, 3);
      const xMax = Math.max(2, challenge.length);
      drawLine(powerBox, {
        points: challenge.map((battle, index) => ({ x: index + 1, y: battle.power, title: fmtDateTime(battle.t), rows: [
          { value: battle.power.toFixed(1), label: '挑战战力', color: modeColor('anarchy') },
          { value: battle.win ? '胜' : battle.lose ? '负' : '—', label: battle.delta ? `变化 ${signed(battle.delta)}` : battle.stage },
        ] })),
        color: modeColor('anarchy'), height: 168, yMin: scale.min, yMax: scale.max, ticks: scale.ticks,
        xMin: 1, xMax, xLabels: [[1, fmtShortDate(challenge[0].t)], [xMax, fmtShortDate(challenge[challenge.length - 1].t)]],
        endLabel: values[values.length - 1].toFixed(1), aria: `${row.name}的挑战战力，当前 ${values[values.length - 1].toFixed(1)}`, marginLeft: 46, marginRight: 58,
      });
    }

    const extremes = analysis.stageExtremes(row.battles);
    if (!extremes) emptyState(stageBox, '场地样本不足', '每张图至少 3 场分出胜负后比较。');
    else stageBox.append(stageRanks('最好', extremes.best), stageRanks('最差', extremes.worst));
  }
  function renderKitTable(boxId, key, label, list) {
    const box = $(boxId);
    const rows = analysis.kitRows(list, key);
    if (!rows.length) { noBattles(box); return; }
    const special = key === 'special';
    renderTable(box, [
      { label },
      { label: '场次', num: true },
      { label: '胜率' },
      special ? { label: '场均使用', num: true } : { label: 'K/D', num: true },
      { label: '主武器', wrap: true },
    ], rows.map((row) => [
      records.iconWithName(row.image, row.name, 'kit-icon'),
      String(row.n),
      meterCell(row.wr),
      special ? (row.avgSpecial == null ? '—' : `${fixed(row.avgSpecial)} 次`) : fixed(row.kd, 2),
      row.weapons.join('、'),
    ]));
  }

  // ---------- 场地 ----------
  function renderStages() {
    const { battles, start, end } = dashboard.analysisSource();
    const rows = analysis.stageRows(battles);
    const labelWidth = Math.min(160, Math.ceil(Math.max(0, ...rows.map((row) => textWidth(row.stage, 13)))) + 14);
    renderStageRanking(rows, labelWidth);
    renderStageTimeline(rows, start, end, labelWidth);
    renderStageWeapon(battles);
  }
  function renderStageRanking(rows, labelWidth) {
    const box = $('stage-ranking');
    if (!rows.length) { noBattles(box); return; }
    const ordered = analysis.rankStages(rows);
    if (dashboard.tableOn('stageRank')) {
      renderTable(box, [{ label: '场地' }, { label: '场次', num: true }, { label: '胜率', num: true }, { label: 'K/D', num: true }, { label: '场均涂地', num: true }, { label: '最近一次', num: true }], ordered.map((row) => [
        row.stage, String(row.n), enough(row) ? pct(row.wr) : '样本不足', fixed(row.kd, 2), count(row.avgPaint), fmtSince(row.last),
      ]));
      return;
    }
    box.textContent = '';
    const width = Math.max(280, Math.floor(box.clientWidth));
    const rightWidth = 92;
    const rowHeight = 26;
    const barHeight = 14;
    const top = 4;
    const plotWidth = width - labelWidth - rightWidth;
    const height = top + ordered.length * rowHeight + 22;
    const eligible = ordered.filter(enough);
    const deviation = Math.max(0.2, Math.ceil(Math.max(0, ...eligible.map((row) => Math.abs(row.wr - 0.5))) * 10 - 1e-9) / 10);
    const x = (value) => labelWidth + ((Math.max(0.5 - deviation, Math.min(0.5 + deviation, value)) - (0.5 - deviation)) / (2 * deviation)) * plotWidth;
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `各场地胜率：${eligible.map((row) => `${row.stage} ${pct(row.wr, 0)}`).join('，')}` }, box);
    const step = plotWidth / (deviation * 20) >= 34 ? 0.1 : 0.2;
    for (let offset = -Math.floor(deviation / step + 1e-9) * step; offset <= deviation + 1e-9; offset += step) {
      const tick = 0.5 + offset;
      const middle = Math.abs(offset) < 1e-9;
      svgEl('line', { x1: x(tick), x2: x(tick), y1: top, y2: top + ordered.length * rowHeight, stroke: middle ? COLOR.ink3 : COLOR.line, 'stroke-width': 1, 'stroke-dasharray': middle ? '3 3' : 'none' }, svg);
      svgText(svg, x(tick), height - 6, `${Math.round(tick * 100)}%`, { 'text-anchor': 'middle', class: 'font-number' });
    }
    ordered.forEach((row, index) => {
      const cy = top + index * rowHeight + rowHeight / 2;
      svgText(svg, 0, cy + 5, row.stage, { fill: COLOR.ink2, 'font-size': 13 });
      if (enough(row)) {
        svgEl('path', { d: hbarPath(x(0.5), x(row.wr), cy - barHeight / 2, cy + barHeight / 2), fill: row.wr >= 0.5 ? COLOR.meterWin : COLOR.meterLose }, svg);
        svgText(svg, width - rightWidth + 10, cy + 5, pct(row.wr, 0), { fill: COLOR.ink, 'font-size': 13, class: 'font-number' });
      } else {
        svgText(svg, width - rightWidth + 10, cy + 5, '样本不足', {});
      }
      svgText(svg, width, cy + 5, `${row.n} 场`, { 'text-anchor': 'end', class: 'font-number' });
      bindTip(rowHit(svg, cy - rowHeight / 2, width, rowHeight), () => [row.stage, [
        { value: enough(row) ? pct(row.wr) : '样本不足', label: `${row.wins} 胜 ${row.losses} 负`, color: enough(row) ? (row.wr >= 0.5 ? COLOR.meterWin : COLOR.meterLose) : COLOR.seq0, box: true },
        ...analysis.ruleBreakdown(row.games).map((item) => ({ value: enough(item) ? pct(item.wr, 0) : '—', label: `${ruleName(item.rule, item.ruleName)} · ${item.n} 场` })),
      ]]);
    });
  }
  function renderStageTimeline(rows, start, end, labelWidth) {
    const box = $('stage-timeline');
    const legend = $('stage-timeline-legend');
    legend.textContent = '';
    if (!rows.length) { noBattles(box); return; }
    legend.append(legendSwatch(COLOR.meterWin, '胜'), legendSwatch(COLOR.meterLose, '负'));
    const ordered = [...rows].sort((a, b) => b.n - a.n || b.last - a.last);
    if (dashboard.tableOn('stageTime')) {
      renderTable(box, [{ label: '场地' }, { label: '场次', num: true }, { label: '最近一次', number: true }], ordered.map((row) => [row.stage, String(row.n), fmtDateTime(row.last)]));
      return;
    }
    box.textContent = '';
    const width = Math.max(260, Math.floor(box.clientWidth));
    const rightWidth = 52;
    const rowHeight = 26;
    const trackHeight = 16;
    const top = 4;
    const plotWidth = width - labelWidth - rightWidth;
    const height = top + ordered.length * rowHeight + 22;
    const x = (t) => labelWidth + ((t - start) / Math.max(1, end - start)) * plotWidth;
    const svg = svgEl('svg', { width, height, viewBox: `0 0 ${width} ${height}`, role: 'img', 'aria-label': `各场地的出现时间：${ordered.map((row) => `${row.stage} ${row.n} 场`).join('，')}` }, box);
    ordered.forEach((row, index) => {
      const cy = top + index * rowHeight + rowHeight / 2;
      svgText(svg, 0, cy + 5, row.stage, { fill: COLOR.ink2, 'font-size': 13 });
      svgEl('rect', { x: labelWidth, y: cy - trackHeight / 2, width: plotWidth, height: trackHeight, rx: 4, fill: COLOR.seq0 }, svg);
      for (const battle of row.games) {
        const fill = battle.win ? COLOR.meterWin : battle.lose ? COLOR.meterLose : COLOR.context;
        svgEl('rect', { x: Math.max(labelWidth, Math.min(labelWidth + plotWidth - 2, x(battle.t) - 1)), y: cy - trackHeight / 2 + 2, width: 2, height: trackHeight - 4, fill }, svg);
      }
      svgText(svg, width, cy + 5, `${row.n} 场`, { 'text-anchor': 'end', class: 'font-number' });
      bindTip(rowHit(svg, cy - rowHeight / 2, width, rowHeight), () => [row.stage, [
        { value: `${row.n} 场`, label: `${row.wins} 胜 ${row.losses} 负` },
        { value: fmtDateTime(row.last), label: '最近一次' },
      ]]);
    });
    const axisY = height - 6;
    svgText(svg, labelWidth, axisY, fmtShortDate(start), { 'text-anchor': 'start', class: 'font-number' });
    svgText(svg, labelWidth + plotWidth / 2, axisY, fmtShortDate(start + (end - start) / 2), { 'text-anchor': 'middle', class: 'font-number' });
    svgText(svg, labelWidth + plotWidth, axisY, fmtShortDate(end), { 'text-anchor': 'end', class: 'font-number' });
  }
  function renderStageWeapon(list) {
    const box = $('stage-weapon');
    const legend = $('stage-weapon-legend');
    legend.textContent = '';
    if (!list.length) { noBattles(box); return; }
    legend.append(...divergingLegend('低于 50%', '高于 50%', '少于 3 场'));
    const matrix = analysis.stageWeaponMatrix(list);
    const table = el('table', { class: 'data matrix sticky-first' });
    table.append(el('thead', {}, el('tr', {}, [
      el('th', { scope: 'col', text: '场地' }),
      ...matrix.weapons.map((name) => el('th', { scope: 'col', class: 'center', text: name })),
      el('th', { scope: 'col', text: '最佳武器' }),
    ])));
    const body = el('tbody');
    for (const row of matrix.rows) {
      const tr = el('tr', {}, el('td', { text: row.stage }));
      row.cells.forEach((cell, index) => {
        tr.append(matrixCell({ n: cell.n, rate: cell.wr, fill: enough(cell) ? divergingColor(cell.wr) : null, unit: '场', title: `${row.stage} · ${matrix.weapons[index]}`, detail: `${cell.wins} 胜 ${cell.losses} 负` }));
      });
      tr.append(row.best
        ? el('td', {}, [document.createTextNode(`${row.best.weapon} `), el('span', { class: 'muted font-number', text: pct(row.best.wr, 0) })])
        : el('td', { class: 'placeholder', text: '—' }));
      body.append(tr);
    }
    table.append(body);
    box.replaceChildren(table);
  }

  // ---------- 鲑鱼跑 ----------
  const shiftResult = (shift) => (shift.cleared ? '通关' : `第 ${shift.failedWave ?? '?'} 波失败`);
  const gradeText = (name, point) => `${name} ${point ?? ''}`.trim();
  function renderGradeChart(list, start, end) {
    const box = $('grade-chart');
    const graded = list.filter((shift) => analysis.gradeLadder(shift) != null);
    if (!list.length) { noShifts(box); return; }
    if (!graded.length) { emptyState(box, '没有评级记录', '换一个打工模式看看。'); return; }
    if (dashboard.tableOn('grade')) {
      renderTable(box, [{ label: '时间', number: true }, { label: '场地' }, { label: '结果' }, { label: '评级', num: true }], [...graded].reverse().map((shift) => [
        fmtDateTime(shift.t), shift.stage, shiftResult(shift), gradeText(shift.gradeName, shift.gradePoint),
      ]));
      return;
    }
    const values = graded.map((shift) => analysis.gradeLadder(shift));
    const scale = niceScale(Math.min(...values) - 10, Math.max(...values) + 10, 4);
    // 最高段位的评级点到 999 为止，纵轴不画到它上面。
    const ladderTop = analysis.GRADES.length * 1000 - 1;
    const ticks = scale.ticks.filter((tick) => tick >= 0 && tick <= ladderTop);
    const yMin = Math.max(0, scale.min);
    const yMax = Math.min(scale.max, ladderTop);
    if (yMax === ladderTop && ladderTop - ticks[ticks.length - 1] >= (scale.ticks[1] - scale.ticks[0]) / 2) ticks.push(ladderTop);
    const last = graded[graded.length - 1];
    const endText = gradeText(last.gradeName, last.gradePoint);
    const parts = box.clientWidth < 480 ? 2 : 4;
    drawLine(box, {
      points: graded.map((shift, index) => ({ x: shift.t, y: values[index], title: fmtDateTime(shift.t), rows: [
        { value: gradeText(shift.gradeName, shift.gradePoint), label: '评级', color: COLOR.salmon },
        { value: shiftResult(shift), label: shift.stage },
      ] })),
      color: COLOR.salmon, height: 248, yMin, yMax, ticks, tickLabel: analysis.gradeLabel, strongTick: (tick) => tick % 1000 === 0,
      xMin: start, xMax: end, xLabels: Array.from({ length: parts + 1 }, (_, index) => { const t = start + ((end - start) * index) / parts; return [t, fmtDate(t)]; }),
      endLabel: endText, aria: `评级走势，当前 ${endText}`,
      marginLeft: Math.ceil(Math.max(...ticks.map((tick) => textWidth(analysis.gradeLabel(tick), 12)))) + 12,
      marginRight: Math.ceil(textWidth(endText, 13)) + 16,
    });
  }
  function renderKings(list) {
    const box = $('king-list');
    const scalesBox = $('king-scales');
    box.textContent = '';
    scalesBox.textContent = '';
    const rows = analysis.kingRows(list);
    scalesBox.hidden = !rows.length;
    if (!rows.length) { emptyState(box, '这段时间没有遇到头目鲑鱼', '换一个时间范围看看。'); return; }
    for (const row of rows) {
      const rate = row.defeated / row.appeared;
      box.append(el('div', { class: 'king-row' }, [
        row.image ? el('img', { src: row.image, alt: '', width: '48', height: '48' }) : el('span', { class: 'king-placeholder', 'aria-hidden': 'true' }),
        el('div', { class: 'king-body' }, [
          el('div', { class: 'king-meta' }, [el('strong', { text: row.name ?? '头目鲑鱼' }), el('span', { class: 'font-number', text: pct(rate, 0) })]),
          el('div', { class: 'king-bar', role: 'img', 'aria-label': `击倒 ${row.defeated} 次，出现 ${row.appeared} 次` }, el('span', { style: `width:${(rate * 100).toFixed(1)}%` })),
          el('div', { class: 'king-foot font-number', text: `击倒 ${row.defeated} / 出现 ${row.appeared}` }),
        ]),
      ]));
    }
    scalesBox.append(el('span', { text: '鳞片' }), ...records.scaleCounts(analysis.scaleTotals(list)));
  }
  function renderTideEvents(list) {
    const box = $('tide-event');
    const legend = $('tide-event-legend');
    legend.textContent = '';
    const matrix = analysis.tideEventMatrix(list);
    if (!matrix.total) { noShifts(box); return; }
    const table = el('table', { class: 'data matrix sticky-first' });
    table.append(el('thead', {}, el('tr', {}, [el('th', { scope: 'col', text: '特殊波次' }), ...WATER.map((water) => el('th', { scope: 'col', class: 'center', text: water }))])));
    const body = el('tbody');
    for (const row of matrix.rows) {
      const tr = el('tr', {}, el('td', { text: row.event ?? '无' }));
      for (const cell of row.cells) {
        tr.append(matrixCell({
          n: cell.n, rate: cell.rate, fill: cell.n >= analysis.MIN_DECIDED ? divergingColor(cell.rate - matrix.average + 0.5, COLOR.salmonDiv) : null, unit: '波',
          title: `${row.event ?? '无特殊波次'} · ${cell.water}`, detail: `通关 ${cell.cleared} / ${cell.n} 波`,
        }));
      }
      body.append(tr);
    }
    table.append(body);
    box.replaceChildren(table);
    legend.append(...divergingLegend('低于平均', `高于平均（${pct(matrix.average, 0)}）`, '少于 3 波', COLOR.salmonDiv));
  }
  const CONTRIBUTION = [
    { key: 'goldenEggs', label: '金鲑鱼卵', unit: '个', step: 2, cap: 40, every: 5 },
    { key: 'goldenAssists', label: '协助', unit: '个', step: 1, cap: 15, every: 5 },
    { key: 'rescues', label: '救援', unit: '次', step: 1, cap: 8, every: 2 },
    { key: 'rescued', label: '被救', unit: '次', step: 1, cap: 8, every: 2 },
  ];
  function renderContribution(list) {
    const box = $('contrib-chart');
    if (!list.length) { noShifts(box); return; }
    const metrics = CONTRIBUTION.map((metric) => ({ ...metric, result: analysis.histogram(list.map((shift) => shift[metric.key]), metric) }));
    if (dashboard.tableOn('contrib')) {
      renderTable(box, [{ label: '项目' }, { label: '场均', num: true }, { label: '中位数', num: true }, { label: '最多', num: true }], metrics.map((metric) => {
        const values = list.map((shift) => shift[metric.key]).filter((value) => value != null);
        return [metric.label, values.length ? fixed(values.reduce((sum, value) => sum + value, 0) / values.length) : '—', fmtMedian(metric.result.median), values.length ? String(Math.max(...values)) : '—'];
      }));
      return;
    }
    box.textContent = '';
    const charts = metrics.map(() => el('div', { class: 'chart' }));
    box.append(el('div', { class: 'chart-pair' }, metrics.map((metric, index) => el('div', { class: 'chart-block' }, [el('p', { class: 'subhead', text: metric.label }), charts[index]]))));
    metrics.forEach((metric, index) => {
      const { result } = metric;
      if (!result.total) { emptyState(charts[index], '没有数据', '旧记录可能没有这一项。'); return; }
      drawHistogram(charts[index], {
        bins: histogramBins(result, { step: metric.step, label: metric.label, unit: metric.unit }),
        color: COLOR.salmon, height: 132, labelEvery: metric.every,
        marker: { position: result.medianPosition, label: `中位数 ${fmtMedian(result.median)}` },
        aria: `每场${metric.label}分布，中位数 ${fmtMedian(result.median)}`,
      });
    });
  }
  function suppliedCell(weapons) {
    if (!weapons.length) return '—';
    return el('span', { class: 'weapon-icons' }, weapons.map((weapon) => records.img(weapon.image, 'weapon-icon', weapon.name ?? '') || el('span', { class: 'supplied-name', text: weapon.name ?? '?' })));
  }
  const ROTATION_PAGE_SIZE_KEY = 'splat3-rotation-page-size';
  const ROTATION_PAGE_SIZES = [5, 10, 15];
  let rotationPageSize = 5;
  try {
    const saved = Number(localStorage.getItem(ROTATION_PAGE_SIZE_KEY));
    if (ROTATION_PAGE_SIZES.includes(saved)) rotationPageSize = saved;
  } catch {}
  let rotationPage = 0;
  let rotationSource = '';
  function renderRotations(list) {
    const box = $('rotation-table');
    const pager = $('rotation-pager');
    const groups = analysis.rotationGroups(list);
    const source = groups.map((group) => `${group.start ?? group.first}:${group.end ?? group.last}:${group.rule}:${group.stage}`).join('|');
    if (source !== rotationSource) { rotationPage = 0; rotationSource = source; box.scrollTop = 0; }
    pager.replaceChildren();
    if (!groups.length) { noShifts(box); return; }
    const pages = Math.ceil(groups.length / rotationPageSize);
    rotationPage = Math.min(rotationPage, pages - 1);
    const first = rotationPage * rotationPageSize;
    const previous = el('button', { type: 'button', class: 'ghost', text: '上一页' });
    const next = el('button', { type: 'button', class: 'ghost', text: '下一页' });
    previous.disabled = rotationPage === 0;
    next.disabled = rotationPage === pages - 1;
    const changePage = (delta) => {
      rotationPage += delta;
      renderRotations(list);
      box.scrollTop = 0;
    };
    previous.addEventListener('click', () => changePage(-1));
    next.addEventListener('click', () => changePage(1));
    const pageSize = el('select', { class: 'card-select', 'aria-label': '每页打工期数' }, ROTATION_PAGE_SIZES.map((size) => {
      const option = el('option', { value: String(size), text: String(size) });
      option.selected = size === rotationPageSize;
      return option;
    }));
    pageSize.addEventListener('change', () => {
      const value = Number(pageSize.value);
      if (!ROTATION_PAGE_SIZES.includes(value)) {
        pageSize.value = String(rotationPageSize);
        return;
      }
      rotationPageSize = value;
      rotationPage = 0;
      try { localStorage.setItem(ROTATION_PAGE_SIZE_KEY, String(value)); } catch {}
      renderRotations(list);
      box.scrollTop = 0;
    });
    pager.replaceChildren(
      el('label', { class: 'rotation-page-size' }, [el('span', { text: `共 ${groups.length} 期 · 每页` }), pageSize, document.createTextNode('期')]),
      el('span', { class: 'log-pager-buttons' }, [previous, el('span', { class: 'font-number', 'aria-live': 'polite', text: `${rotationPage + 1} / ${pages}` }), next]),
    );
    renderTable(box, [
      { label: '时间', number: true },
      { label: '场地' },
      { label: '发放武器' },
      { label: '场次', num: true },
      { label: '通关率', num: true },
      { label: '评级', num: true },
      { label: '头目鲑鱼' },
      { label: '鳞片' },
    ], groups.slice(first, first + rotationPageSize).map((group) => {
      const range = group.start != null ? [group.start, group.end] : [group.first, group.last];
      const hasScales = group.scales.gold || group.scales.silver || group.scales.bronze;
      return [
        `${fmtDateTime(range[0])} – ${fmtDateTime(range[1])}`,
        // 鲑鱼跑以外的打工模式（大型跑、团队打工竞赛）在场地后面标出来。
        el('span', { class: 'rotation-stage' }, [
          document.createTextNode(group.stage),
          group.ongoing ? el('span', { class: 'rotation-ongoing', role: 'img', 'aria-label': '进行中', title: '进行中' }) : null,
          group.rule && group.rule !== 'REGULAR' ? el('span', { class: 'pill', text: COOP_RULE_LABEL[group.rule] ?? group.rule }) : null,
        ]),
        suppliedCell(group.supplied),
        String(group.n),
        pct(group.cleared / group.n, 0),
        group.grade ? gradeText(group.grade.name, group.grade.point) : '—',
        group.king ? el('span', { class: 'icon-name', title: `${group.king.name ?? '头目鲑鱼'} · 击倒 ${group.king.defeated} / 出现 ${group.king.appeared}` }, [
          group.king.image
            ? el('img', { class: 'enemy-icon', src: group.king.image, alt: group.king.name ?? '头目鲑鱼', loading: 'lazy' })
            : document.createTextNode(group.king.name ?? '头目鲑鱼'),
          el('span', { class: 'font-number', text: `${group.king.defeated} / ${group.king.appeared}` }),
        ]) : '—',
        hasScales ? el('span', { class: 'scale-inline' }, records.scaleCounts(group.scales)) : '—',
      ];
    }));
  }
  // 击倒数量：SplatNet「鲑鱼跑记录」里的累计数，和 SplatNet 一样分巨大鲑鱼、头目鲑鱼两组。
  // 不受时间范围和打工模式筛选影响；还没查到时不显示这张卡片。
  // 两种显示方式：图鉴格子，或按目标画的排行条（analysis.defeatBars），选择记在本机。
  const DEFEAT_VIEW_KEY = 'splat3-defeat-view';
  let defeatView = 'bars';
  try {
    if (localStorage.getItem(DEFEAT_VIEW_KEY) === 'grid') defeatView = 'grid';
  } catch {}
  $('defeat-view').addEventListener('click', (event) => {
    const button = event.target.closest('button[data-defeat-view]');
    if (!button) return;
    defeatView = button.getAttribute('data-defeat-view');
    try {
      localStorage.setItem(DEFEAT_VIEW_KEY, defeatView);
    } catch {}
    renderDefeats();
  });
  const defeatIcon = (row) => el('span', { class: 'defeat-icon' }, row.image
    ? records.img(row.image, '', row.name)
    : analysis.KING_IMAGES[row.id] ? el('img', { src: analysis.KING_IMAGES[row.id], alt: row.name ?? '' }) : null);
  function defeatGridView(rows) {
    return el('div', { class: 'defeat-grid' }, rows.map((row) => el('div', { class: `defeat-tile${row.count ? '' : ' is-zero'}` }, [
      defeatIcon(row),
      el('div', { class: 'defeat-text' }, [el('span', { text: row.name ?? '—' }), el('strong', { text: count(row.count) })]),
    ])));
  }
  // 排行条分两列，先读完左列；每列顶上标出目标的位置，横条上的锚线也画在那里。
  function defeatBarView(rows, target) {
    const bars = analysis.defeatBars(rows, target);
    const percent = (share) => `${(share * 100).toFixed(2)}%`;
    const anchorLeft = `min(calc(${percent(bars.anchor)} - 1px), calc(100% - 2px))`;
    const column = (list) => el('div', { class: 'defeat-bar-col' }, [
      el('div', { class: 'defeat-bar-axis', 'aria-hidden': 'true' }, [
        el('span'), el('span'),
        el('span', { class: 'defeat-bar-scale' }, el('span', { class: `defeat-bar-mark${bars.anchor > 0.9 ? ' is-end' : ''}`, style: `left:${percent(bars.anchor)}`, text: count(bars.target) })),
        el('span'),
      ]),
      ...list.map((row) => el('div', { class: 'defeat-bar-row' }, [
        defeatIcon(row),
        el('span', { class: 'defeat-bar-name', text: row.name ?? '—' }),
        row.share == null ? el('span') : el('span', { class: 'defeat-bar', role: 'img', 'aria-label': `${count(row.count)} / ${count(bars.target)}` }, [
          el('span', { class: 'defeat-bar-fill', style: `width:${percent(row.share)}` }),
          el('span', { class: 'defeat-bar-anchor', style: `left:${anchorLeft}` }),
        ]),
        el('strong', { text: count(row.count) }),
      ])),
    ]);
    const half = Math.ceil(bars.rows.length / 2);
    return el('div', { class: 'defeat-bars' }, [bars.rows.slice(0, half), bars.rows.slice(half)].filter((list) => list.length).map(column));
  }
  function renderDefeats() {
    const record = dashboard.coopRecord();
    const box = $('defeat-list');
    $('defeat-card').hidden = !record;
    if (!record) return;
    $('defeat-sub').title = record.fetchedAt ? `${fmtDateTime(record.fetchedAt)} 更新` : '';
    for (const button of $('defeat-view').querySelectorAll('button[data-defeat-view]')) button.setAttribute('aria-pressed', String(button.getAttribute('data-defeat-view') === defeatView));
    const groups = [
      ['巨大鲑鱼', record.enemies, analysis.DEFEAT_TARGETS.enemies],
      ['头目鲑鱼', record.bosses, analysis.DEFEAT_TARGETS.bosses],
    ].filter(([, rows]) => rows.length);
    if (!groups.length) { emptyState(box, '还没有击倒记录', '打工之后再来看看。'); return; }
    box.replaceChildren(...groups.map(([label, rows, target]) => el('div', { class: 'defeat-group' }, [
      el('p', { class: 'subhead', text: label }),
      defeatView === 'bars' ? defeatBarView(rows, target) : defeatGridView(rows),
    ])));
  }
  function renderSalmon() {
    const { shifts, start, end } = dashboard.analysisSource();
    renderGradeChart(shifts, start, end);
    renderKings(shifts);
    renderTideEvents(shifts);
    renderContribution(shifts);
    renderRotations(shifts);
    renderDefeats();
  }

  const RENDERERS = { battles: renderBattles, weapons: renderWeapons, stages: renderStages, salmon: renderSalmon };
  window.Splat3Analysis = Object.freeze({
    VIEWS: Object.keys(RENDERERS),
    render(view) {
      COLOR = dashboard.ui.colors();
      hideTip();
      RENDERERS[view]?.();
    },
  });
})();
