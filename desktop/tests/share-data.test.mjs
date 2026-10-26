import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test, { after } from 'node:test';
import vm from 'node:vm';

// 明确使用有夏令时、非 UTC 的时区，避免本地月边界测试在 UTC 环境里漏检。
const previousTZ = process.env.TZ;
process.env.TZ = 'America/New_York';
after(() => { if (previousTZ === undefined) delete process.env.TZ; else process.env.TZ = previousTZ; });
const context = vm.createContext({ window: {}, atob });
for (const name of ['game-names.js', 'dashboard-data.js', 'analysis-data.js', 'share-data.js']) {
  vm.runInContext(await readFile(new URL(`../ui/${name}`, import.meta.url), 'utf8'), context);
}
const { Splat3DashboardData: data, Splat3ShareData: share } = context.window;
const equal = (value, expected) => assert.deepEqual(JSON.parse(JSON.stringify(value)), expected);
const now = new Date(2026, 9, 20, 12).getTime();
const period = share.periodFor(new Date(2026, 8, 15).getTime(), 'month', now);
const start = period.start;
const battle = (t, overrides = {}) => ({ t, mode: 'x', rule: 'AREA', bankara: null, win: true, lose: false, kills: 5, deaths: 2, duration: 300, weapon: '斯普拉射击枪', weaponId: 'w1', stage: '鳗鲶区', power: null, powerKind: null, ...overrides });
const shift = (t, overrides = {}) => ({ t, cleared: true, stage: '鲑坝', waves: [], goldenEggs: 10, powerEggs: 1000, defeated: null, rescues: 2, rescued: 1, scales: {}, gradeName: null, gradePoint: null, ...overrides });
const ranked = (battles, points = []) => share.rankedSummary(battles, points, period);
const salmon = (shifts) => share.salmonSummary(shifts, period);

test('months use local midnight, seasons use UTC, including DST and year boundaries', () => {
  assert.equal(start, new Date(2026, 8, 1).getTime());
  assert.notEqual(start, Date.UTC(2026, 8, 1));
  assert.equal(period.end, new Date(2026, 9, 1).getTime());
  const november = share.periodFor(new Date(2026, 10, 15).getTime(), 'month', now);
  assert.equal((november.end - november.start) / 3600000, 30 * 24 + 1);
  const winter = share.periodFor(Date.UTC(2027, 0, 15), 'season', Date.UTC(2027, 0, 20));
  assert.equal(winter.start, Date.UTC(2026, 11, 1));
  assert.equal(winter.end, Date.UTC(2027, 2, 1));
  assert.equal(winter.title, '2026 年 12 月 – 2027 年 2 月');
  assert.equal(winter.file, '2026年12月-2027年2月');
  assert.equal(share.periodFor(Date.UTC(2026, 8, 1) - 1, 'season', now).start, Date.UTC(2026, 5, 1));
  assert.equal(share.periodFor(Date.UTC(2026, 8, 1), 'season', now).start, Date.UTC(2026, 8, 1));
  assert.equal(ranked([battle(start - 1), battle(start), battle(period.end - 1), battle(period.end)]).n, 2);
});

test('options contain recorded periods only, newest first, and label ongoing periods', () => {
  const periods = share.periodsFor([battle(start), battle(now), battle(start + 1)], 'month', now);
  assert.equal(periods.length, 2);
  assert.equal(periods[0].label, '2026 年 10 月（截至 20 日）');
  assert.equal(periods[1].label, '2026 年 9 月');
  const seasons = share.periodsFor([battle(now)], 'season', now);
  assert.equal(seasons[0].label, '2026 年 9–11 月（进行中）');
  assert.match(seasons[0].range, /截至 10 月 20 日/);
  equal(share.periodsFor([], 'month', now), []);
});

