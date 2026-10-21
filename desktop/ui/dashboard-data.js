(() => {
  'use strict';

  // 面板数据的纯函数部分：把事实行整理成图表用的结构，不碰页面。
  const MODE_KEY = { X_MATCH: 'x', BANKARA: 'anarchy', EVENT_MATCH: 'event', REGULAR: 'regular', PRIVATE_BATTLE: 'private' };
  const LOSSES = new Set(['LOSE', 'DEEMED_LOSE', 'EXEMPTED_LOSE']);
  const WATER = ['干潮', '普通', '满潮'];
  const isNum = (value) => typeof value === 'number' && Number.isFinite(value);
  const round1 = (value) => Math.round(value * 10) / 10;

  // SplatNet 返回的对战名称跟着任天堂账号的语言，打工名称固定是英文。
  // 按游戏文本（game-names.js）把繁体和英文换成同一条文本的简体中文；查不到的原样显示。
  const officialName = (kind, text) => window.Splat3GameNames?.lookup(kind, text) ?? null;
  const trimmed = (name) => (typeof name === 'string' ? name.trim() : '');
  function gameName(kind, name) {
    const text = trimmed(name);
    return text ? officialName(kind, text) ?? text : null;
  }

  // 同一场地或武器在不同时期的记录里可能是不同语言，按 id 归并成一个名称：
  // 优先用能在游戏文本里查到的写法，都查不到时用最近一条记录的写法。
  function namesById(rows, idKey, nameKey, kind) {
    const names = new Map();
    for (const row of rows) {
      const id = row[idKey];
      const text = trimmed(row[nameKey]);
      if (!id || !text) continue;
      const known = officialName(kind, text);
      if (known) names.set(id, { name: known, known: true });
      else if (!names.get(id)?.known) names.set(id, { name: text, known: false });
    }
    return new Map([...names].map(([id, entry]) => [id, entry.name]));
  }
  const nameOf = (names, id, name, kind) => (id && names?.has(id) ? names.get(id) : gameName(kind, name));

  // 图片链接在 facts.images 里只存一份，行里的图片字段是它的下标；直接给出链接的也照常使用。
  const linkIn = (links) => (value) => (typeof value === 'string' ? value : Number.isInteger(value) ? links?.[value] ?? null : null);
  const ownLinks = linkIn([]);

  function normalizeBattle(row, names, link = ownLinks) {
    return {
      id: row.id,
      t: row.played_at_ms,
      mode: MODE_KEY[row.mode] || 'other',
      bankara: row.bankara_mode === 'CHALLENGE' ? 'challenge' : row.bankara_mode === 'OPEN' ? 'open' : null,
      rule: row.rule_code || null,
      ruleName: gameName('rules', row.rule_name),
      stage: nameOf(names?.stages, row.stage_id, row.stage_name, 'stages') || '未知场地',
      weapon: nameOf(names?.weapons, row.weapon_id, row.weapon_name, 'weapons') || '未知武器',
      weaponId: row.weapon_id || row.weapon_name || null,
      weaponImage: link(row.weapon_image),
      sub: gameName('subWeapons', row.sub_weapon_name),
      subImage: link(row.sub_weapon_image),
      special: gameName('specialWeapons', row.special_weapon_name),
      specialImage: link(row.special_weapon_image),
      win: row.judgement === 'WIN',
      lose: LOSSES.has(row.judgement),
      duration: isNum(row.duration_seconds) ? row.duration_seconds : null,
      knockout: row.knockout === 'WIN' || row.knockout === 'LOSE',
      kills: isNum(row.kills) ? row.kills : null,
      assists: isNum(row.assists) ? row.assists : null,
      deaths: isNum(row.deaths) ? row.deaths : null,
      specials: isNum(row.specials) ? row.specials : null,
      paint: isNum(row.paint) ? row.paint : null,
      power: isNum(row.power) ? row.power : null,
      powerKind: row.power_kind || null,
      delta: null,
    };
  }

  // 头目鲑鱼的 id 是 base64 的 “CoopEnemy-23”，取出编号（和 OBS 卡片的图片编号一致）。
  function enemyNumber(id) {
    if (typeof id !== 'string' || !id || typeof atob !== 'function') return null;
    try {
      const match = /-(\d+)$/.exec(atob(id));
      return match ? Number(match[1]) : null;
    } catch {
      return null;
    }
  }

  // 打工时长没有原始数据，按波次估算：开场 60 秒，每波 100 秒，头目鲑鱼再加 100 秒。
  function normalizeShift(row, names, link = ownLinks) {
    const waves = (row.waves || [])
      .filter((wave) => !wave.boss_wave)
      .map((wave) => ({ water: WATER[wave.water_level] ?? '未知潮位', cleared: wave.cleared === true, event: gameName('events', wave.event_wave) }));
    const playedWaves = waves.length || (row.cleared ? 3 : isNum(row.result_wave) ? row.result_wave : 1);
    const kingAppeared = row.boss_appeared === true;
    return {
      id: row.id,
      t: row.played_at_ms,
      rule: row.rule || null,
      stage: nameOf(names?.coopStages, row.stage_id, row.stage_name, 'coopStages') || '未知场地',
      cleared: row.cleared === true,
      failedWave: row.cleared === false && isNum(row.result_wave) ? row.result_wave : null,
      waves,
      kingAppeared,
      kingDefeated: row.boss_defeated === true,
      kingName: gameName('bosses', row.boss_name),
      kingId: enemyNumber(row.boss_id),
      goldenEggs: isNum(row.golden_eggs) ? row.golden_eggs : null,
      defeated: isNum(row.defeat_enemy_count) ? row.defeat_enemy_count : null,
      powerEggs: isNum(row.power_eggs) ? row.power_eggs : null,
      rescues: isNum(row.rescue_count) ? row.rescue_count : null,
      rescued: isNum(row.rescued_count) ? row.rescued_count : null,
      gradeName: gameName('grades', row.grade_name),
      gradePoint: isNum(row.grade_point) ? row.grade_point : null,
      danger: isNum(row.danger_rate) ? row.danger_rate : null,
      scales: { gold: row.scale_gold, silver: row.scale_silver, bronze: row.scale_bronze },
      goldenAssists: isNum(row.golden_assists) ? row.golden_assists : null,
      supplied: (row.supplied_weapons || []).map((weapon) => ({
        name: gameName('weapons', weapon?.name),
        image: link(weapon?.image),
      })),
      rotationStart: isNum(row.rotation_start_ms) ? row.rotation_start_ms : null,
      rotationEnd: isNum(row.rotation_end_ms) ? row.rotation_end_ms : null,
      duration: 60 + playedWaves * 100 + (kingAppeared ? 100 : 0),
    };
  }

  // 战力变化只和同一赛道比较：X 赛按规则，挑战按武器；新赛道从 ±0 开始。
  // 和 OBS 卡片一样，取同赛道里最近一次不同的战力作对比。
  function assignPowerDeltas(battles) {
    const tracks = new Map();
    for (const battle of battles) {
      let key = null;
      if (battle.mode === 'x' && battle.powerKind === 'x' && battle.rule) key = `x:${battle.rule}`;
      else if (battle.mode === 'anarchy' && battle.bankara === 'challenge' && battle.weaponId) key = `weapon:${battle.weaponId}`;
      if (!key || battle.power == null) continue;
      const track = tracks.get(key);
      if (!track) {
        tracks.set(key, { last: battle.power, previous: null });
        battle.delta = 0;
        continue;
      }
      if (Math.abs(battle.power - track.last) > 0.0001) {
        track.previous = track.last;
        track.last = battle.power;
      }
      battle.delta = track.previous == null ? 0 : round1(battle.power - track.previous);
    }
  }

  // X 战力按规则分开，每次数值变化记一个点；没有 X 战力的规则没有点。
  function deriveXSettlements(battles) {
    const last = new Map();
    const points = [];
    for (const battle of battles) {
      if (battle.mode !== 'x' || battle.powerKind !== 'x' || !battle.rule || battle.power == null) continue;
      if (last.get(battle.rule) === battle.power) continue;
      last.set(battle.rule, battle.power);
      points.push({ t: battle.t, rule: battle.rule, value: battle.power });
    }
    return points;
  }

  function prepare(facts) {
    const byTime = (a, b) => a.played_at_ms - b.played_at_ms;
    const battleRows = (facts?.battles || []).filter((row) => isNum(row.played_at_ms)).sort(byTime);
    const coopRows = (facts?.coop || []).filter((row) => isNum(row.played_at_ms)).sort(byTime);
    const names = {
      stages: namesById(battleRows, 'stage_id', 'stage_name', 'stages'),
      weapons: namesById(battleRows, 'weapon_id', 'weapon_name', 'weapons'),
      coopStages: namesById(coopRows, 'stage_id', 'stage_name', 'coopStages'),
    };
    const link = linkIn(Array.isArray(facts?.images) ? facts.images : []);
    const battles = battleRows.map((row) => normalizeBattle(row, names, link));
    const shifts = coopRows.map((row) => normalizeShift(row, names, link));
    assignPowerDeltas(battles);
    return { battles, shifts, xSettlements: deriveXSettlements(battles), coopRecord: normalizeCoopRecord(facts?.coop_record, link) };
  }

  // SplatNet「鲑鱼跑记录」的累计击倒数量：巨大鲑鱼（enemies）和头目鲑鱼（bosses），按 SplatNet 给的顺序。
  // 还没查到时为 null。
  function normalizeCoopRecord(record, link = ownLinks) {
    if (!record || typeof record !== 'object') return null;
    const rows = (list) => (Array.isArray(list) ? list : [])
      .filter((row) => isNum(row?.defeat_count))
      .map((row) => ({ id: isNum(row.enemy_id) ? row.enemy_id : null, name: gameName('bosses', row.name), image: link(row.image), count: row.defeat_count }));
    return { fetchedAt: isNum(record.fetched_at_ms) ? record.fetched_at_ms : null, enemies: rows(record.enemies), bosses: rows(record.bosses) };
  }

  // 赛季从每年 3、6、9、12 月 1 日（UTC）开始。
  function seasonStart(nowMs) {
    const date = new Date(nowMs);
    const month = [11, 8, 5, 2].find((candidate) => candidate <= date.getUTCMonth());
    return month === undefined ? Date.UTC(date.getUTCFullYear() - 1, 11, 1) : Date.UTC(date.getUTCFullYear(), month, 1);
  }

  // 胜率只算有胜负的对局，和 OBS 卡片的近期胜率一致。
  function summarize(list) {
    let wins = 0;
    let losses = 0;
    let kills = 0;
    let deaths = 0;
    let fought = 0;
    let paint = 0;
    let painted = 0;
    for (const battle of list) {
      if (battle.win) wins += 1;
      else if (battle.lose) losses += 1;
      if (battle.kills != null && battle.deaths != null) {
        kills += battle.kills;
        deaths += battle.deaths;
        fought += 1;
      }
      if (battle.paint != null) {
        paint += battle.paint;
        painted += 1;
      }
    }
    const decided = wins + losses;
    return {
      n: list.length,
      wins,
      losses,
      wr: decided ? wins / decided : null,
      kd: deaths ? kills / deaths : null,
      avgK: fought ? kills / fought : null,
      avgD: fought ? deaths / fought : null,
      avgPaint: painted ? paint / painted : null,
    };
  }

  // 对战记录页：页面内筛选（"all" 表示不限），按列排序（空值排在最后），每页 20 条。
  const LOG_PAGE_SIZE = 20;
  function filterRecords(rows, filters, salmon) {
    const pass = (key, value) => !filters?.[key] || filters[key] === 'all' || filters[key] === value;
    return rows.filter((item) => {
      if (!pass('stage', item.stage)) return false;
      if (filters?.result === 'win' && !(salmon ? item.cleared : item.win)) return false;
      if (filters?.result === 'lose' && !(salmon ? !item.cleared : item.lose)) return false;
      if (salmon) return true;
      return pass('rule', item.rule) && pass('weapon', item.weapon);
    });
  }
  function sortRecords(rows, sort) {
    const sign = sort?.dir === 'asc' ? 1 : -1;
    const key = sort?.key || 't';
    return [...rows].sort((a, b) => {
      const left = a[key];
      const right = b[key];
      if (left == null || right == null) return (left == null) - (right == null) || b.t - a.t;
      return sign * (left - right) || b.t - a.t;
    });
  }
  function pageOf(rows, page, size = LOG_PAGE_SIZE) {
    const pages = Math.max(1, Math.ceil(rows.length / size));
    const current = Math.min(Math.max(0, page || 0), pages - 1);
    return { page: current, pages, first: current * size, items: rows.slice(current * size, current * size + size) };
  }

  function average(list, pick) {
    const values = list.map(pick).filter(isNum);
    return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : null;
  }

  window.Splat3DashboardData = Object.freeze({
    WATER,
    isNum,
    round1,
    prepare,
    normalizeBattle,
    normalizeShift,
    normalizeCoopRecord,
    assignPowerDeltas,
    deriveXSettlements,
    gameName,
    seasonStart,
    summarize,
    average,
    LOG_PAGE_SIZE,
    filterRecords,
    sortRecords,
    pageOf,
  });
})();
