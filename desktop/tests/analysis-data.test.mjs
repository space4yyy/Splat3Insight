import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const ui = new URL('../ui/', import.meta.url);

async function loadScripts(...names) {
  const context = vm.createContext({ window: {}, atob });
  for (const name of names) vm.runInContext(await readFile(new URL(name, ui), 'utf8'), context);
  return context.window;
}

const { Splat3DashboardData: data, Splat3AnalysisData: analysis } = await loadScripts('game-names.js', 'dashboard-data.js', 'analysis-data.js');
// vm 里创建的数组和对象来自另一个 realm，比较前先转成普通的 JSON 值。
const deepEqual = (actual, expected) => assert.deepEqual(JSON.parse(JSON.stringify(actual)), expected);
const MINUTE = 60000;
const HOUR = 60 * MINUTE;

// 整理后的对战记录（和 dashboard-data.js 的 normalizeBattle 输出一致）。
function battle(overrides) {
  const win = overrides.result === 'win';
  const lose = overrides.result === 'lose';
  return {
    t: 0, mode: 'x', bankara: null, rule: 'AREA', stage: '鳗鲶区', weapon: '斯普拉机动枪', weaponId: 'w1',
    win, lose, knockout: false, duration: 300, kills: 5, deaths: 5, specials: 2, paint: 1000, power: null,
    sub: '吸盘炸弹', special: '螃蟹坦克', ...overrides,
  };
}
function shift(overrides) {
  return {
    t: 0, rule: 'REGULAR', stage: '鲑坝', cleared: true, waves: [], kingAppeared: false, kingDefeated: false, kingName: null, kingId: null,
    goldenEggs: 10, rescues: 1, rescued: 1, goldenAssists: 3, gradeName: '达人', gradePoint: 400,
    scales: { gold: null, silver: null, bronze: null }, supplied: [], rotationStart: null, rotationEnd: null, ...overrides,
  };
}

test('normalized records carry kits, assists, supplied weapons, the rotation and the King', () => {
  const { battles, shifts } = data.prepare({
    battles: [{
      id: 'b1', played_at_ms: 1, mode: 'BANKARA', rule_code: 'AREA', judgement: 'WIN',
      sub_weapon_name: '吸盤炸彈', sub_weapon_image: 'https://example.test/sub.png',
      special_weapon_name: '螃蟹坦克', special_weapon_image: null,
    }],
    coop: [{
      id: 'c1', played_at_ms: 2, cleared: true, waves: [], boss_appeared: true, boss_id: 'Q29vcEVuZW15LTIz', boss_name: 'Cohozuna',
      golden_assists: 4, supplied_weapons: [{ name: 'Splattershot', image: 'https://example.test/c.png' }, { name: null, image: null }],
      rotation_start_ms: 100, rotation_end_ms: 200,
    }],
  });
  assert.equal(battles[0].sub, '吸盘炸弹');
  assert.equal(battles[0].subImage, 'https://example.test/sub.png');
  assert.equal(battles[0].special, '螃蟹坦克');
  assert.equal(battles[0].specialImage, null);
  assert.equal(shifts[0].goldenAssists, 4);
  assert.equal(shifts[0].supplied[0].name, '斯普拉射击枪');
  assert.equal(shifts[0].supplied[1].name, null);
  assert.equal(shifts[0].rotationStart, 100);
  assert.equal(shifts[0].kingId, 23);
  assert.equal(shifts[0].kingName, '横纲');
});

test('image links listed once in the facts are looked up by position', () => {
  const { battles, shifts } = data.prepare({
    images: ['https://example.test/weapon.png', 'https://example.test/sub.png'],
    battles: [{ id: 'b1', played_at_ms: 1, mode: 'X_MATCH', weapon_image: 0, sub_weapon_image: 1, special_weapon_image: 7 }],
    coop: [{ id: 'c1', played_at_ms: 2, cleared: true, waves: [], supplied_weapons: [{ name: 'Splattershot', image: 0 }, { name: null, image: null }] }],
  });
  assert.equal(battles[0].weaponImage, 'https://example.test/weapon.png');
  assert.equal(battles[0].subImage, 'https://example.test/sub.png');
  assert.equal(battles[0].specialImage, null);
  assert.equal(shifts[0].supplied[0].image, 'https://example.test/weapon.png');
  assert.equal(shifts[0].supplied[1].image, null);
});

