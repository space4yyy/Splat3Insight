import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const ui = new URL('../ui/', import.meta.url);

async function loadScripts(...names) {
  const context = vm.createContext({ window: {} });
  for (const name of names) vm.runInContext(await readFile(new URL(name, ui), 'utf8'), context);
  return context.window;
}

const { Splat3DashboardData: data, Splat3GameNames: gameNames } = await loadScripts('game-names.js', 'dashboard-data.js');
const { Splat3OverlayCard: cards } = await loadScripts('game-names.js', 'overlay-card-renderer.js');

function battleRow(overrides) {
  return {
    id: overrides.id,
    played_at_ms: overrides.played_at_ms,
    mode: 'X_MATCH',
    bankara_mode: null,
    rule_code: 'AREA',
    judgement: 'WIN',
    power: null,
    power_kind: 'x',
    ...overrides,
  };
}

test('season windows start on the first of March, June, September and December (UTC)', () => {
  assert.equal(data.seasonStart(Date.UTC(2026, 9, 16, 12)), Date.UTC(2026, 8, 1));
  assert.equal(data.seasonStart(Date.UTC(2026, 2, 1, 0, 0, 1)), Date.UTC(2026, 2, 1));
  assert.equal(data.seasonStart(Date.UTC(2026, 1, 20)), Date.UTC(2025, 11, 1));
  assert.equal(data.seasonStart(Date.UTC(2026, 11, 31)), Date.UTC(2026, 11, 1));
});

test('X power changes compare only within the same rule', () => {
  const { battles, xSettlements } = data.prepare({
    battles: [
      battleRow({ id: 'a1', played_at_ms: 1, rule_code: 'AREA', power: 2000 }),
      battleRow({ id: 'g1', played_at_ms: 2, rule_code: 'GOAL', power: 2300 }),
      battleRow({ id: 'a2', played_at_ms: 3, rule_code: 'AREA', power: 2000 }),
      battleRow({ id: 'a3', played_at_ms: 4, rule_code: 'AREA', power: 2012.5 }),
      battleRow({ id: 'a4', played_at_ms: 5, rule_code: 'AREA', power: 2012.5 }),
      battleRow({ id: 'l1', played_at_ms: 6, rule_code: 'LOFT', power: null }),
    ],
    coop: [],
  });
  const delta = Object.fromEntries(battles.map((battle) => [battle.id, battle.delta]));
  assert.deepEqual(delta, { a1: 0, g1: 0, a2: 0, a3: 12.5, a4: 12.5, l1: null });
  assert.deepEqual(Array.from(xSettlements, (point) => `${point.rule}:${point.value}`), ['AREA:2000', 'GOAL:2300', 'AREA:2012.5']);
});

test('Challenge power changes compare by weapon and Open power has no change', () => {
  const challenge = (id, t, weapon, power) => battleRow({ id, played_at_ms: t, mode: 'BANKARA', bankara_mode: 'CHALLENGE', weapon_id: weapon, power, power_kind: 'weapon' });
  const { battles } = data.prepare({
    battles: [
      challenge('c1', 1, 'w1', 1900),
      challenge('c2', 2, 'w2', 1500),
      challenge('c3', 3, 'w1', 1888.2),
      battleRow({ id: 'o1', played_at_ms: 4, mode: 'BANKARA', bankara_mode: 'OPEN', power: 1700, power_kind: 'team' }),
    ],
    coop: [],
  });
  const delta = Object.fromEntries(battles.map((battle) => [battle.id, battle.delta]));
  assert.deepEqual(delta, { c1: 0, c2: 0, c3: -11.8, o1: null });
});

test('win rate counts decided battles only and undated rows are dropped', () => {
  const { battles } = data.prepare({
    battles: [
      battleRow({ id: 'w', played_at_ms: 3, judgement: 'WIN', kills: 6, deaths: 2 }),
      battleRow({ id: 'l', played_at_ms: 1, judgement: 'LOSE', kills: 2, deaths: 4 }),
      battleRow({ id: 'd', played_at_ms: 2, judgement: 'DEEMED_LOSE' }),
      battleRow({ id: 'x', played_at_ms: 4, judgement: 'DRAW' }),
      battleRow({ id: 'u', played_at_ms: null, judgement: 'WIN' }),
    ],
    coop: [],
  });
  assert.deepEqual(Array.from(battles, (battle) => battle.id), ['l', 'd', 'w', 'x']);
  const summary = data.summarize(battles);
  assert.equal(summary.n, 4);
  assert.equal(summary.wins, 1);
  assert.equal(summary.losses, 2);
  assert.equal(summary.wr, 1 / 3);
  assert.equal(summary.kd, 8 / 6);
});