test('ranked includes X and Anarchy only, counts decided wins, and uses local calendar days', () => {
  const rows = ['x', 'anarchy', 'regular', 'event', 'private', 'other'].map((mode, i) => battle(start + i, { mode }));
  rows.push(battle(start + 10, { win: false, lose: false }));
  const summary = ranked(rows);
  assert.equal(summary.n, 3);
  assert.equal(summary.hero.winRate, 100);
  assert.equal(summary.hero.kd, 2.5);
  assert.equal(summary.hero.hours, 900 / 3600);
  assert.equal(summary.rules[1][1], null);
  const calendar = ranked([battle(start), battle(new Date(2026, 8, 2).getTime() - 1), battle(new Date(2026, 8, 2).getTime())]).calendar;
  equal(calendar.counts.slice(0, 3), [2, 1, 0]);
  equal(calendar.levels.slice(0, 3), [5, 3, 0]);
  assert.equal(calendar.offset, 1); // 2026 年 9 月 1 日是周二。
});

test('X power compares each rule separately, preserves the in-period peak and omits absent rules', () => {
  const points = [
    { t: start - 3, rule: 'AREA', value: 1900 },
    { t: start - 2, rule: 'CLAM', value: 2800 },
    { t: start, rule: 'AREA', value: 2100 },
    { t: start + 1, rule: 'LOFT', value: 1700 },
    { t: start + 2, rule: 'AREA', value: 2050 },
    { t: start + 3, rule: 'LOFT', value: 1750 },
  ];
  const summary = ranked([], points);
  assert.equal(summary.xPower.AREA.power, 2050);
  assert.equal(summary.xPower.AREA.delta, 150);
  assert.equal(summary.xPower.LOFT.delta, 50);
  assert.equal(summary.xPower.CLAM.power, null);
  equal(summary.xPower.CLAM.points, []);
  equal(summary.xPower.AREA.points[0], { t: start, value: 1900 });
  assert.equal(summary.peak.value, 2100);
});

test('weapon challenge power chooses the highest latest weapon, with its own baseline', () => {
  const row = (t, id, power, extra = {}) => battle(t, { mode: 'anarchy', bankara: 'challenge', powerKind: 'weapon', weaponId: id, power, ...extra });
  const stats = ranked([
    row(start - 2, 'a', 2000), row(start - 1, 'b', 3000),
    row(start, 'a', 2400), row(start + 1, 'a', 2100), row(start + 2, 'b', 2050),
    row(start + 3, 'c', 2090), row(start + 4, 'd', 4000, { powerKind: 'team' }),
  ]).anarchy.challenge;
  assert.equal(stats.power, 2100);
  assert.equal(stats.delta, 100);
  assert.equal(ranked([row(start, 'c', 2200)]).anarchy.challenge.delta, 0);
});

test('open power stops at rating resets and does not borrow challenge or X power', () => {
  const row = (t, power) => battle(t, { mode: 'anarchy', bankara: 'open', powerKind: power == null ? null : 'team', power });
  const rows = [row(start - 1, 2000), row(start, 2050), row(start + 1, null), row(start + 2, 1800), row(start + 3, 1820)];
  assert.equal(ranked(rows).anarchy.open.delta, 20);
  assert.equal(ranked([row(start - 1, 2000), row(start, 2050)]).anarchy.open.delta, 50);
  assert.equal(ranked([row(start - 1, 2000), row(start, null)]).anarchy.open.power, null);
  assert.equal(ranked([...rows, battle(start + 4, { power: 9999, powerKind: 'x' })]).anarchy.open.power, 1820);
  assert.equal(ranked([row(start - 2, 2000), row(start - 1, null), row(start, 1800)]).anarchy.open.delta, 0);
  assert.equal(ranked([row(start - 1, 1700), row(start, 1800), row(start, null)]).anarchy.open.delta, 100);
});