test('rule breakdown counts knockouts by side and splits the modes', () => {
  const rows = analysis.ruleBreakdown([
    battle({ rule: 'TURF_WAR', mode: 'regular', result: 'win', duration: 180 }),
    battle({ rule: 'AREA', result: 'win', knockout: true }),
    battle({ rule: 'AREA', result: 'lose', knockout: true }),
    battle({ rule: 'AREA', mode: 'anarchy', result: 'lose' }),
    battle({ rule: 'AREA', result: 'draw' }),
  ]);
  deepEqual(rows.map((row) => row.rule), ['AREA', 'TURF_WAR']);
  const area = rows[0];
  assert.equal(area.n, 4);
  assert.equal(area.wins, 1);
  assert.equal(area.losses, 2);
  assert.equal(area.koWins, 1);
  assert.equal(area.koLosses, 1);
  deepEqual(area.modes.map((item) => [item.mode, item.n]), [['x', 3], ['anarchy', 1]]);
});

test('streaks count within a session and restart after a 30 minute gap or a draw', () => {
  const at = (minutes, result) => battle({ t: minutes * MINUTE, duration: 180, result });
  const buckets = analysis.streakBuckets([
    at(0, 'win'), at(4, 'win'), at(8, 'lose'), at(12, 'lose'), at(16, 'lose'), at(20, 'lose'), at(24, 'lose'),
    at(28, 'draw'), at(32, 'win'),
    at(120, 'lose'),
  ]);
  const byKey = Object.fromEntries(buckets.map((bucket) => [bucket.key, bucket.n]));
  // 开局：第 1 场、平局之后那一场、隔了一个半小时的那一场。
  assert.equal(byKey[0], 3);
  assert.equal(byKey[1], 1);
  assert.equal(byKey[2], 1);
  assert.equal(byKey[-1], 1);
  assert.equal(byKey[-2], 1);
  assert.equal(byKey[-3], 1);
  // 连败 4 场之后的两场（第 5 场败、平局）都算在 4+ 里。
  assert.equal(byKey[-4], 2);
  assert.equal(buckets.find((bucket) => bucket.key === 0).wins, 2);
});

test('histograms cap the last bin and place the median between bins when needed', () => {
  const kills = analysis.histogram([0, 1, 1, 2, 20], { step: 1, cap: 15 });
  assert.equal(kills.bins.length, 16);
  assert.equal(kills.bins[1].value, 2);
  assert.equal(kills.bins[15].value, 1);
  assert.equal(kills.median, 1);
  assert.equal(kills.medianPosition, 1);
  const eggs = analysis.histogram([12, 13, 14, null], { step: 2, cap: 40 });
  assert.equal(eggs.total, 3);
  assert.equal(eggs.bins[6].value, 2);
  assert.equal(eggs.bins[7].value, 1);
  assert.equal(eggs.median, 13);
  assert.equal(eggs.medianPosition, 6.25);
  assert.equal(analysis.histogram([1, 2], { step: 1, cap: 8 }).median, 1.5);
});

test('ranked durations separate knockouts, full time and overtime', () => {
  const summary = analysis.durationSummary([
    battle({ duration: 120, result: 'win', knockout: true }),
    battle({ duration: 300, result: 'lose' }),
    battle({ duration: 301, result: 'win' }),
    battle({ duration: 420, result: 'lose' }),
    battle({ rule: 'TURF_WAR', duration: 180, result: 'win' }),
    battle({ duration: null, result: 'win' }),
  ]);
  assert.equal(summary.n, 4);
  assert.equal(summary.bins[4].wins, 1);
  assert.equal(summary.bins[10].losses, 1);
  assert.ok(summary.bins[10].fullTime);
  assert.equal(summary.bins[11].wins, 1);
  assert.equal(summary.bins[14].losses, 1);
  assert.equal(summary.overtime.n, 2);
  assert.equal(summary.overtime.wr, 0.5);
  assert.equal(summary.average, (120 + 300 + 301 + 420) / 4);
});