test('shift playtime is estimated from the waves played and the king wave', () => {
  const shift = data.normalizeShift({
    id: 's1',
    played_at_ms: 1,
    cleared: true,
    result_wave: 0,
    boss_appeared: true,
    boss_defeated: false,
    waves: [
      { wave_number: 1, water_level: 0, cleared: true, boss_wave: false },
      { wave_number: 2, water_level: 1, cleared: true, boss_wave: false },
      { wave_number: 3, water_level: 2, cleared: true, boss_wave: false },
      { wave_number: 4, water_level: 1, cleared: false, boss_wave: true },
    ],
  });
  assert.equal(shift.duration, 60 + 3 * 100 + 100);
  assert.deepEqual(Array.from(shift.waves, (wave) => wave.water), ['干潮', '普通', '满潮']);
  assert.equal(shift.kingDefeated, false);
});

test('names follow the game text and unknown names stay unchanged', () => {
  assert.equal(data.gameName('stages', '魚露遺跡'), '鱼露遗迹');
  assert.equal(data.gameName('stages', "Um'ami Ruins"), '鱼露遗迹');
  assert.equal(data.gameName('stages', '鱼露遗迹'), '鱼露遗迹');
  assert.equal(data.gameName('weapons', ' 斯普拉射擊槍 '), '斯普拉射击枪');
  assert.equal(data.gameName('weapons', 'Splattershot'), '斯普拉射击枪');
  assert.equal(data.gameName('rules', '真格魚虎對戰'), '真格鱼虎对战');
  assert.equal(data.gameName('weapons', 'Some Future Weapon'), 'Some Future Weapon');
  assert.equal(data.gameName('weapons', ''), null);
  assert.equal(data.gameName('weapons', null), null);
});

test('the shared lookup maps game text and returns null for unknown names', () => {
  assert.equal(gameNames.lookup('awards', '塗墨點數 No.1'), '涂墨点数 No.1');
  assert.equal(gameNames.lookup('awards', '#1 Turf Inker'), '涂墨点数 No.1');
  assert.equal(gameNames.lookup('grades', 'Profreshional +2'), '达人+2');
  assert.equal(gameNames.lookup('weapons', 'Not A Weapon'), null);
  assert.equal(gameNames.lookup('nope', 'Splattershot'), null);
});

test('each Traditional or English game name maps to one Simplified name', () => {
  for (const [kind, rows] of Object.entries(gameNames.text)) {
    const targets = new Map();
    for (const [simplified, ...others] of rows) {
      assert.ok(simplified, `${kind} row has a Simplified name`);
      for (const text of [simplified, ...others].filter(Boolean)) {
        targets.set(text, (targets.get(text) ?? new Set()).add(simplified));
      }
    }
    for (const [text, names] of targets) {
      const sources = rows.filter((row) => row.slice(1).includes(text));
      if (sources.length) assert.equal(names.size, 1, `${kind}: ${text}`);
    }
  }
});

test('stage and weapon names merge by id across languages', () => {
  const { battles } = data.prepare({
    battles: [
      battleRow({ id: 'b1', played_at_ms: 1, stage_id: 'stage-1', stage_name: '溫泉花大峽谷', weapon_id: 'weapon-40', weapon_name: '斯普拉射擊槍' }),
      battleRow({ id: 'b2', played_at_ms: 2, stage_id: 'stage-1', stage_name: 'Scorch Gorge', weapon_id: 'weapon-40', weapon_name: 'Splattershot' }),
      battleRow({ id: 'b3', played_at_ms: 3, stage_id: 'stage-9', stage_name: '未來場地', weapon_id: 'weapon-9', weapon_name: '未來武器' }),
      battleRow({ id: 'b4', played_at_ms: 4, stage_id: 'stage-9', stage_name: 'Future Stage', weapon_id: 'weapon-9', weapon_name: 'Future Weapon' }),
    ],
    coop: [],
  });
  assert.deepEqual(Array.from(battles, (battle) => battle.stage), ['温泉花大峡谷', '温泉花大峡谷', 'Future Stage', 'Future Stage']);
  assert.deepEqual(Array.from(battles, (battle) => battle.weapon), ['斯普拉射击枪', '斯普拉射击枪', 'Future Weapon', 'Future Weapon']);
});