test('streaks span time gaps and break on losses or other results; tied kills choose the latest', () => {
  const summary = ranked([
    battle(start), battle(start + 36000000), battle(start + 36000001, { win: false, lose: false }),
    battle(start + 36000002, { kills: 21, weapon: '旧武器' }), battle(start + 36000003, { kills: 21, weapon: '新武器' }),
    battle(start + 36000004), battle(start + 36000005, { win: false, lose: true }),
  ]);
  equal(summary.highlights[0], ['最长连胜', '3 场', '']);
  equal(summary.highlights[1], ['单场最多击倒', '21', '新武器']);
  assert.equal(salmon([shift(start), shift(start + 1), shift(start + 2, { cleared: false }), shift(start + 3)]).highlights[0][1], '2 场');
});

test('stage highlights require three decided battles or three shifts, even with only one stage', () => {
  assert.equal(ranked([battle(start), battle(start + 1), battle(start + 2, { win: false })]).highlights[2][1], '—');
  assert.equal(ranked([battle(start), battle(start + 1), battle(start + 2)]).highlights[2][1], '鳗鲶区');
  assert.equal(salmon([shift(start), shift(start + 1)]).highlights[2][1], '—');
  assert.equal(salmon([shift(start), shift(start + 1), shift(start + 2)]).highlights[2][1], '鲑坝');
  assert.equal(salmon([shift(start), shift(start + 1), shift(start + 2, { cleared: false, disconnected: true })]).highlights[2][1], '—');
});

test('disconnected shifts count as played but not in the clear rate', () => {
  const summary = salmon([shift(start), shift(start + 1, { cleared: false }), shift(start + 2, { cleared: false, disconnected: true })]);
  assert.equal(summary.hero.shifts, 3);
  assert.equal(summary.hero.clearRate, 50);
});

test('grade baseline is the latest graded record before the period, danger converts per record', () => {
  const rows = [shift(start - 3, { gradeName: '传说', gradePoint: 500 }), shift(start - 2, { gradeName: '传说', gradePoint: 520 }), shift(start - 1), shift(start, { gradeName: '传说', gradePoint: 540, danger: 2.4 }), shift(start + 1, { gradeName: '传说', gradePoint: 560, danger: 333 })];
  equal(salmon(rows).grade.series, [8520, 8540, 8560]);
  assert.equal(salmon(rows).grade.hazardMax, 333);
  assert.equal(salmon(rows).grade.hazardAverage, 287);
  equal(salmon(rows.slice(3)).grade.series, [8540, 8540, 8560]);
  equal(salmon([rows[0], shift(start)]).grade.series, []);
});

test('special waves choose four most frequent events, skip plain and fewer than three waves', () => {
  const waves = [['狂潮', 6], ['雾', 5], ['烤架鱼进发', 4], ['寻找金鲑鱼', 3], ['走私鱼来袭', 3], ['巨型龙卷风', 2], [null, 10], ['无', 4]].flatMap(([event, n]) => Array.from({ length: n }, () => ({ water: '普通', event, cleared: true })));
  const summary = salmon([shift(start, { waves })]);
  equal(summary.events.map((row) => row[0]), ['狂潮', '雾', '烤架鱼进发', '寻找金鲑鱼']);
  equal(summary.tides, [['干潮', null], ['普通', 100], ['满潮', null]]);
});

test('normalized defeated counts sum across all shift modes, contributions exclude golden assists', () => {
  const { shifts } = data.prepare({ coop: [
    { id: 'a', played_at_ms: start, rule: 'REGULAR', defeat_enemy_count: 10, golden_eggs: 5, golden_assists: 100 },
    { id: 'b', played_at_ms: start + 1, rule: 'BIG_RUN', defeat_enemy_count: 20, golden_eggs: 15 },
    { id: 'c', played_at_ms: start + 2, rule: 'TEAM_CONTEST', defeat_enemy_count: null },
  ] });
  assert.equal(shifts[2].defeated, null);
  const summary = salmon(shifts);
  assert.equal(summary.n, 3);
  assert.equal(summary.hero.bosses, 30);
  assert.equal(summary.hero.golden, 20);
  assert.equal(summary.contribution.golden, 10);
  assert.equal(salmon([shift(start)]).hero.bosses, null);
  assert.equal(ranked([]).hero.kd, null);
});