test('weapon rows use the latest challenge power of that weapon only', () => {
  const rows = analysis.weaponRows([
    battle({ t: 1, mode: 'anarchy', bankara: 'challenge', power: 1900, result: 'win', weaponImage: 'old.png' }),
    battle({ t: 2, mode: 'anarchy', bankara: 'open', power: 1500, result: 'lose', weaponImage: null }),
    battle({ t: 3, mode: 'anarchy', bankara: 'challenge', power: 1912.5, result: 'win', specials: 4 }),
    battle({ t: 4, weapon: '北斋', weaponId: 'w2', mode: 'x', power: 2300, result: 'lose' }),
  ]);
  const dualies = rows.find((row) => row.key === 'w1');
  assert.equal(dualies.n, 3);
  assert.equal(dualies.power, 1912.5);
  assert.equal(dualies.image, 'old.png');
  assert.equal(dualies.avgSpecial, (2 + 2 + 4) / 3);
  assert.equal(dualies.last, 3);
  assert.equal(rows.find((row) => row.key === 'w2').power, null);
  const sorted = analysis.sortRows(rows, { key: 'power', dir: 'desc' });
  assert.equal(sorted[sorted.length - 1].key, 'w2');
});

test('rolling win rate skips games without a result', () => {
  const points = analysis.rollingWinRate([
    battle({ t: 1, result: 'win' }),
    battle({ t: 2, result: 'draw' }),
    battle({ t: 3, result: 'lose' }),
    battle({ t: 4, result: 'win' }),
  ], 2);
  deepEqual(points.map((point) => [point.index, point.rate, point.t]), [[2, 0.5, 3], [3, 0.5, 4]]);
});

test('stage extremes need two stages with three decided games each', () => {
  const games = (stage, wins, losses) => [
    ...Array.from({ length: wins }, () => battle({ stage, result: 'win' })),
    ...Array.from({ length: losses }, () => battle({ stage, result: 'lose' })),
  ];
  assert.equal(analysis.stageExtremes([...games('A', 3, 0), ...games('B', 1, 1)]), null);
  const extremes = analysis.stageExtremes([...games('A', 3, 0), ...games('B', 1, 2), ...games('C', 2, 1), ...games('D', 0, 3)]);
  deepEqual(extremes.best.map((row) => row.stage), ['A', 'C']);
  deepEqual(extremes.worst.map((row) => row.stage), ['D', 'B']);
});

test('kit rows group by sub or special weapon and list the main weapons by use', () => {
  const rows = analysis.kitRows([
    battle({ weapon: '斯普拉射击枪', sub: '吸盘炸弹', result: 'win' }),
    battle({ weapon: '斯普拉机动枪', sub: '吸盘炸弹', result: 'win' }),
    battle({ weapon: '斯普拉机动枪', sub: '吸盘炸弹', result: 'lose', subImage: 'sub.png' }),
    battle({ weapon: '斯普拉滚筒', sub: '冰壶炸弹', result: 'lose' }),
    battle({ weapon: '斯普拉滚筒', sub: null, result: 'lose' }),
  ], 'sub');
  deepEqual(rows.map((row) => [row.name, row.n]), [['吸盘炸弹', 3], ['冰壶炸弹', 1]]);
  deepEqual(rows[0].weapons, ['斯普拉机动枪', '斯普拉射击枪']);
  assert.equal(rows[0].image, 'sub.png');
});

