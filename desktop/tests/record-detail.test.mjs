import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const ui = new URL('../ui/', import.meta.url);
const context = vm.createContext({ window: {}, URL });
for (const name of ['game-names.js', 'record-detail.js']) vm.runInContext(await readFile(new URL(name, ui), 'utf8'), context);
const detail = context.window.Splat3RecordDetail;
const plain = (value) => JSON.parse(JSON.stringify(value));

const player = (name, weapon, kill, assist, death, special, paint, me = false) => ({
  name, isMyself: me, weapon: { name: weapon }, paint, result: { kill, assist, death, special },
});

test('turf battles show paint ratios, official weapon and award names', () => {
  const battle = plain(detail.battleDetail({
    duration: 180,
    knockout: 'NEITHER',
    myTeam: { result: { paintRatio: 0.523 }, players: [player('我', '斯普拉射擊槍', 8, 2, 3, 4, 1180, true), player('队友', 'Splattershot', 3, 1, 5, 2, 900)] },
    otherTeams: [{ result: { paintRatio: 0.477 }, players: [player('对手', '北齋', 6, 0, 4, 3, 1000)] }],
    awards: [{ name: '塗墨點數 No.1' }, { name: '#1 Splatter' }],
  }));
  assert.equal(battle.score.kind, 'paint');
  assert.equal(Math.round(battle.score.mine * 10) / 10, 52.3);
  assert.equal(battle.teams.length, 2);
  assert.deepEqual(battle.teams[0].players.map((entry) => entry.weapon), ['斯普拉射击枪', '斯普拉射击枪']);
  assert.equal(battle.teams[0].players[0].me, true);
  assert.equal(battle.teams[1].players[0].weapon, '北斋');
  assert.equal(battle.awards[0], '涂墨点数 No.1');
  assert.equal(battle.knockout, null);
  assert.equal(battle.duration, 180);
});

test('ranked battles show counts and knockouts; tricolor has two opposing teams', () => {
  const ranked = plain(detail.battleDetail({ knockout: 'WIN', myTeam: { result: { score: 100 }, players: [] }, otherTeams: [{ result: { score: 32 }, players: [] }] }));
  assert.deepEqual(ranked.score, { kind: 'count', mine: 100, theirs: [32] });
  assert.equal(ranked.knockout, 'WIN');
  const tricolor = plain(detail.battleDetail({ myTeam: { result: { paintRatio: 0.4 } }, otherTeams: [{ result: { paintRatio: 0.35 } }, { result: { paintRatio: 0.25 } }] }));
  assert.equal(tricolor.teams.length, 3);
  assert.equal(tricolor.score.theirs.length, 2);
  // 缺少字段时留空，不猜。
  const empty = plain(detail.battleDetail({}));
  assert.equal(empty.score, null);
  assert.deepEqual(empty.awards, []);
  assert.equal(empty.teams[0].players.length, 0);
});

test('cleared shifts list every wave and the king as EX', () => {
  const shift = plain(detail.shiftDetail({
    resultWave: 0,
    waveResults: [
      { waveNumber: 1, waterLevel: 1, eventWave: null, deliverNorm: 21, teamDeliverCount: 30 },
      { waveNumber: 2, waterLevel: 2, eventWave: { name: 'Cohock Charge' }, deliverNorm: 23, teamDeliverCount: 25 },
      { waveNumber: 3, waterLevel: 0, eventWave: { name: 'Fog' }, deliverNorm: 25, teamDeliverCount: 26 },
      { waveNumber: 4, waterLevel: 1, eventWave: null, deliverNorm: null, teamDeliverCount: null },
    ],
    bossResult: { hasDefeatBoss: true, boss: { name: 'Cohozuna' } },
    myResult: { goldenDeliverCount: 18, deliverCount: 1203, rescueCount: 2, rescuedCount: 1, defeatEnemyCount: 14 },
    scale: { gold: 1, silver: 2, bronze: 3 },
    afterGrade: { name: 'Eggsecutive VP' },
    afterGradePoint: 999,
    dangerRate: 2.33,
  }));
  assert.deepEqual(shift.waves.map((wave) => [wave.number, wave.water, wave.event, wave.cleared]), [
    [1, '普通', null, true],
    [2, '满潮', '大胖鱼大量出现', true],
    [3, '干潮', '雾', true],
  ]);
  assert.deepEqual(shift.boss, { name: '横纲', image: null, defeated: true });
  assert.equal(shift.mine.goldenEggs, 18);
  assert.equal(shift.grade, '传说');
  assert.deepEqual(shift.scale, { gold: 1, silver: 2, bronze: 3 });
});

