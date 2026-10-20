(() => {
  'use strict';
  const RULES = [
    { key: 'AREA', name: '真格区域', color: 'var(--rule-1)' },
    { key: 'LOFT', name: '真格塔楼', color: 'var(--rule-2)' },
    { key: 'GOAL', name: '真格鱼虎', color: 'var(--rule-3)' },
    { key: 'CLAM', name: '真格蛤蜊', color: 'var(--rule-4)' },
  ];
  const RULE = Object.fromEntries(RULES.map((rule) => [rule.key, rule]));
  const CONTENT = { ranked: '真格', salmon: '打工' };
  const KIND = { month: '月度总结', season: '赛季总结' };
  const KINGS = [
    { id: 23, name: '横纲', image: 'images/boss1_color.png', silhouette: 'images/boss1.png' },
    { id: 24, name: '辰龙', image: 'images/boss2_color.png', silhouette: 'images/boss2.png' },
    { id: 25, name: '巨颚', image: 'images/boss3_color.png', silhouette: 'images/boss3.png' },
    { id: 30, name: '头目联合', image: 'images/boss_all_color.png', silhouette: 'images/boss_all.png' },
  ];
  const GRADES = ['新手', '半吊子', '独当一面', '熟练', '达人', '达人+1', '达人+2', '达人+3', '传说'];
  const gradeLabel = (value) => `${GRADES[Math.floor(value / 1000)]} ${Math.round(value % 1000)}`;

  const SVG = 'http://www.w3.org/2000/svg';
  function el(tag, props = {}, children = []) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (key === 'text') node.textContent = value;
      else if (key === 'style') node.style.cssText = value;
      else node.setAttribute(key, value);
    }
    for (const child of [].concat(children)) if (child != null) node.append(child);
    return node;
  }
  function svg(tag, attrs = {}, text) {
    const node = document.createElementNS(SVG, tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    if (text != null) node.textContent = text;
    return node;
  }
  const fixed1 = (value) => value == null ? '—' : value.toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
  const grouped = (value) => value == null ? '—' : value.toLocaleString('en-US');
  const pct = (value) => value == null ? '—' : `${fixed1(value)}%`;
  const integerPct = (value) => value == null ? '—' : `${Math.round(value)}%`;
  const deltaText = (value) => value == null || Math.abs(value) < 0.00001 ? '' : signed(value);
  const signed = (value) => (value >= 0 ? `+${fixed1(value)}` : `−${fixed1(-value)}`);
  const signedInt = (value) => (value >= 0 ? `+${grouped(value)}` : `−${grouped(-value)}`);
  let uid = 0;

  function card(title, dotColor, body, extra) {
    const heading = el('h3', {}, [dotColor ? el('span', { class: 'mode-dot', style: `background:${dotColor}` }) : null, title]);
    return el('section', { class: 's-card' }, [el('div', { class: 's-card-head' }, [heading, extra || null]), body]);
  }
  function row(className, cards) {
    return el('div', { class: `s-row ${className}` }, cards);
  }
  function tile(label, value, unit, accent, icon) {
    return el('div', { class: accent ? 's-tile is-accent' : 's-tile' }, [
      el('span', { class: 'label' }, [icon ? el('img', { src: icon, alt: '' }) : null, label]),
      el('span', { class: 'value' }, [value, unit ? el('span', { class: 'unit', text: unit }) : null]),
    ]);
  }
  // 两段的条：a 是胜或通关，b 是负或失败。
  function splitBar(rate, label) {
    if (rate == null) return el('div', { class: 'bar' });
    return el('div', { class: 'bar', role: 'img', 'aria-label': label }, [
      el('span', { class: 'a', style: `flex:${rate}` }),
      rate < 100 ? el('span', { class: 'b', style: `flex:${100 - rate}` }) : null,
    ]);
  }
  function highlights(list) {
    return card('亮点', null, el('div', { class: 'highlights' }, list.map(([label, value, note]) => el('div', { class: 'hl' }, [
      el('span', { class: 'label', text: label }),
      el('span', { class: 'value' }, el('span', { class: 'hl-text' }, [
        value, note ? el('small', { text: note }) : null,
      ])),
    ]))));
  }

  function fitHighlights(node) {
    for (const value of node.querySelectorAll('.hl .value')) {
      value.style.fontSize = '26px';
      const width = value.clientWidth - 1;
      if (width <= 0) continue;
      const text = value.querySelector('.hl-text');
      // 小字也按 em 同步缩小；先估算比例，再处理像素取整造成的微小溢出。
      let size = Math.max(1, Math.min(26, Math.floor(26 * width / text.offsetWidth * 2) / 2));
      value.style.fontSize = `${size}px`;
      while (text.offsetWidth > width && size > 1) {
        size -= 0.5;
        value.style.fontSize = `${size}px`;
      }
    }
  }

  // ---------- 真格 ----------

  function rankedHero(data) {
    return el('div', { class: 's-hero' }, [
      tile('对战', grouped(data.battles), '场'),
      tile('胜率', fixed1(data.winRate), data.winRate == null ? null : '%', true),
      tile('K/D', data.kd == null ? '—' : data.kd.toFixed(2)),
      tile('游戏时长', fixed1(data.hours), data.hours == null ? null : '小时'),
    ]);
  }

  // 每天的真格对战：周一开头的月历，颜色越亮场次越多。
  function calendar(data) {
    const { counts, offset, levels, month } = data;
    const cells = ['一', '二', '三', '四', '五', '六', '日'].map((day) => el('span', { class: 'wd', text: day }));
    for (let index = 0; index < offset; index += 1) cells.push(el('span'));
    counts.forEach((count, index) => {
      const level = levels[index];
      cells.push(el('span', {
        class: 'day',
        style: `background:var(--seq-${level});color:var(--day-${level})`,
        title: `${month} 月 ${index + 1} 日：${count} 场`,
        text: String(index + 1),
      }));
    });
    const scale = el('span', { class: 'scale' }, ['少', ...[0, 1, 2, 3, 4, 5].map((level) => el('i', { style: `background:var(--seq-${level})` })), '多']);
    return card('每天的对战', null, el('div', { class: `cal${offset + counts.length > 35 ? ' is-six-weeks' : ''}` }, cells), scale);
  }

  function ruleBars(rules) {
    return card('规则胜率', null, el('div', { class: 'bars' }, rules.map(([key, rate, games]) => el('div', { class: 'bar-row' }, [
      el('div', { class: 'bar-head' }, [
        el('span', { text: RULE[key].name }),
        el('span', {}, [el('span', { class: 'pct', text: pct(rate) }), el('span', { class: 'n', text: `${games} 场` })]),
      ]),
      splitBar(rate, `胜 ${fixed1(rate)}%`),
    ]))));
  }

  function xList(xPower, peak) {
    const rows = RULES.map((rule) => {
      const { power, delta } = xPower[rule.key];
      return el('div', { class: 'xrow' }, [
        el('span', { class: 'mode-dot', style: `background:${rule.color}` }),
        el('span', { class: 'name', text: rule.name }),
        el('span', { class: 'power', text: fixed1(power) }),
        el('span', { class: `delta ${delta >= 0 ? 'up' : 'down'}`, text: deltaText(delta) }),
      ]);
    });
    return card('X 战力', 'var(--mode-x)', el('div', { class: 'xlist' }, rows), peak ? el('span', { class: 'head-note', text: `最高 ${fixed1(peak.value)} · ${RULE[peak.rule].name}` }) : null);
  }

  // 本赛季 X 战力走势：四条线共用一个纵轴，线尾标出最新的值。
  function xTrend(data, period) {
    const width = 896;
    const height = 214;
    const left = 70;
    const right = 112;
    const top = 12;
    const bottom = 36;
    const values = Object.values(data.xPower).flatMap((track) => track.points.map((point) => point.value));
    const tMin = period.start;
    const tMax = Math.max(tMin + 1, period.until);
    const vMin = values.length ? Math.floor(Math.min(...values) / 100) * 100 : 0;
    const vMax = Math.max(vMin + 100, values.length ? Math.ceil(Math.max(...values) / 100) * 100 : 100);
    const x = (time) => left + ((time - tMin) / (tMax - tMin)) * (width - left - right);
    const y = (value) => height - bottom - ((value - vMin) / (vMax - vMin)) * (height - top - bottom);
    const chart = svg('svg', { class: 'chart', viewBox: `0 0 ${width} ${height}`, width, height, role: 'img', 'aria-label': '本赛季各规则 X 战力走势' });
    if (!values.length) chart.append(svg('text', { x: width / 2, y: height / 2, fill: 'var(--ink-3)', 'font-size': 32, 'text-anchor': 'middle' }, '—'));
    for (let value = vMin; values.length && value <= vMax; value += Math.max(100, Math.ceil((vMax - vMin) / 500) * 100)) {
      chart.append(svg('line', { x1: left, x2: width - right, y1: y(value), y2: y(value), stroke: 'var(--line)', 'stroke-width': 2 }));
      chart.append(svg('text', { x: left - 14, y: y(value) + 7, 'text-anchor': 'end', fill: 'var(--ink-3)', 'font-size': 20 }, grouped(value)));
    }
    for (let index = 0; index < 4; index += 1) {
      const time = tMin + (tMax - tMin) * index / 3;
      const date = new Date(time);
      chart.append(svg('text', { x: x(time), y: height - 6, 'text-anchor': 'middle', fill: 'var(--ink-3)', 'font-size': 20 }, `${date.getUTCMonth() + 1}/${date.getUTCDate()}`));
    }
    const ends = [];
    for (const rule of RULES) {
      const track = data.xPower[rule.key];
      if (!track.points.length) continue;
      const points = track.points.map((point) => [x(point.t), y(point.value)]);
      // 最后一次实测之后保持最后的战力，统一延伸到截至时间，圆点和标签对齐。
      points.push([x(tMax), y(track.power)]);
      chart.append(svg('path', { d: points.map(([px, py], index) => `${index ? 'L' : 'M'}${px.toFixed(1)},${py.toFixed(1)}`).join(' '), fill: 'none', stroke: rule.color, 'stroke-width': 4, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }));
      const [ex, ey] = points[points.length - 1];
      chart.append(svg('circle', { cx: ex, cy: ey, r: 7, fill: rule.color, stroke: 'var(--card-solid)', 'stroke-width': 3 }));
      ends.push({ y: ey, value: track.power });
    }
    // 线尾的数值按高低排开，至少隔 26px，免得叠在一起。
    ends.sort((a, b) => a.y - b.y);
    for (let index = 1; index < ends.length; index += 1) ends[index].y = Math.max(ends[index].y, ends[index - 1].y + 26);
    if (ends.length && ends[ends.length - 1].y > height - bottom) {
      ends[ends.length - 1].y = height - bottom;
      for (let index = ends.length - 2; index >= 0; index -= 1) ends[index].y = Math.min(ends[index].y, ends[index + 1].y - 26);
    }
    for (const end of ends) chart.append(svg('text', { x: width - right + 16, y: end.y + 8, fill: 'var(--ink)', 'font-size': 22 }, fixed1(end.value)));
    const legend = el('div', { class: 'legend' }, RULES.map((rule) => el('span', {}, [el('i', { style: `background:${rule.color}` }), rule.name])));
    return card('X 战力走势', 'var(--mode-x)', chart, legend);
  }

  // 武器图走和对战记录相同的本地图片缓存。不用 records.img：它是懒加载的，导出进 SVG 后可能不画。
  function weaponIcon(url, small) {
    const src = window.Splat3Records.imageSrc(url);
    const image = src ? el('img', { src, alt: '' }) : null;
    if (image) image.addEventListener('error', () => image.remove(), { once: true });
    return el('span', { class: small ? 'weapon-icon is-small' : 'weapon-icon' }, image);
  }

  function weapons(list) {
    return card('常用武器', null, el('div', { class: 'weapons' }, list.map(([name, games, rate, image]) => el('div', { class: 'weapon' }, [
      weaponIcon(image),
      el('span', { style: 'min-width:0' }, [el('span', { class: 'weapon-name', text: name }), el('span', { class: 'weapon-meta', text: `${games} 场` })]),
      el('span', { class: 'weapon-rate', text: pct(rate) }),
    ]))), el('span', { class: 'head-note', text: '胜率' }));
  }

  // 蛮颓比赛：挑战看武器挑战力（取这段时间最高的那把武器），开放看蛮颓战力。
  function anarchy(data) {
    const column = (mode, stats, powerLabel, powerIcon) => el('div', { class: 'mode-col' }, [
      el('span', { class: 'label', text: `${mode} · ${stats.games} 场` }),
      el('span', { class: 'rate' }, [fixed1(stats.winRate), stats.winRate == null ? null : el('small', { text: '%' })]),
      splitBar(stats.winRate, `${mode}胜率 ${fixed1(stats.winRate)}%`),
      el('span', { class: 'label' }, [powerLabel, el('span', { class: `delta ${stats.delta >= 0 ? 'up' : 'down'}`, text: deltaText(stats.delta) })]),
      el('span', { class: 'power' }, [powerIcon, fixed1(stats.power)]),
    ]);
    return card('蛮颓比赛', 'var(--mode-anarchy)', el('div', { class: 'modes' }, [
      column('挑战', data.challenge, '武器挑战力', data.challenge.power == null ? null : weaponIcon(data.challenge.image, true)),
      column('开放', data.open, '蛮颓战力', null),
    ]));
  }

  // ---------- 打工 ----------

  function salmonHero(data) {
    return el('div', { class: 's-hero' }, [
      tile('打工', grouped(data.shifts), '场'),
      tile('通关率', fixed1(data.clearRate), data.clearRate == null ? null : '%', true),
      tile('金鲑鱼卵', grouped(data.golden), null, false, 'images/salmon-run-golden-egg.png'),
      tile('巨大鲑鱼击倒', grouped(data.bosses)),
    ]);
  }

  function kings(counts, scales) {
    const columns = KINGS.map((king) => {
      const [defeated, appeared] = counts[king.id] || [0, 0];
      if (!appeared) {
        return el('div', { class: 'king is-missing' }, [
          el('img', { src: king.silhouette, alt: '' }),
          el('span', { class: 'name', text: king.name }),
          el('span', { class: 'count', text: '未遇到' }),
        ]);
      }
      return el('div', { class: 'king' }, [
        el('img', { src: king.image, alt: '' }),
        el('span', { class: 'name', text: king.name }),
        el('span', { class: 'count' }, ['击倒 ', el('b', { text: String(defeated) }), ` / ${appeared}`]),
        el('span', { class: 'meter', role: 'img', 'aria-label': `击倒 ${defeated} 次，出现 ${appeared} 次` }, el('span', { style: `width:${((defeated / appeared) * 100).toFixed(1)}%` })),
      ]);
    });
    const scaleTotals = el('span', { class: 'scales' }, ['鳞片', ...[['gold', '金'], ['silver', '银'], ['bronze', '铜']].flatMap(([key, label]) => [
      el('img', { src: `images/scale-${key}.png`, alt: `${label}鳞片` }),
      el('b', { text: grouped(scales[key]) }),
    ])]);
    return card('头目鲑鱼', 'var(--mode-salmon)', el('div', { class: 'kings' }, columns), scaleTotals);
  }

  // 评价：现在的评价、这段时间的走势（每场一个点），以及危险度。
  function grade(data, kind) {
    const missing = !data.series.length;
    const series = missing ? [0, 0] : data.series;
    const end = series[series.length - 1];
    const change = end - series[0];
    const width = 412;
    const height = 70;
    const pad = { top: 10, right: 10, bottom: 6, left: 6 };
    const min = Math.min(...series);
    const max = Math.max(...series);
    const span = Math.max(max - min, 1);
    const x = (index) => pad.left + (index / Math.max(series.length - 1, 1)) * (width - pad.left - pad.right);
    const y = (value) => pad.top + (1 - (value - min) / span) * (height - pad.top - pad.bottom);
    const points = series.map((value, index) => [x(index), y(value)]);
    const line = points.map(([px, py], index) => `${index ? 'L' : 'M'}${px.toFixed(1)},${py.toFixed(1)}`).join(' ');
    const fillId = `grade-fill-${(uid += 1)}`;
    const chart = svg('svg', { class: 'chart', viewBox: `0 0 ${width} ${height}`, width, height, role: 'img', 'aria-label': `评价从 ${gradeLabel(series[0])} 到 ${gradeLabel(end)}` });
    const gradient = svg('linearGradient', { id: fillId, x1: 0, y1: 0, x2: 0, y2: 1 });
    gradient.append(svg('stop', { offset: '0', style: 'stop-color:var(--meter-a);stop-opacity:0.32' }), svg('stop', { offset: '1', style: 'stop-color:var(--meter-a);stop-opacity:0' }));
    const defs = svg('defs');
    defs.append(gradient);
    chart.append(defs);
    chart.append(svg('path', { d: `${line} L${points[points.length - 1][0].toFixed(1)},${height} L${points[0][0].toFixed(1)},${height} Z`, fill: `url(#${fillId})` }));
    chart.append(svg('path', { d: line, fill: 'none', stroke: 'var(--meter-a)', 'stroke-width': 4, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }));
    const [ex, ey] = points[points.length - 1];
    chart.append(svg('circle', { cx: ex, cy: ey, r: 7, fill: 'var(--meter-a)', stroke: 'var(--card-solid)', 'stroke-width': 3 }));
    const now = el('div', { class: 'grade-now' }, [
      el('span', { text: missing ? '—' : GRADES[Math.floor(end / 1000)] }),
      el('span', { class: 'pt', text: missing ? '' : String(Math.round(end % 1000)) }),
      change ? el('span', { class: `chg ${change > 0 ? 'up' : 'down'}`, text: signedInt(change) }) : null,
    ]);
    const hazard = el('div', { class: 'stats-2' }, [
      el('div', { class: 'stat' }, [el('span', { class: 'label', text: '最高危险度' }), el('span', { class: 'value' }, [data.hazardMax == null ? '—' : `${data.hazardMax}%`, data.hazardMax >= 333 ? el('span', { class: 'max-tag', text: 'MAX' }) : null])]),
      el('div', { class: 'stat' }, [el('span', { class: 'label', text: '平均危险度' }), el('span', { class: 'value', text: data.hazardAverage == null ? '—' : `${data.hazardAverage}%` })]),
    ]);
    const body = el('div', { style: 'display:grid;gap:12px' }, [now, missing ? null : chart, hazard]);
    return card('评价', null, body, el('span', { class: 'head-note', text: missing ? '' : `${kind === 'month' ? '月初' : '赛季初'} ${gradeLabel(series[0])}` }));
  }

  function waves(tides, events) {
    const tideRow = el('div', { class: 'tides' }, tides.map(([water, rate]) => el('div', {}, [el('span', { text: water }), integerPct(rate)])));
    const eventRows = el('div', { class: 'events' }, events.map(([name, rate, count]) => el('div', { class: 'event' }, [
      el('span', { class: 'name', text: name }),
      splitBar(rate, `${name}通关率 ${Math.round(rate)}%`),
      el('span', { class: 'pct', text: integerPct(rate) }),
      el('span', { class: 'n', text: `${count} 波` }),
    ])));
    return card('潮位 × 特殊波次', null, el('div', { style: 'display:grid;gap:12px' }, [tideRow, eventRows]), el('span', { class: 'head-note', text: '通关率' }));
  }

  function contribution(data) {
    const item = (icon, value, label) => el('div', { class: 'contrib-item' }, [
      el('img', { src: icon, alt: '' }),
      el('span', {}, [el('span', { class: 'value', text: value }), el('span', { class: 'label', text: label })]),
    ]);
    return card('个人贡献', null, el('div', { class: 'contrib' }, [
      item('images/salmon-run-golden-egg.png', fixed1(data.golden), '金鲑鱼卵'),
      item('images/salmon-run-power-egg.png', grouped(data.power), '红鲑鱼卵'),
      item('images/salmon-run-rescue.png', fixed1(data.rescues), '救援'),
      item('images/salmon-run-rescued.png', fixed1(data.rescued), '被救'),
    ]), el('span', { class: 'head-note', text: '场均' }));
  }

  function buildShare(content, kind, showName, look, period, data, name) {
    const head = el('header', { class: 's-head' }, [
      el('div', { class: 's-top' }, [
        el('span', { class: 's-brand' }, [el('span', { class: 'b1', text: 'Splat3' }), ' ', el('span', { class: 'b2', text: 'Insight' })]),
        el('span', { class: 's-kicker', text: `${CONTENT[content]} · ${KIND[kind]}` }),
      ]),
      el('h2', { class: 's-title', text: period.title }),
      el('div', { class: 's-sub', text: showName && name ? `${name} · ${period.range}` : period.range }),
    ]);
    const body = content === 'ranked'
      ? [
        rankedHero(data.hero),
        kind === 'month' ? row('row-a', [calendar(data.calendar), ruleBars(data.rules)]) : row('row-a is-wide', [xTrend(data, period)]),
        kind === 'month' ? row('row-b', [xList(data.xPower, data.peak), weapons(data.weapons)]) : row('row-b', [ruleBars(data.rules), weapons(data.weapons)]),
        row('row-c', [anarchy(data.anarchy), highlights(data.highlights)]),
      ]
      : [
        salmonHero(data.hero),
        row('row-a is-wide', [kings(data.kings, data.scales)]),
        row('row-b', [grade(data.grade, kind), waves(data.tides, data.events)]),
        row('row-c', [contribution(data.contribution), highlights(data.highlights)]),
      ];
    const classes = ['share', period.kind === 'season' && new Date(period.start).getUTCFullYear() !== new Date(period.end - 1).getUTCFullYear() ? 'is-cross-year' : null, content === 'salmon' ? 'is-salmon' : 'is-ranked', look === 'light' ? 'is-light' : null].filter(Boolean).join(' ');
    return el('article', { class: classes, 'aria-label': `${CONTENT[content]}${KIND[kind]}：${period.title}` }, [head, ...body]);
  }

  async function toDataUrl(url) {
    const response = await fetch(url);
    if (!response.ok) throw new Error('读取图片资源失败');
    const blob = await response.blob();
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result);
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
  }
  const escapeXml = (text) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  async function sharePng(node) {
    const response = await fetch('share-image.css');
    if (!response.ok) throw new Error('读取分享图样式失败');
    const shareCss = await response.text();
    const fonts = [
      `@font-face { font-family: "DFP GBZY7"; src: url("${await toDataUrl('fonts/DFP_GBZY7.ttf')}") format("truetype"); }`,
      `@font-face { font-family: "Insight Latin"; src: url("${await toDataUrl('fonts/BlitzMain.woff2')}") format("woff2"); unicode-range: U+0000-2017, U+201E-2FFF; }`,
    ].join('\n');
    const clone = node.cloneNode(true);
    // 独立的原尺寸节点需要先参与布局，才能用同一套规则适配长名称。
    const measuring = el('div', { style: 'position:fixed;left:-10000px;top:0;visibility:hidden;pointer-events:none' }, clone);
    document.body.append(measuring);
    try {
      fitHighlights(clone);
      await document.fonts.ready;
      fitHighlights(clone);
    } finally {
      measuring.remove();
    }
    const cache = new Map();
    for (const image of clone.querySelectorAll('img')) {
      const src = image.src;
      try {
        if (!cache.has(src)) cache.set(src, toDataUrl(src));
        image.setAttribute('src', await cache.get(src));
      } catch {
        image.remove();
      }
    }
    const body = new XMLSerializer().serializeToString(clone);
    const svgText = `<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="1440"><foreignObject x="0" y="0" width="1080" height="1440"><div xmlns="http://www.w3.org/1999/xhtml"><style>${escapeXml(`${fonts}\n${shareCss}`)}</style>${body}</div></foreignObject></svg>`;
    const image = new Image();
    image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svgText)}`;
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = 1080;
    canvas.height = 1440;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('生成图片失败');
    context.drawImage(image, 0, 0);
    await new Promise((resolve) => setTimeout(resolve, 300));
    context.clearRect(0, 0, 1080, 1440);
    context.drawImage(image, 0, 0);
    return new Promise((resolve, reject) => canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('生成图片失败')), 'image/png'));
  }

  function init(getContext, { invoke, revealPath }) {
    const $ = (id) => document.getElementById(id);
    const dialog = $('share-dialog');
    const preview = $('share-preview');
    const content = $('share-content');
    const kind = $('share-kind');
    let selectedContent = 'ranked';
    let selectedKind = 'month';
    const periodSelect = $('share-period');
    const showName = $('share-name');
    const save = $('share-save');
    const status = $('share-status');
    const statusText = $('share-status-text');
    const reveal = $('share-reveal');
    const lightQuery = matchMedia('(prefers-color-scheme: light)');
    let context;
    let periods = [];
    let period;
    let summary;
    let savedPath;
    let busy = false;
    let revision = 0;
    const look = () => document.documentElement.dataset.theme === 'light' || (!document.documentElement.dataset.theme && lightQuery.matches) ? 'light' : 'dark';
    const build = () => buildShare(selectedContent, selectedKind, showName.checked, look(), period, summary, context.name || '');
    function updateControls() {
      for (const button of [...content.querySelectorAll('button'), ...kind.querySelectorAll('button')]) button.disabled = busy;
      showName.disabled = busy;
      periodSelect.disabled = busy || !periods.length;
      save.disabled = busy || !summary?.n;
    }
    function render(resetPeriod = false) {
      revision += 1;
      for (const button of content.querySelectorAll('button')) button.setAttribute('aria-pressed', String(button.dataset.content === selectedContent));
      for (const button of kind.querySelectorAll('button')) button.setAttribute('aria-pressed', String(button.dataset.kind === selectedKind));
      status.hidden = true;
      reveal.hidden = true;
      savedPath = null;
      const source = selectedContent === 'salmon' ? context.data.shifts : context.data.battles.filter(window.Splat3ShareData.ranked);
      const now = Date.now();
      const previous = resetPeriod ? null : Number(periodSelect.value);
      periods = window.Splat3ShareData.periodsFor(source, selectedKind, now);
      const current = window.Splat3ShareData.periodFor(now, selectedKind, now);
      period = periods.find((item) => item.start === previous) || periods.find((item) => item.start === current.start) || periods[0] || current;
      periodSelect.replaceChildren(...periods.map((item) => el('option', { value: item.start, text: item.label })));
      periodSelect.value = String(period.start);
      $('share-period-label').textContent = selectedKind === 'month' ? '月份' : '赛季';
      summary = selectedContent === 'salmon' ? window.Splat3ShareData.salmonSummary(context.data.shifts, period) : window.Splat3ShareData.rankedSummary(context.data.battles, context.data.xSettlements, period);
      preview.classList.toggle('is-empty', !summary.n);
      if (summary.n) {
        const node = build();
        node.style.transform = 'scale(0.25)';
        preview.replaceChildren(node);
        fitHighlights(node);
        document.fonts.ready.then(() => { if (node.isConnected) fitHighlights(node); });
      } else preview.textContent = selectedContent === 'salmon' ? '这段时间没有打工记录' : '这段时间没有真格对战';
      updateControls();
    }
    $('share-button').addEventListener('click', () => {
      context = getContext();
      selectedContent = context.view === 'salmon' || context.mode === 'salmon' ? 'salmon' : 'ranked';
      selectedKind = 'month';
      showName.checked = true;
      try { const saved = JSON.parse(localStorage.getItem('splat3-share')); if (typeof saved?.showName === 'boolean') showName.checked = saved.showName; } catch {}
      dialog.showModal();
      // showModal 会把焦点交给第一个按钮并画出焦点框，改为聚焦对话框本身，按 Tab 仍会进入各个控件。
      dialog.focus();
      render(true);
    });
    // 点在对话框外面（遮罩上）时关闭。按下和松开都在外面才算，从框里拖到外面不会误关。
    const outside = (event) => {
      const box = dialog.getBoundingClientRect();
      return event.target === dialog
        && (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom);
    };
    let pressedOutside = false;
    dialog.addEventListener('pointerdown', (event) => { pressedOutside = outside(event); });
    dialog.addEventListener('click', (event) => {
      if (pressedOutside && outside(event)) dialog.close();
      pressedOutside = false;
    });
    for (const [group, key] of [[content, 'content'], [kind, 'kind']]) {
      group.addEventListener('click', (event) => {
        const button = event.target.closest(`button[data-${key}]`);
        if (!button || busy) return;
        if (key === 'content') selectedContent = button.dataset.content;
        else selectedKind = button.dataset.kind;
        render(true);
      });
    }
    periodSelect.addEventListener('change', () => render());
    showName.addEventListener('change', () => {
      try { localStorage.setItem('splat3-share', JSON.stringify({ showName: showName.checked })); } catch {}
      render();
    });
    $('share-cancel').addEventListener('click', () => dialog.close());
    dialog.addEventListener('close', () => {
      revision += 1;
      // 关闭后释放快照引用，避免后续同步更新时仍保留上一批历史记录。
      context = null;
      summary = null;
      periods = [];
      savedPath = null;
      preview.replaceChildren();
    });
    reveal.addEventListener('click', async () => {
      if (!savedPath) return;
      try { await revealPath(savedPath); } catch (error) { statusText.textContent = `打开位置失败：${String(error)}`; status.hidden = false; }
    });
    save.addEventListener('click', async () => {
      if (busy || !summary?.n) return;
      busy = true;
      updateControls();
      const currentRevision = revision;
      const filename = `Splat3Insight-${CONTENT[selectedContent]}-${period.file}.png`;
      status.hidden = true;
      reveal.hidden = true;
      try {
        // 导出独立的原尺寸节点，避免把预览的缩放写入 PNG。
        const blob = await sharePng(build());
        const bytes = new Uint8Array(await blob.arrayBuffer());
        const path = await invoke('save_share_image', bytes, { headers: { 'File-Name': encodeURIComponent(filename) } });
        if (revision !== currentRevision || !dialog.open) return;
        savedPath = path;
        statusText.textContent = `已保存到「下载」：${path.split(/[\\/]/).pop()}`;
        status.hidden = false;
        reveal.hidden = false;
      } catch (error) {
        if (revision !== currentRevision || !dialog.open) return;
        statusText.textContent = `保存失败：${String(error)}`;
        status.hidden = false;
      } finally {
        busy = false;
        updateControls();
      }
    });
    const followTheme = () => { if (dialog.open) render(); };
    lightQuery.addEventListener('change', followTheme);
    new MutationObserver(followTheme).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return { close: () => dialog.close() };
  }
  window.Splat3Share = Object.freeze({ init, buildShare, sharePng });
})();