test('Salmon Run stages, grades and kings use the game text', () => {
  const { shifts } = data.prepare({
    battles: [],
    coop: [
      { id: 'c1', played_at_ms: 1, stage_id: 'coop-6', stage_name: "Marooner's Bay", grade_name: 'Eggsecutive VP', boss_name: 'Cohozuna', waves: [{ wave_number: 1, water_level: 2, event_wave: 'Cohock Charge', cleared: true }] },
      { id: 'c2', played_at_ms: 2, stage_id: 'coop-1', stage_name: '鮭壩', grade_name: 'Profreshional +2', waves: [] },
      { id: 'c3', played_at_ms: 3, stage_id: 'coop-1', stage_name: 'Spawning Grounds', grade_name: '達人+1', waves: [] },
    ],
  });
  assert.deepEqual(Array.from(shifts, (shift) => shift.stage), ['漂浮落难船', '鲑坝', '鲑坝']);
  assert.deepEqual(Array.from(shifts, (shift) => shift.gradeName), ['传说', '达人+2', '达人+1']);
  assert.equal(shifts[0].kingName, '横纲');
  assert.equal(shifts[0].waves[0].event, '大胖鱼大量出现');
});

test('OBS cards show official names and keep their short rule labels', () => {
  assert.equal(cards.labels.rule('真格魚虎對戰'), '真格鱼虎');
  assert.equal(cards.labels.rule('占地對戰'), '占地对战');
  assert.equal(cards.labels.rule('Tricolor Turf War'), '三色夺宝对战');
  assert.equal(cards.labels.rule('Mystery Rule'), 'Mystery Rule');
  assert.equal(cards.labels.rule(''), '—');
  assert.equal(cards.labels.coopStage('Spawning Grounds'), '鲑坝');
  assert.equal(cards.labels.coopStage('Wahoo World'), '醋饭海洋世界');
  assert.equal(cards.labels.coopStage('Future Site'), 'Future Site');
  assert.equal(cards.labels.grade('Profreshional +2'), '达人+2');
  assert.equal(cards.labels.grade('Eggsecutive VP'), '传说');
});

test('combined OBS routes show the selected type with the newest record', () => {
  const battle = (time) => ({ selected: { played_time: time } });
  const salmon = (time) => ({ salmon_run: { played_time: time } });
  assert.equal(cards.newestCardType({
    bankaramatch: battle('2026-10-16T10:00:00Z'),
    salmonrun: salmon('2026-10-16T11:00:00Z'),
  }), 'salmonrun');
  assert.equal(cards.newestCardType({
    xmatch: battle('2026-10-16T12:00:00Z'),
    salmonrun: salmon('2026-10-16T11:00:00Z'),
  }), 'xmatch');
  assert.equal(cards.newestCardType({ xmatch: { selected: null }, bankaramatch: battle('2026-10-15T09:00:00Z') }), 'bankaramatch');
  assert.equal(cards.newestCardType({ xmatch: { selected: null } }), null);
});

test('record log filters, sorts with blanks last, and pages by 20', () => {
  const rows = [
    { id: 'a', t: 1, stage: '鲑坝', rule: 'AREA', weapon: 'W1', win: true, lose: false, kills: 5, paint: 900 },
    { id: 'b', t: 2, stage: '鲑坝', rule: 'LOFT', weapon: 'W2', win: false, lose: true, kills: null, paint: 1200 },
    { id: 'c', t: 3, stage: '新卷堡', rule: 'AREA', weapon: 'W1', win: false, lose: false, kills: 9, paint: 700 },
  ];
  const ids = (list) => Array.from(list, (row) => row.id);
  assert.deepEqual(ids(data.filterRecords(rows, { rule: 'AREA', stage: 'all', weapon: 'all', result: 'all' }, false)), ['a', 'c']);
  assert.deepEqual(ids(data.filterRecords(rows, { result: 'lose' }, false)), ['b']);
  assert.deepEqual(ids(data.filterRecords(rows, { result: 'win', stage: '鲑坝' }, false)), ['a']);
  const shifts = [{ id: 's1', t: 1, stage: '鲑坝', cleared: true }, { id: 's2', t: 2, stage: '鲑坝', cleared: false }];
  assert.deepEqual(ids(data.filterRecords(shifts, { result: 'lose', rule: 'AREA' }, true)), ['s2']);

  assert.deepEqual(ids(data.sortRecords(rows, { key: 't', dir: 'desc' })), ['c', 'b', 'a']);
  assert.deepEqual(ids(data.sortRecords(rows, { key: 'kills', dir: 'desc' })), ['c', 'a', 'b']);
  assert.deepEqual(ids(data.sortRecords(rows, { key: 'kills', dir: 'asc' })), ['a', 'c', 'b']);
  assert.deepEqual(ids(data.sortRecords(rows, { key: 'paint', dir: 'asc' })), ['c', 'a', 'b']);

  const many = Array.from({ length: 45 }, (_, index) => ({ id: `r${index}` }));
  const last = data.pageOf(many, 5);
  assert.equal(last.page, 2);
  assert.equal(last.pages, 3);
  assert.equal(last.first, 40);
  assert.equal(last.items.length, 5);
  assert.equal(data.pageOf([], 0).pages, 1);
});