test('failed shifts mark the failing wave and keep missing data empty', () => {
  const shift = plain(detail.shiftDetail({
    resultWave: 2,
    waveResults: [
      { waveNumber: 1, waterLevel: 0, deliverNorm: 21, teamDeliverCount: 24 },
      { waveNumber: 2, waterLevel: 1, deliverNorm: 23, teamDeliverCount: 17 },
    ],
    bossResult: null,
    scale: null,
  }));
  assert.deepEqual(shift.waves.map((wave) => wave.cleared), [true, false]);
  assert.equal(shift.boss, null);
  assert.equal(shift.scale, null);
  assert.equal(shift.mine.goldenEggs, null);
  assert.equal(shift.grade, null);
});

const image = (path) => `https://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/${path}.png?Expires=1&Signature=s&Key-Pair-Id=k`;

test('image keys drop the signature and only accept Nintendo resource links', () => {
  assert.equal(detail.imageKey(image('weapon_illust/abc_0')), 'v3/weapon_illust/abc_0.png');
  assert.equal(detail.imageKey('https://example.com/resources/prod/v3/a/b.png'), null);
  assert.equal(detail.imageKey('http://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/a/b.png'), null);
  assert.equal(detail.imageKey('https://api.lp1.av5ja.srv.nintendo.net/resources/prod/../../etc/passwd'), null);
  assert.equal(detail.imageKey(null), null);
});

test('battle details carry stage, weapon, nameplate, team color and my gear images', () => {
  const battle = plain(detail.battleDetail({
    vsStage: { image: { url: image('stage_img/gorge') } },
    myTeam: {
      color: { r: 0.9, g: 0.8, b: 0.1, a: 1 },
      result: { paintRatio: 0.5 },
      players: [{
        name: '我', isMyself: true, byname: ' 溫泉花大峽谷的廣域標記槍操作者 ',
        weapon: { name: '斯普拉射擊槍', image: { url: image('weapon_illust/shot') }, subWeapon: { name: 'Curling Bomb', image: { url: image('sub_img/curling') } }, specialWeapon: { name: 'Ultra Stamp', image: { url: image('special_img/stamp') } } },
        nameplate: { background: { image: { url: image('npl_img/plate') }, textColor: { r: 1, g: 1, b: 1, a: 1 } }, badges: [{ image: { url: image('badge_img/one') } }, null] },
        result: { kill: 1, assist: 0, death: 0, special: 1 }, paint: 500,
      }],
    },
    otherTeams: [{ color: { r: 0.2, g: 0.3, b: 0.9, a: 1 }, result: { paintRatio: 0.5 }, players: [] }],
    player: {
      headGear: { name: 'Annaki Beret', brand: { name: 'Annaki' }, image: { url: image('gear_img/beret') }, primaryGearPower: { name: 'Ink Saver (Main)', image: { url: image('skill_img/ink') } }, additionalGearPowers: [{ name: 'Swim Speed Up', image: { url: image('skill_img/swim') } }] },
    },
  }));
  assert.equal(battle.stageImage, image('stage_img/gorge'));
  assert.equal(battle.teams[0].color, 'rgba(230, 204, 26, 1)');
  const me = battle.teams[0].players[0];
  assert.equal(me.weaponImage, image('weapon_illust/shot'));
  assert.equal(me.sub.name, '冰壶炸弹');
  assert.equal(me.special.name, '终极印章');
  assert.equal(me.special.image, image('special_img/stamp'));
  assert.deepEqual(me.nameplate, { title: '温泉花大峡谷的广域标记枪操作者', background: image('npl_img/plate'), textColor: 'rgba(255, 255, 255, 1)', badges: [image('badge_img/one')] });
  assert.equal(battle.gear.length, 1);
  assert.equal(battle.gear[0].slot, '头部');
  assert.equal(battle.gear[0].image, image('gear_img/beret'));
  assert.equal(battle.gear[0].primary.image, image('skill_img/ink'));
  assert.equal(battle.gear[0].additional.length, 1);
  assert.notEqual(battle.gear[0].primary.name, 'Ink Saver (Main)');
});

