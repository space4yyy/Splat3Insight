(() => {
  'use strict';

  // 分享图统计只读取整理好的记录，不修改输入，也不依赖页面状态。
  const dashboard = window.Splat3DashboardData;
  const analysis = window.Splat3AnalysisData;
  const RULES = ['AREA', 'LOFT', 'GOAL', 'CLAM'];
  const isNum = (value) => typeof value === 'number' && Number.isFinite(value);
  const ranked = (battle) => battle.mode === 'x' || battle.mode === 'anarchy';
  const inside = (row, period) => row.t >= period.start && row.t < period.end;
  const last = (list) => list[list.length - 1];
  const pad = (value) => String(value).padStart(2, '0');
  const percent = (value) => value == null ? null : value * 100;
  const total = (list, key) => {
    const values = list.map((row) => row[key]).filter(isNum);
    return values.length ? values.reduce((sum, value) => sum + value, 0) : null;
  };
  const average = (list, key) => dashboard.average(list, (row) => row[key]);
  const pctText = (value) => value == null ? '' : `${(value * 100).toFixed(1)}%`;

  function periodFor(time, kind, now = Date.now()) {
    const date = new Date(time);
    const start = kind === 'season'
      ? dashboard.seasonStart(time)
      : new Date(date.getFullYear(), date.getMonth(), 1).getTime();
    const first = new Date(start);
    const end = kind === 'season'
      ? Date.UTC(first.getUTCFullYear(), first.getUTCMonth() + 3, 1)
      : new Date(first.getFullYear(), first.getMonth() + 1, 1).getTime();
    // 赛季日期使用 UTC，月份使用本地日期，避免时区改变赛季的标题。
    const parts = (t) => {
      const d = new Date(t);
      return kind === 'season'
        ? [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate()]
        : [d.getFullYear(), d.getMonth() + 1, d.getDate()];
    };
    const [year, month] = parts(start);
    const [endYear, endMonth, endDay] = parts(end - 1);
    const ongoing = start <= now && now < end;
    const current = new Date(now);
    const title = kind === 'month'
      ? `${year} 年 ${month} 月`
      : year === endYear
        ? `${year} 年 ${month}–${endMonth} 月`
        : `${year} 年 ${month} 月 – ${endYear} 年 ${endMonth} 月`;
    const file = kind === 'month'
      ? `${year}年${month}月`
      : year === endYear
        ? `${year}年${month}-${endMonth}月`
        : `${year}年${month}月-${endYear}年${endMonth}月`;
    const endDate = `${year === endYear ? '' : `${endYear}.`}${pad(endMonth)}.${pad(endDay)}`;
    const cutoff = ongoing ? ` · 截至 ${current.getMonth() + 1} 月 ${current.getDate()} 日` : '';
    const range = `${year}.${pad(month)}.01 – ${endDate}${cutoff}`;
    const progress = kind === 'month' ? `（截至 ${current.getDate()} 日）` : '（进行中）';
    const label = `${title}${ongoing ? progress : ''}`;
    return {
      kind, start, end, until: Math.min(now, end),
      year, month, title, range, file, label, ongoing,
    };
  }

  function periodsFor(list, kind, now = Date.now()) {
    const periods = new Map();
    for (const row of list) {
      if (!isNum(row.t)) continue;
      const date = new Date(row.t);
      const start = kind === 'season'
        ? dashboard.seasonStart(row.t)
        : new Date(date.getFullYear(), date.getMonth(), 1).getTime();
      if (!periods.has(start)) periods.set(start, periodFor(row.t, kind, now));
    }
    return [...periods.values()].sort((a, b) => b.start - a.start);
  }

  function longest(list, success) {
    let current = 0;
    let best = 0;
    for (const row of list) {
      current = success(row) ? current + 1 : 0;
      best = Math.max(best, current);
    }
    return best;
  }

  function most(list, key) {
    // 同值时更新为较新的记录，单场最多击倒也遵循这个规则。
    return list.reduce((best, row) => (
      isNum(row[key]) && (!best || row[key] >= best[key]) ? row : best
    ), null);
  }

  // 调用方先按 X 规则或挑战武器分轨；期末值只和同一轨道的期初值比较。
  function powerTrack(all, period) {
    const points = all.filter((row) => inside(row, period));
    if (!points.length) return { power: null, delta: null, points: [] };
    const before = last(all.filter((row) => row.t < period.start));
    // 没有期前记录时，以本期第一个值为起点；本期无值时保持不可用。
    const start = before || points[0];
    return {
      power: last(points).value,
      delta: last(points).value - start.value,
      points: before ? [{ t: period.start, value: before.value }, ...points] : points,
    };
  }

  function rankedSummary(battles, xSettlements, period) {
    const all = battles.filter(ranked);
    const list = all.filter((row) => inside(row, period));
    const summary = dashboard.summarize(list);
    const xPower = Object.fromEntries(RULES.map((rule) => {
      const track = xSettlements.filter((point) => point.rule === rule && isNum(point.value));
      return [rule, powerTrack(track, period)];
    }));
    const peak = most(
      xSettlements.filter((row) => RULES.includes(row.rule) && inside(row, period)),
      'value',
    );
    const challengeAll = all.filter((row) => (
      row.mode === 'anarchy' && row.bankara === 'challenge'
      && row.powerKind === 'weapon' && isNum(row.power) && row.weaponId
    ));
    // 每把武器先取本期最后的战力，再选其中最高的一把，不取历史峰值。
    const tracks = new Map();
    for (const row of challengeAll.filter((item) => inside(item, period))) tracks.set(row.weaponId, row);
    const chosen = [...tracks.values()].sort((a, b) => b.power - a.power || b.t - a.t)[0];
    const challengeTrack = chosen
      ? challengeAll.filter((row) => row.weaponId === chosen.weaponId)
        .map((row) => ({ t: row.t, value: row.power }))
      : [];
    const challengePower = chosen
      ? powerTrack(challengeTrack, period)
      : { power: null, delta: null };
    const openAll = all.filter((row) => (
      row.mode === 'anarchy' && row.bankara === 'open' && row.t < period.end
    ));
    const openLast = last(openAll.filter((row) => (
      inside(row, period) && row.powerKind === 'team' && isNum(row.power)
    )));
    let openPower = { power: null, delta: null };
    if (openLast) {
      const throughEnd = openAll.slice(0, openAll.lastIndexOf(openLast) + 1);
      let reset = -1;
      // 没有战力的开放对局代表重新测定，变化的起点不能越过它。
      throughEnd.forEach((row, index) => {
        if (row.powerKind !== 'team' || !isNum(row.power)) reset = index;
      });
      const segment = throughEnd.slice(reset + 1).filter((row) => row.powerKind === 'team' && isNum(row.power));
      // 在最后一段测定中取期前末值；没有时，用这一段的第一个值。
      const baseline = last(segment.filter((row) => row.t < period.start)) || segment[0];
      openPower = { power: openLast.power, delta: openLast.power - baseline.power };
    }
    const modeStats = (mode, power) => {
      const rows = list.filter((row) => row.mode === 'anarchy' && row.bankara === mode);
      return { games: rows.length, winRate: percent(dashboard.summarize(rows).wr), ...power };
    };
    const counts = Array.from({ length: new Date(period.year, period.month, 0).getDate() }, () => 0);
    if (period.kind === 'month') {
      for (const row of list) counts[new Date(row.t).getDate() - 1] += 1;
    }
    const maxDay = Math.max(0, ...counts);
    const best = analysis.stageRows(list)
      .filter((row) => row.wins + row.losses >= analysis.MIN_DECIDED)
      .sort((a, b) => b.wr - a.wr || b.n - a.n)[0];
    const kills = most(list, 'kills');
    return {
      n: list.length,
      hero: {
        battles: list.length,
        winRate: percent(summary.wr),
        kd: summary.kd,
        hours: total(list, 'duration') == null ? null : total(list, 'duration') / 3600,
      },
      calendar: {
        counts,
        // 0 场单独一档，其余相对当月单日最大场数分为 1–5 档；周一开头。
        levels: counts.map((n) => n ? Math.ceil(n / maxDay * 5) : 0),
        offset: (new Date(period.start).getDay() + 6) % 7,
        month: period.month,
      },
      rules: RULES.map((rule) => {
        const stats = dashboard.summarize(list.filter((row) => row.rule === rule));
        return [rule, percent(stats.wr), stats.n];
      }),
      xPower, peak,
      weapons: analysis.weaponRows(list)
        .sort((a, b) => b.n - a.n || b.last - a.last)
        .slice(0, 3)
        .map((row) => [row.name, row.n, percent(row.wr), row.image]),
      anarchy: {
        challenge: modeStats('challenge', { ...challengePower, image: chosen?.weaponImage }),
        open: modeStats('open', openPower),
      },
      highlights: [
        ['最长连胜', `${longest(list, (row) => row.win)} 场`, ''],
        ['单场最多击倒', kills ? String(kills.kills) : '—', kills?.weapon || ''],
        ['最拿手的场地', best?.stage || '—', pctText(best?.wr)],
      ],
    };
  }

  function salmonSummary(shifts, period) {
    const list = shifts.filter((row) => inside(row, period));
    const graded = list.filter((row) => analysis.gradeLadder(row) != null);
    // 评价起点取本期之前最后一条有评价的记录，跳过缺评价的打工。
    const before = last(shifts.filter((row) => (
      row.t < period.start && analysis.gradeLadder(row) != null
    )));
    const series = graded.length
      ? [analysis.gradeLadder(before || graded[0]), ...graded.map(analysis.gradeLadder)]
      : [];
    // 记录中的危险度可能是百分数或比例，逐条换算后再统计。
    const dangers = list.filter((row) => isNum(row.danger))
      .map((row) => row.danger < 10 ? row.danger * 100 : row.danger);
    const matrix = analysis.tideEventMatrix(list);
    // 特殊波次合并三种潮位，只列至少 3 波、出现最多的前 4 种。
    const events = matrix.rows
      .filter((row) => row.event && row.event !== '无')
      .map((row) => {
        const n = row.cells.reduce((sum, cell) => sum + cell.n, 0);
        const cleared = row.cells.reduce((sum, cell) => sum + cell.cleared, 0);
        return [row.event, 100 * cleared / n, n];
      })
      .filter((row) => row[2] >= 3)
      .sort((a, b) => b[2] - a[2])
      .slice(0, 4);
    // 最拿手的场地只看有结果的打工，掉线的场次和无效对战一样不算。
    const stages = new Map();
    for (const row of list.filter((shift) => !shift.disconnected)) {
      const stage = stages.get(row.stage) || { stage: row.stage, n: 0, cleared: 0 };
      stage.n += 1;
      stage.cleared += Number(row.cleared);
      stages.set(row.stage, stage);
    }
    const best = [...stages.values()]
      .filter((row) => row.n >= 3)
      .sort((a, b) => b.cleared / b.n - a.cleared / a.n || b.n - a.n)[0];
    const eggs = most(list, 'goldenEggs');
    return {
      n: list.length,
      hero: {
        shifts: list.length,
        clearRate: percent(dashboard.clearRate(list)),
        golden: total(list, 'goldenEggs'),
        bosses: total(list, 'defeated'),
      },
      kings: Object.fromEntries(analysis.kingRows(list)
        .map((row) => [row.id, [row.defeated, row.appeared]])),
      scales: analysis.scaleTotals(list),
      grade: {
        series,
        hazardMax: dangers.length ? Math.round(Math.max(...dangers)) : null,
        hazardAverage: dangers.length
          ? Math.round(dangers.reduce((sum, value) => sum + value, 0) / dangers.length)
          : null,
      },
      tides: ['干潮', '普通', '满潮'].map((water) => {
        const cells = matrix.rows.flatMap((row) => row.cells.filter((cell) => cell.water === water));
        const n = cells.reduce((sum, cell) => sum + cell.n, 0);
        const cleared = cells.reduce((sum, cell) => sum + cell.cleared, 0);
        return [water, n ? 100 * cleared / n : null];
      }),
      events,
      contribution: {
        golden: average(list, 'goldenEggs'),
        power: average(list, 'powerEggs') == null ? null : Math.round(average(list, 'powerEggs')),
        rescues: average(list, 'rescues'),
        rescued: average(list, 'rescued'),
      },
      highlights: [
        ['最长连续通关', `${longest(list, (row) => row.cleared)} 场`, ''],
        ['单场最多金鲑鱼卵', eggs ? String(eggs.goldenEggs) : '—', eggs?.stage || ''],
        ['最拿手的场地', best?.stage || '—', best ? pctText(best.cleared / best.n) : ''],
      ],
    };
  }
  window.Splat3ShareData = Object.freeze({ ranked, periodFor, periodsFor, rankedSummary, salmonSummary });
})();
