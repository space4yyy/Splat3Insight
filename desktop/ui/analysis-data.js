(() => {
  'use strict';

  // 分析页（对战、武器、场地、鲑鱼跑）的纯函数：把整理好的记录汇总成图表用的数据，不碰页面。
  // 输入是 Splat3DashboardData 整理后的对战和打工记录，按时间从早到晚排列。
  const data = window.Splat3DashboardData;
  const { summarize, average, isNum, WATER } = data;
  const RANKED_RULES = ['AREA', 'LOFT', 'GOAL', 'CLAM'];
  const RULE_ORDER = [...RANKED_RULES, 'TURF_WAR', 'TRI_COLOR'];
  const MODE_ORDER = ['x', 'anarchy', 'event', 'regular', 'private', 'other'];
  // 至少有 3 场分出胜负才比较胜率，和总览的热力图一致。
  const MIN_DECIDED = 3;
  const decided = (stat) => stat.wins + stat.losses;

  function groupBy(list, keyOf) {
    const groups = new Map();
    for (const item of list) {
      const key = keyOf(item);
      if (key == null || key === '') continue;
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(item);
    }
    return groups;
  }
  // 最近一条有这个字段的值（旧记录可能没有图片）。
  function lastWith(list, key) {
    for (let index = list.length - 1; index >= 0; index -= 1) {
      if (list[index][key] != null) return list[index][key];
    }
    return null;
  }
  function median(values) {
    const numbers = values.filter(isNum).sort((a, b) => a - b);
    if (!numbers.length) return null;
    const middle = Math.floor(numbers.length / 2);
    return numbers.length % 2 ? numbers[middle] : (numbers[middle - 1] + numbers[middle]) / 2;
  }
  // 按列排序，空值排在最后；相同时场次多的在前。
  function sortRows(rows, sort) {
    const sign = sort?.dir === 'asc' ? 1 : -1;
    const key = sort?.key || 'n';
    return [...rows].sort((a, b) => {
      const left = a[key];
      const right = b[key];
      if (left == null || right == null) return (left == null) - (right == null) || b.n - a.n;
      return sign * (left - right) || b.n - a.n;
    });
  }

  // ----- 对战 -----

  // 规则对比：每个规则的胜负、完胜（提前打满计数获胜）和完败，以及各模式的胜率。
  function ruleBreakdown(battles) {
    const order = (rule) => (RULE_ORDER.includes(rule) ? RULE_ORDER.indexOf(rule) : RULE_ORDER.length);
    return [...groupBy(battles, (battle) => battle.rule).entries()]
      .sort(([a], [b]) => order(a) - order(b))
      .map(([rule, games]) => ({
        rule,
        ruleName: lastWith(games, 'ruleName'),
        ...summarize(games),
        koWins: games.filter((battle) => battle.knockout && battle.win).length,
        koLosses: games.filter((battle) => battle.knockout && battle.lose).length,
        modes: MODE_ORDER
          .map((mode) => ({ mode, ...summarize(games.filter((battle) => battle.mode === mode)) }))
          .filter((item) => item.n),
      }));
  }

  // 连胜与连败之后：每场之前在同一次游玩里连胜或连败了几场（4 表示 4 场及以上），0 表示开局。
  // 两场之间隔了 30 分钟以上算新的一次游玩；平局等没有胜负的对局会打断连胜连败。
  const SESSION_GAP_MS = 30 * 60000;
  const STREAK_KEYS = [-4, -3, -2, -1, 0, 1, 2, 3, 4];
  function streakBuckets(battles, gapMs = SESSION_GAP_MS) {
    const buckets = new Map(STREAK_KEYS.map((key) => [key, []]));
    let streak = 0;
    let lastEnd = null;
    for (const battle of battles) {
      if (lastEnd == null || battle.t - lastEnd > gapMs) streak = 0;
      buckets.get(Math.max(-4, Math.min(4, streak))).push(battle);
      streak = battle.win ? Math.max(0, streak) + 1 : battle.lose ? Math.min(0, streak) - 1 : 0;
      lastEnd = battle.t + (battle.duration ?? 0) * 1000;
    }
    return STREAK_KEYS.map((key) => ({ key, ...summarize(buckets.get(key)) }));
  }

  // 直方图：每 step 一格，cap 及以上合成最后一格。
  function histogram(values, { step = 1, cap }) {
    const numbers = values.filter(isNum);
    const count = Math.floor(cap / step) + 1;
    const bins = Array.from({ length: count }, (_, index) => ({ low: index * step, last: index === count - 1, value: 0 }));
    for (const value of numbers) bins[Math.min(count - 1, Math.max(0, Math.floor(value / step)))].value += 1;
    const middle = median(numbers);
    return {
      bins,
      total: numbers.length,
      median: middle,
      // 中位数在图上的位置，以格子为单位，0 是第一格的中心。
      medianPosition: middle == null ? null : Math.min(count - 1, (middle - (step - 1) / 2) / step),
    };
  }

  // 对局时长（真格规则）：5:00 之前结束的是完胜或完败，正好 5:00 是时间到，超过 5:00 是加时。
  const DURATION_BINS = [
    ...Array.from({ length: 10 }, (_, index) => ({ from: index * 30, to: index * 30 + 30 })),
    { from: 300, to: 300, fullTime: true },
    { from: 300, to: 330, overtime: true },
    { from: 330, to: 360, overtime: true },
    { from: 360, to: 390, overtime: true },
    { from: 390, to: null, overtime: true },
  ];
  function durationBin(seconds) {
    if (seconds < 300) return Math.max(0, Math.floor(seconds / 30));
    if (seconds === 300) return 10;
    return Math.min(14, 11 + Math.floor((seconds - 301) / 30));
  }
  function durationSummary(battles) {
    const ranked = battles.filter((battle) => RANKED_RULES.includes(battle.rule) && isNum(battle.duration));
    const bins = DURATION_BINS.map((bin) => ({ ...bin, n: 0, wins: 0, losses: 0 }));
    for (const battle of ranked) {
      const bin = bins[durationBin(battle.duration)];
      bin.n += 1;
      if (battle.win) bin.wins += 1;
      else if (battle.lose) bin.losses += 1;
    }
    return {
      n: ranked.length,
      bins,
      average: average(ranked, (battle) => battle.duration),
      overtime: summarize(ranked.filter((battle) => battle.duration > 300)),
    };
  }

  // ----- 武器 -----

  // 武器排行：每把武器的场次、胜率、K/D 和场均数据；挑战战力取这把武器最近一场蛮颓挑战的战力。
  function weaponRows(battles) {
    return [...groupBy(battles, (battle) => battle.weaponId || battle.weapon).entries()].map(([key, used]) => {
      const challenge = used.filter((battle) => battle.mode === 'anarchy' && battle.bankara === 'challenge' && isNum(battle.power));
      return {
        key,
        name: used[used.length - 1].weapon,
        image: lastWith(used, 'weaponImage'),
        sub: lastWith(used, 'sub'),
        special: lastWith(used, 'special'),
        battles: used,
        ...summarize(used),
        avgSpecial: average(used, (battle) => battle.specials),
        power: challenge.length ? challenge[challenge.length - 1].power : null,
        last: used[used.length - 1].t,
      };
    });
  }

  // 滚动胜率：只算分出胜负的对局，每满 window 场出一个点。
  function rollingWinRate(battles, window = 10) {
    const games = battles.filter((battle) => battle.win || battle.lose);
    const points = [];
    let wins = 0;
    games.forEach((battle, index) => {
      wins += battle.win ? 1 : 0;
      if (index >= window) wins -= games[index - window].win ? 1 : 0;
      if (index >= window - 1) points.push({ index: index + 1, t: battle.t, rate: wins / window, battle });
    });
    return points;
  }

  // 这把武器在哪些场地打得最好、最差：每张图至少 3 场分出胜负，各取最多 3 张。
  function stageExtremes(battles) {
    const rows = [...groupBy(battles, (battle) => battle.stage).entries()]
      .map(([stage, games]) => ({ stage, ...summarize(games) }))
      .filter((row) => decided(row) >= MIN_DECIDED)
      .sort((a, b) => b.wr - a.wr || b.n - a.n);
    if (rows.length < 2) return null;
    const take = Math.min(3, Math.floor(rows.length / 2));
    return { best: rows.slice(0, take), worst: rows.slice(-take).reverse() };
  }

  // 按副武器或特殊武器汇总：key 是 'sub' 或 'special'。
  function kitRows(battles, key) {
    const imageKey = key === 'sub' ? 'subImage' : 'specialImage';
    return [...groupBy(battles, (battle) => battle[key]).entries()]
      .map(([name, used]) => ({
        name,
        image: lastWith(used, imageKey),
        ...summarize(used),
        avgSpecial: average(used, (battle) => battle.specials),
        weapons: [...groupBy(used, (battle) => battle.weapon).entries()]
          .sort((a, b) => b[1].length - a[1].length)
          .map(([weapon]) => weapon),
      }))
      .sort((a, b) => b.n - a.n);
  }

  // ----- 场地 -----

  function stageRows(battles) {
    return [...groupBy(battles, (battle) => battle.stage).entries()].map(([stage, games]) => ({
      stage,
      games,
      ...summarize(games),
      last: games[games.length - 1].t,
    }));
  }
  // 场地排行：样本够的按胜率从高到低，样本不足的排在后面按场次。
  function rankStages(rows) {
    const enough = (row) => decided(row) >= MIN_DECIDED;
    return [...rows].sort((a, b) => enough(b) - enough(a) || (enough(a) ? b.wr - a.wr : 0) || b.n - a.n);
  }
  // 场地 × 武器：列是场次最多的几把武器；最佳武器至少 3 场分出胜负、胜率不低于 50%。
  function stageWeaponMatrix(battles, columns = 6) {
    const weapons = [...groupBy(battles, (battle) => battle.weapon).entries()]
      .sort((a, b) => b[1].length - a[1].length)
      .slice(0, columns)
      .map(([weapon]) => weapon);
    const rows = stageRows(battles)
      .sort((a, b) => b.n - a.n)
      .map((row) => {
        const byWeapon = groupBy(row.games, (battle) => battle.weapon);
        const best = [...byWeapon.entries()]
          .map(([weapon, games]) => ({ weapon, ...summarize(games) }))
          .filter((item) => decided(item) >= MIN_DECIDED && item.wr >= 0.5)
          .sort((a, b) => b.wr - a.wr || b.n - a.n)[0] || null;
        return { stage: row.stage, n: row.n, cells: weapons.map((weapon) => summarize(byWeapon.get(weapon) || [])), best };
      });
    return { weapons, rows };
  }

  // ----- 鲑鱼跑 -----

  // 评级按游戏里的顺序；评级走势把“段位序号 × 1000 + 评级点”连成一条线。
  const GRADES = ['新手', '半吊子', '独当一面', '熟练', '达人', '达人+1', '达人+2', '达人+3', '传说'];
  function gradeLadder(shift) {
    const index = GRADES.indexOf(shift.gradeName);
    return index < 0 || !isNum(shift.gradePoint) ? null : index * 1000 + shift.gradePoint;
  }
  function gradeLabel(value) {
    const index = Math.max(0, Math.min(GRADES.length - 1, Math.floor(value / 1000)));
    const point = Math.round(value - index * 1000);
    return point ? `${GRADES[index]} ${point}` : GRADES[index];
  }

  // 头目鲑鱼的编号对应的图片，和 OBS 卡片一致。
  const KING_IMAGES = { 23: 'images/boss1_color.png', 24: 'images/boss2_color.png', 25: 'images/boss3_color.png', 30: 'images/boss_all_color.png' };
  const KING_ORDER = [23, 24, 25, 30];
  function kingRows(shifts) {
    const order = (id) => (KING_ORDER.includes(id) ? KING_ORDER.indexOf(id) : KING_ORDER.length);
    return [...groupBy(shifts.filter((shift) => shift.kingAppeared), (shift) => shift.kingId ?? shift.kingName ?? '?').values()]
      .map((list) => ({
        id: list[0].kingId,
        name: lastWith(list, 'kingName'),
        image: KING_IMAGES[list[0].kingId] ?? null,
        appeared: list.length,
        defeated: list.filter((shift) => shift.kingDefeated).length,
      }))
      .sort((a, b) => order(a.id) - order(b.id) || b.appeared - a.appeared);
  }
  function scaleTotals(shifts) {
    const totals = { gold: 0, silver: 0, bronze: 0 };
    for (const shift of shifts) {
      for (const key of Object.keys(totals)) if (isNum(shift.scales?.[key])) totals[key] += shift.scales[key];
    }
    return totals;
  }

  // 潮位 × 特殊波次（不含头目鲑鱼那一波）：每格的波次数和通关率；average 是全部波次的通关率。
  function tideEventMatrix(shifts) {
    const waves = shifts.flatMap((shift) => shift.waves).filter((wave) => WATER.includes(wave.water));
    const counts = new Map();
    for (const wave of waves) if (wave.event) counts.set(wave.event, (counts.get(wave.event) || 0) + 1);
    const events = [null, ...[...counts.entries()].sort((a, b) => b[1] - a[1]).map(([event]) => event)];
    return {
      total: waves.length,
      average: waves.length ? waves.filter((wave) => wave.cleared).length / waves.length : null,
      rows: events.map((event) => ({
        event,
        cells: WATER.map((water) => {
          const matched = waves.filter((wave) => (wave.event ?? null) === event && wave.water === water);
          const cleared = matched.filter((wave) => wave.cleared).length;
          return { water, n: matched.length, cleared, rate: matched.length ? cleared / matched.length : null };
        }),
      })),
    };
  }

  // 各期打工：有起止时间的按起止时间分组；旧记录没有时，场地、发放武器和规则都相同、
  // 并且和这一组上一场相隔不到 48 小时的记录算同一期。最新的一期在前。
  const FALLBACK_GAP_MS = 48 * 3600000;
  function rotationGroups(shifts, now = Date.now()) {
    const groups = [];
    const open = new Map();
    for (const shift of shifts) {
      const windowed = shift.rotationStart != null && shift.rotationEnd != null;
      const key = windowed
        ? `window:${shift.rotationStart}:${shift.rotationEnd}:${shift.rule}`
        : `guess:${shift.stage}:${(shift.supplied || []).map((weapon) => weapon.name).join('|')}:${shift.rule}`;
      let group = open.get(key);
      if (!group || (!windowed && shift.t - group.last > FALLBACK_GAP_MS)) {
        group = { key, start: windowed ? shift.rotationStart : null, end: windowed ? shift.rotationEnd : null, shifts: [], first: shift.t, last: shift.t };
        open.set(key, group);
        groups.push(group);
      }
      group.shifts.push(shift);
      group.last = shift.t;
    }
    return groups
      .map((group) => {
        const list = group.shifts;
        const lastShift = list[list.length - 1];
        const appeared = list.filter((shift) => shift.kingAppeared);
        const graded = list.filter((shift) => gradeLadder(shift) != null);
        const lastGraded = graded[graded.length - 1] || null;
        return {
          start: group.start,
          end: group.end,
          first: group.first,
          last: group.last,
          ongoing: group.end != null && group.end > now,
          stage: lastShift.stage,
          rule: lastShift.rule,
          supplied: lastShift.supplied || [],
          n: list.length,
          cleared: list.filter((shift) => shift.cleared).length,
          avgGoldenEggs: average(list, (shift) => shift.goldenEggs),
          king: appeared.length ? { name: lastWith(appeared, 'kingName'), appeared: appeared.length, defeated: appeared.filter((shift) => shift.kingDefeated).length } : null,
          scales: scaleTotals(list),
          grade: lastGraded ? { name: lastGraded.gradeName, point: lastGraded.gradePoint } : null,
        };
      })
      .sort((a, b) => (b.start ?? b.first) - (a.start ?? a.first) || b.last - a.last);
  }

  window.Splat3AnalysisData = Object.freeze({
    RANKED_RULES,
    RULE_ORDER,
    MIN_DECIDED,
    SESSION_GAP_MS,
    GRADES,
    KING_IMAGES,
    median,
    sortRows,
    ruleBreakdown,
    streakBuckets,
    histogram,
    durationBin,
    durationSummary,
    weaponRows,
    rollingWinRate,
    stageExtremes,
    kitRows,
    stageRows,
    rankStages,
    stageWeaponMatrix,
    gradeLadder,
    gradeLabel,
    kingRows,
    scaleTotals,
    tideEventMatrix,
    rotationGroups,
  });
})();