test('opponent weapon rows deduplicate within a battle, track appearances, and rank matchup extremes', () => {
  const opp = (id, name, image = null) => ({ id, name, image });
  const list = [
    battle({ t: 1, result: 'win', kills: 8, deaths: 2, opponents: [opp('o1', '斯普拉射击枪', 's1.png'), opp('o1', '斯普拉射击枪', 's2.png'), opp('o2', '北斋')] }),
    battle({ t: 2, result: 'win', kills: 6, deaths: 3, opponents: [opp('o1', '斯普拉射击枪'), opp('o2', '北斋', 'b.png')] }),
    battle({ t: 3, result: 'win', kills: 7, deaths: 4, opponents: [opp('o1', '斯普拉射击枪')] }),
    battle({ t: 4, result: 'lose', kills: 3, deaths: 6, opponents: [opp('o2', '北斋'), opp('o3', '公升4K')] }),
    battle({ t: 5, result: 'lose', kills: 2, deaths: 5, opponents: [opp('o2', '北斋'), opp('o3', '公升4K')] }),
    battle({ t: 6, result: 'win', kills: 5, deaths: 5, opponents: [] }),
  ];
  const rows = analysis.opponentWeaponRows(list);
  deepEqual(rows.map((row) => [row.key, row.n, row.appearances, row.wins, row.losses]), [
    ['o2', 4, 4, 2, 2],
    ['o1', 3, 4, 3, 0],
    ['o3', 2, 2, 0, 2],
  ]);
  // 登场率分母是有对手武器数据的 5 场对战。
  assert.equal(rows[0].share, 4 / 5);
  assert.equal(rows[1].share, 3 / 5);
  assert.equal(rows[1].image, 's2.png');
  assert.equal(rows[1].wr, 1);
  assert.equal(rows[0].wr, 0.5);

  // o3 只有 2 场分出胜负，不进入对阵胜率对比；o1（100%）与 o2（50%）各满 3 场。
  const extremes = analysis.opponentExtremes(rows, 5);
  deepEqual(extremes.best.map((row) => row.key), ['o1']);
  deepEqual(extremes.worst.map((row) => row.key), ['o2']);
  deepEqual(extremes.ordered.map((row) => row.key), ['o1', 'o2']);
  assert.equal(analysis.opponentExtremes(rows.filter((row) => row.key !== 'o1')), null);
});

test('stage ranking and the stage × weapon matrix only trust three decided games', () => {
  const list = [
    ...Array.from({ length: 3 }, () => battle({ stage: 'A', weapon: 'W1', result: 'win' })),
    battle({ stage: 'A', weapon: 'W2', result: 'lose' }),
    ...Array.from({ length: 3 }, () => battle({ stage: 'B', weapon: 'W2', result: 'lose' })),
    battle({ stage: 'C', weapon: 'W1', result: 'win' }),
  ];
  const ranked = analysis.rankStages(analysis.stageRows(list));
  deepEqual(ranked.map((row) => row.stage), ['A', 'B', 'C']);
  const matrix = analysis.stageWeaponMatrix(list, 1);
  deepEqual(matrix.weapons, ['W1']);
  deepEqual(matrix.rows.map((row) => row.stage), ['A', 'B', 'C']);
  assert.equal(matrix.rows[0].cells[0].n, 3);
  assert.equal(matrix.rows[0].best.weapon, 'W1');
  // B 的 W2 全败，不算最佳武器。
  assert.equal(matrix.rows[1].best, null);
});

test('grades form one ladder from 新手 to 传说', () => {
  assert.equal(analysis.gradeLadder(shift({ gradeName: '达人+2', gradePoint: 340 })), 6340);
  assert.equal(analysis.gradeLadder(shift({ gradeName: 'Unknown', gradePoint: 340 })), null);
  assert.equal(analysis.gradeLadder(shift({ gradePoint: null })), null);
  assert.equal(analysis.gradeLabel(6340), '达人+2 340');
  assert.equal(analysis.gradeLabel(8000), '传说');
});

test('kings are listed in game order with their images', () => {
  const rows = analysis.kingRows([
    shift({ kingAppeared: true, kingId: 24, kingName: '辰龙', kingDefeated: true }),
    shift({ kingAppeared: true, kingId: 23, kingName: '横纲', kingDefeated: false }),
    shift({ kingAppeared: true, kingId: 24, kingName: '辰龙', kingDefeated: false }),
    shift({ kingAppeared: false, kingId: 25 }),
  ]);
  deepEqual(rows.map((row) => [row.name, row.appeared, row.defeated]), [['横纲', 1, 0], ['辰龙', 2, 1]]);
  assert.equal(rows[1].image, 'images/boss2_color.png');
  deepEqual(analysis.scaleTotals([shift({ scales: { gold: 1, silver: 2, bronze: null } }), shift({ scales: { gold: 0, silver: 1, bronze: 3 } })]), { gold: 1, silver: 3, bronze: 3 });
});