test('shift details list supplied weapons, crew, enemies and wave specials with images', () => {
  const member = (name, me) => ({
    player: { name, byname: me ? 'Splatlandian Youth' : '', nameplate: { background: { image: { url: image('npl_img/p') } }, badges: [] } },
    weapons: [{ name: 'Splattershot', image: { url: image('weapon_illust/shot') } }, { name: 'Grizzco Blaster', image: { url: image('weapon_illust/bear') } }],
    specialWeapon: { name: 'Booyah Bomb', image: { url: image('special_img/booyah') } },
    goldenDeliverCount: me ? 18 : 12, goldenAssistCount: 3, deliverCount: 900, rescueCount: 1, rescuedCount: 2, defeatEnemyCount: 7,
  });
  const shift = plain(detail.shiftDetail({
    coopStage: { image: { url: image('stage_img/grounds') } },
    weapons: [{ name: 'Splattershot', image: { url: image('weapon_illust/shot') } }],
    resultWave: 0,
    waveResults: [{ waveNumber: 1, waterLevel: 1, deliverNorm: 20, teamDeliverCount: 25, specialWeapons: [{ name: 'Booyah Bomb', image: { url: image('special_img/booyah') } }] }],
    bossResult: { hasDefeatBoss: false, boss: { name: 'Horrorboros', image: { url: image('coop_enemy_img/king') } } },
    myResult: member('我', true),
    memberResults: [member('队友', false)],
    enemyResults: [{ enemy: { name: 'Steelhead', image: { url: image('coop_enemy_img/steelhead') } }, defeatCount: 3, teamDefeatCount: 7, popCount: 8 }],
  }));
  assert.equal(shift.stageImage, image('stage_img/grounds'));
  assert.equal(shift.supplied[0].name, '斯普拉射击枪');
  assert.equal(shift.members.length, 2);
  assert.equal(shift.members[0].me, true);
  assert.equal(shift.members[0].nameplate.title, '蛮颓的年轻人');
  assert.equal(shift.members[1].nameplate.title, null);
  assert.deepEqual(shift.members[0].weapons.map((weapon) => weapon.name), ['斯普拉射击枪', '熊先生印章爆破枪']);
  assert.equal(shift.members[1].special.image, image('special_img/booyah'));
  assert.deepEqual(shift.enemies, [{ name: '炸弹鱼', image: image('coop_enemy_img/steelhead'), defeated: 3, teamDefeated: 7, appeared: 8 }]);
  assert.equal(shift.waves[0].specials.length, 1);
  assert.deepEqual(shift.boss, { name: '辰龙', image: image('coop_enemy_img/king'), defeated: false });
});

test('titles split into official adjective and subject text and become Simplified Chinese', () => {
  const { title } = context.window.Splat3GameNames;
  assert.equal(title('Scorch Gorge Sploosh-o-matic User'), '温泉花大峡谷的广域标记枪操作者');
  assert.equal(title('温泉花大峡谷的 广域标记枪操作者'), '温泉花大峡谷的广域标记枪操作者');
  // 名词的另一种写法单独对照；中文数据里这个形容词自带空格。
  assert.equal(title('Thank You for Princess'), 'Thank You For 公主');
  assert.equal(title('Scorch Gorge'), null);
  assert.equal(title('かけだしの イカ'), null);
});

test('nameplate titles come from byname; unknown titles stay as they are, a title alone makes a plain nameplate', () => {
  const plateOf = (fields) => plain(detail.battleDetail({ myTeam: { players: [{ name: '我', ...fields }] } })).teams[0].players[0].nameplate;
  assert.deepEqual(plateOf({ byname: 'かけだしの イカ' }), { title: 'かけだしの イカ', background: null, textColor: null, badges: [] });
  assert.equal(plateOf({ nameplate: { badges: [] } }).title, null);
  assert.equal(plateOf({ byname: '   ' }), null);
  assert.equal(plateOf({ byname: 42 }), null);
});
