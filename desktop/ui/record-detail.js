(() => {
  'use strict';

  // 对战记录详情：从归档的原始 JSON 取出比分、队伍、装备、奖牌、波次和图片链接，纯函数，不碰页面。
  // 名称查官方游戏文本，查不到原样显示；字段缺失时留空，由页面显示“—”或不显示图片。
  const WATER = ['干潮', '普通', '满潮'];
  const IMAGE_HOST = 'api.lp1.av5ja.srv.nintendo.net';
  const IMAGE_PREFIX = '/resources/prod/';
  const num = (value) => (typeof value === 'number' && Number.isFinite(value) ? value : null);
  function official(kind, name) {
    const text = typeof name === 'string' ? name.trim() : '';
    if (!text) return null;
    return window.Splat3GameNames?.lookup(kind, text) ?? text;
  }
  const list = (value) => (Array.isArray(value) ? value : []);
  const knockoutOf = (raw) => (raw?.knockout === 'WIN' || raw?.knockout === 'LOSE' ? raw.knockout : null);

  // 图片在本地缓存里的路径：任天堂资源服务器上的图片去掉签名后的路径，和桌面端的规则一致。
  function imageKey(url) {
    if (typeof url !== 'string') return null;
    let parsed;
    try {
      parsed = new URL(url);
    } catch {
      return null;
    }
    if (parsed.protocol !== 'https:' || parsed.hostname !== IMAGE_HOST || parsed.port || !parsed.pathname.startsWith(IMAGE_PREFIX)) return null;
    const key = parsed.pathname.slice(IMAGE_PREFIX.length);
    const segments = key.split('/');
    return segments.length >= 2 && segments.every((segment) => /^[A-Za-z0-9_.-]+$/.test(segment) && !segment.startsWith('.')) ? key : null;
  }
  const imageOf = (node) => (typeof node?.image?.url === 'string' ? node.image.url : null);
  // SplatNet 的颜色是 0–1 的 RGBA。
  function colorOf(color) {
    const channels = ['r', 'g', 'b'].map((key) => num(color?.[key]));
    if (channels.some((value) => value == null)) return null;
    const alpha = num(color.a) ?? 1;
    return `rgba(${channels.map((value) => Math.round(value * 255)).join(', ')}, ${alpha})`;
  }
  // 称号（byname）是 SplatNet 按账号语言拼好的一句，拆得开就换成简体，拆不开原样显示。
  function nameplateOf(player) {
    const plate = player?.nameplate;
    const byname = typeof player?.byname === 'string' ? player.byname.trim() : '';
    const title = byname ? window.Splat3GameNames?.title(byname) ?? byname : null;
    if (!plate && !title) return null;
    return {
      title,
      background: imageOf(plate?.background),
      textColor: colorOf(plate?.background?.textColor),
      badges: list(plate?.badges).map((badge) => imageOf(badge)).filter(Boolean),
    };
  }
  const named = (kind, node) => (node ? { name: official(kind, node.name), image: imageOf(node) } : null);

  function teamPlayers(team) {
    return list(team?.players).map((player) => ({
      name: typeof player?.name === 'string' && player.name ? player.name : null,
      me: player?.isMyself === true,
      weapon: official('weapons', player?.weapon?.name),
      weaponImage: imageOf(player?.weapon),
      sub: named('subWeapons', player?.weapon?.subWeapon),
      special: named('specialWeapons', player?.weapon?.specialWeapon),
      nameplate: nameplateOf(player),
      kills: num(player?.result?.kill),
      assists: num(player?.result?.assist),
      deaths: num(player?.result?.death),
      specials: num(player?.result?.special),
      paint: num(player?.paint),
    }));
  }

  // 比分：涂地类看涂地比例（百分比），真格规则看计数；三色比赛有两支对方队伍。
  function battleScore(raw) {
    const mine = raw?.myTeam?.result;
    const others = list(raw?.otherTeams).map((team) => team?.result);
    if (num(mine?.paintRatio) != null) {
      return { kind: 'paint', mine: mine.paintRatio * 100, theirs: others.map((result) => (num(result?.paintRatio) == null ? null : result.paintRatio * 100)) };
    }
    if (num(mine?.score) != null) {
      return { kind: 'count', mine: mine.score, theirs: others.map((result) => num(result?.score)) };
    }
    return null;
  }

  // 我这场的装备：头、衣、鞋各自的主能力和副能力。
  const GEAR_SLOTS = [['headGear', '头部'], ['clothingGear', '衣服'], ['shoesGear', '鞋子']];
  function myGear(raw) {
    return GEAR_SLOTS.map(([key, slot]) => {
      const gear = raw?.player?.[key];
      if (!gear) return null;
      return {
        slot,
        name: official(key, gear.name),
        brand: official('brands', gear.brand?.name),
        image: imageOf(gear) || (typeof gear.originalImage?.url === 'string' ? gear.originalImage.url : null),
        primary: named('abilities', gear.primaryGearPower),
        additional: list(gear.additionalGearPowers).map((power) => named('abilities', power)).filter(Boolean),
      };
    }).filter(Boolean);
  }

  function battleDetail(raw) {
    return {
      stageImage: imageOf(raw?.vsStage),
      score: battleScore(raw),
      knockout: knockoutOf(raw),
      duration: num(raw?.duration),
      teams: [
        { mine: true, color: colorOf(raw?.myTeam?.color), players: teamPlayers(raw?.myTeam) },
        ...list(raw?.otherTeams).map((team) => ({ mine: false, color: colorOf(team?.color), players: teamPlayers(team) })),
      ],
      gear: myGear(raw),
      awards: list(raw?.awards).map((award) => official('awards', award?.name)).filter(Boolean),
    };
  }

  function memberOf(result, me) {
    return {
      name: typeof result?.player?.name === 'string' && result.player.name ? result.player.name : null,
      me,
      nameplate: nameplateOf(result?.player),
      weapons: list(result?.weapons).map((weapon) => named('weapons', weapon)).filter(Boolean),
      special: named('specialWeapons', result?.specialWeapon),
      goldenEggs: num(result?.goldenDeliverCount),
      goldenAssists: num(result?.goldenAssistCount),
      powerEggs: num(result?.deliverCount),
      rescues: num(result?.rescueCount),
      rescued: num(result?.rescuedCount),
      defeated: num(result?.defeatEnemyCount),
    };
  }

  // 打工：resultWave 为 0 表示通关；否则是失败的那一波，之前的波次都已通过。
  // 头目鲑鱼那一波没有配额，单独作为 EX 显示。
  function shiftDetail(raw) {
    const resultWave = num(raw?.resultWave);
    const waves = list(raw?.waveResults)
      .filter((wave) => wave?.deliverNorm != null)
      .map((wave) => {
        const number = num(wave.waveNumber);
        return {
          number,
          water: WATER[num(wave.waterLevel)] ?? null,
          event: official('events', wave.eventWave?.name),
          quota: num(wave.deliverNorm),
          delivered: num(wave.teamDeliverCount),
          cleared: resultWave === 0 || (number != null && resultWave != null && number < resultWave),
          specials: list(wave.specialWeapons).map((special) => named('specialWeapons', special)).filter(Boolean),
        };
      });
    const bossResult = raw?.bossResult && typeof raw.bossResult === 'object' ? raw.bossResult : null;
    const scale = raw?.scale && typeof raw.scale === 'object' ? raw.scale : null;
    const my = raw?.myResult || {};
    return {
      stageImage: imageOf(raw?.coopStage),
      supplied: list(raw?.weapons).map((weapon) => named('weapons', weapon)).filter(Boolean),
      waves,
      boss: bossResult ? { name: official('bosses', bossResult.boss?.name), image: imageOf(bossResult.boss), defeated: bossResult.hasDefeatBoss === true } : null,
      mine: {
        goldenEggs: num(my.goldenDeliverCount),
        goldenAssists: num(my.goldenAssistCount),
        powerEggs: num(my.deliverCount),
        rescues: num(my.rescueCount),
        rescued: num(my.rescuedCount),
        defeated: num(my.defeatEnemyCount),
      },
      members: raw?.myResult ? [memberOf(raw.myResult, true), ...list(raw?.memberResults).map((result) => memberOf(result, false))] : [],
      enemies: list(raw?.enemyResults).map((result) => ({
        name: official('bosses', result?.enemy?.name),
        image: imageOf(result?.enemy),
        defeated: num(result?.defeatCount),
        teamDefeated: num(result?.teamDefeatCount),
        appeared: num(result?.popCount),
      })),
      scale: scale ? { gold: num(scale.gold), silver: num(scale.silver), bronze: num(scale.bronze) } : null,
      danger: num(raw?.dangerRate),
      grade: official('grades', raw?.afterGrade?.name),
      gradePoint: num(raw?.afterGradePoint),
    };
  }

  window.Splat3RecordDetail = Object.freeze({ battleDetail, shiftDetail, imageKey });
})();
