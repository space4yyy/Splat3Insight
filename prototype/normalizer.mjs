function judgementLabel(value) {
  if (value === 'WIN') return 'WIN';
  if (value === 'LOSE' || value === 'DEEMED_LOSE') return 'LOSE';
  return value || 'UNKNOWN';
}

export function readPower(value) {
  if (Number.isFinite(value)) return value;
  if (Number.isFinite(value?.power)) return value.power;
  return null;
}

export function normalizeBankaraMode(value) {
  const normalized = String(value ?? '').toUpperCase();
  if (normalized.includes('CHALLENGE')) return 'CHALLENGE';
  if (normalized.includes('OPEN')) return 'OPEN';
  return value ? normalized : null;
}

export function resolvePower(detail) {
  const mode = detail?.vsMode?.mode;

  if (mode === 'X_MATCH') {
    return {
      value: readPower(detail.xMatch?.lastXPower),
      kind: 'x',
      path: 'xMatch.lastXPower',
      key: 'X_MATCH'
    };
  }

  if (mode === 'BANKARA') {
    const bankara = detail.bankaraMatch || {};
    const bankaraMode = normalizeBankaraMode(bankara.mode);

    if (bankaraMode === 'CHALLENGE') {
      return {
        value: readPower(bankara.weaponPower),
        kind: 'weapon',
        path: 'bankaraMatch.weaponPower',
        key: 'BANKARA:CHALLENGE'
      };
    }

    if (bankaraMode === 'OPEN') {
      return {
        value: readPower(bankara.bankaraPower),
        kind: 'team',
        path: 'bankaraMatch.bankaraPower.power',
        key: 'BANKARA:OPEN'
      };
    }

    // Keep exports from older/newer SplatNet schemas usable when the mode
    // marker is absent: prefer the team power, then the weapon power.
    const teamPower = readPower(bankara.bankaraPower);
    if (teamPower !== null) {
      return {
        value: teamPower,
        kind: 'team',
        path: 'bankaraMatch.bankaraPower.power',
        key: 'BANKARA:OPEN'
      };
    }
    return {
      value: readPower(bankara.weaponPower),
      kind: 'weapon',
      path: 'bankaraMatch.weaponPower',
      key: 'BANKARA:CHALLENGE'
    };
  }

  return {
    value: null,
    kind: null,
    path: null,
    key: mode || 'OTHER'
  };
}

export function normalizeVs(raw, fileName) {
  const detail = raw?.data?.vsHistoryDetail;
  if (!detail) return null;

  const players = detail.myTeam?.players || [];
  const player = players.find((entry) => entry?.isMyself) || detail.player || {};
  const result = player.result || null;
  const power = resolvePower(detail);

  return {
    id: detail.id || fileName,
    sourceFile: fileName,
    mode: detail.vsMode?.mode || 'BATTLE',
    bankaraMode: normalizeBankaraMode(detail.bankaraMatch?.mode),
    rule: detail.vsRule?.name || 'Unknown Rule',
    stage: detail.vsStage?.name || 'Unknown Stage',
    stageImage: detail.vsStage?.image?.url || '',
    judgement: judgementLabel(detail.judgement),
    playedTime: detail.playedTime || null,
    playerName: player.name || 'Player',
    weapon: player.weapon?.name || 'Unknown Weapon',
    weaponImage: player.weapon?.image?.url || '',
    paint: player.paint ?? null,
    power: power.value,
    powerKind: power.kind,
    powerPath: power.path,
    powerKey: power.key,
    stats: result
      ? {
          kills: result.kill ?? null,
          deaths: result.death ?? null,
          assists: result.assist ?? null,
          specials: result.special ?? null
        }
      : null
  };
}