test('defeat bars fill toward the target, grow past it, and leave the Triumvirate without one', () => {
  deepEqual(analysis.DEFEAT_TARGETS, { enemies: 10000, bosses: 1000 });
  deepEqual(analysis.defeatBars([{ id: 4, count: 2500 }, { id: 5, count: 5000 }], 10000), {
    target: 10000,
    anchor: 1,
    rows: [{ id: 5, count: 5000, share: 0.5 }, { id: 4, count: 2500, share: 0.25 }],
  });
  // 超过目标时刻度放大到最多的那个，目标的锚线跟着往左移。
  const past = analysis.defeatBars([{ id: 4, count: 12500 }, { id: 5, count: 5000 }], 10000);
  assert.equal(past.anchor, 0.8);
  deepEqual(past.rows.map((row) => row.share), [1, 0.4]);
  // 头目联合没有目标：不算进刻度，也不画横条，排在最后。
  deepEqual(analysis.defeatBars([{ id: 30, count: 4000 }, { id: 23, count: 250 }], 1000), {
    target: 1000,
    anchor: 1,
    rows: [{ id: 23, count: 250, share: 0.25 }, { id: 30, count: 4000, share: null }],
  });
});

test('the tide × event matrix keeps plain waves first and orders events by frequency', () => {
  const wave = (water, event, cleared) => ({ water, event, cleared });
  const matrix = analysis.tideEventMatrix([
    shift({ waves: [wave('普通', null, true), wave('满潮', '雾', false), wave('普通', '狂潮', true)] }),
    shift({ waves: [wave('满潮', '雾', true), wave('干潮', null, true), wave('未知潮位', null, false)] }),
  ]);
  assert.equal(matrix.total, 5);
  assert.equal(matrix.average, 4 / 5);
  deepEqual(matrix.rows.map((row) => row.event), [null, '雾', '狂潮']);
  const fog = matrix.rows[1].cells.find((cell) => cell.water === '满潮');
  assert.equal(fog.n, 2);
  assert.equal(fog.rate, 0.5);
});

test('disconnected shifts count as played but stay out of clear rates and wave totals', () => {
  const wave = (water, cleared) => ({ water, event: null, cleared });
  const list = [
    shift({ t: 1 * HOUR, waves: [wave('普通', true)] }),
    shift({ t: 2 * HOUR, cleared: false, waves: [wave('普通', false)] }),
    shift({ t: 3 * HOUR, cleared: false, disconnected: true, waves: [wave('满潮', false)] }),
  ];
  const matrix = analysis.tideEventMatrix(list);
  assert.equal(matrix.total, 2);
  assert.equal(matrix.average, 1 / 2);
  const [group] = analysis.rotationGroups(list, 4 * HOUR);
  assert.equal(group.n, 3);
  assert.equal(group.clear, 1 / 2);
});

test('rotations group by their window, fall back to stage and weapons, and list the newest first', () => {
  const supplied = [{ name: '斯普拉射击枪' }];
  const groups = analysis.rotationGroups([
    shift({ t: 1 * HOUR, rotationStart: 0, rotationEnd: 40 * HOUR, kingAppeared: true, kingId: 23, kingName: '横纲', kingDefeated: true, scales: { gold: 1, silver: 0, bronze: 2 } }),
    shift({ t: 2 * HOUR, rule: 'BIG_RUN', stage: '醋饭海洋世界', rotationStart: 0, rotationEnd: 48 * HOUR }),
    shift({ t: 3 * HOUR, rotationStart: 0, rotationEnd: 40 * HOUR, cleared: false, gradePoint: 420 }),
    shift({ t: 50 * HOUR, stage: '新卷堡', supplied }),
    shift({ t: 60 * HOUR, stage: '新卷堡', supplied }),
    shift({ t: 120 * HOUR, stage: '新卷堡', supplied }),
  ], 30 * HOUR);
  assert.equal(groups.length, 4);
  // 起点相同时，最近打过的一期在前。
  deepEqual(groups.map((group) => group.n), [1, 2, 2, 1]);
  const regular = groups[2];
  assert.equal(regular.start, 0);
  assert.equal(regular.clear, 1 / 2);
  assert.equal(regular.ongoing, true);
  deepEqual(regular.king, { name: '横纲', image: 'images/boss1_color.png', appeared: 1, defeated: 1 });
  deepEqual(regular.scales, { gold: 1, silver: 0, bronze: 2 });
  deepEqual(regular.grade, { name: '达人', point: 420 });
  assert.equal(groups[0].start, null);
  assert.equal(groups[0].first, 120 * HOUR);
  // 大型跑和同时段的鲑鱼跑分成两期，各自带着打工模式。
  assert.equal(groups[3].rule, 'BIG_RUN');
  assert.equal(regular.rule, 'REGULAR');
});
