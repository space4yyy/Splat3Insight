const localeSources = {
  'zh-CN': {
    label: '中文',
    source: 'https://leanny.github.io/splat3/data/language/CNzh.json'
  },
  'en-US': {
    label: 'English',
    source: 'https://leanny.github.io/splat3/data/language/USen.json'
  },
  'ja-JP': {
    label: '日本語',
    source: 'https://leanny.github.io/splat3/data/language/JPja.json'
  }
};

const fallback = {
  'zh-CN': {
    language: '语言',
    title: '战斗数据，变成墨水。',
    description: '使用本地 s3s 导出数据做样式验证。只展示本场结果和必要数值，尽量不遮挡游戏画面。',
    eyebrow: 'SPLAT 3 STREAMING WIDGET / STYLE PROTOTYPE',
    exported: '份对战导出数据 · 只读',
    source: '来源',
    dataPath: '数据结构',
    playerPath: '玩家定位',
    powerPath: '分数来源',
    xPower: 'X战力',
    win: '胜利',
    lose: '失败',
    open: '开放',
    challenge: '挑战',
    unknownTime: '时间未知',
    errorNoRecord: '没有找到可用于预览的 VS 对战记录。',
    errorData: '读取本地导出数据失败',
    variantClassic: 'A — 经典紧凑',
    variantPower: 'B — 强调战力',
    variantRibbon: 'C — 极简横条',
    compareHint: '使用 ← → 比较',
    previousVariant: '上一个样式',
    nextVariant: '下一个样式'
  },
  'en-US': {
    language: 'Language',
    title: 'Battle data, turned into ink.',
    description: 'Style validation with local s3s exports. Only the result and essential numbers stay on screen.',
    eyebrow: 'SPLAT 3 STREAMING WIDGET / STYLE PROTOTYPE',
    exported: 'exported battles · read-only',
    source: 'source',
    dataPath: 'data shape',
    playerPath: 'player lookup',
    powerPath: 'Power source',
    xPower: 'X POWER',
    win: 'WIN',
    lose: 'LOSE',
    open: 'OPEN',
    challenge: 'CHALLENGE',
    unknownTime: 'TIME UNKNOWN',
    errorNoRecord: 'No VS battle record is available for preview.',
    errorData: 'Failed to read local export data',
    variantClassic: 'A — Classic compact',
    variantPower: 'B — Power focus',
    variantRibbon: 'C — Minimal ribbon',
    compareHint: 'use ← → to compare',
    previousVariant: 'Previous style',
    nextVariant: 'Next style'
  },
  'ja-JP': {
    language: '言語',
    title: 'バトルデータをインクに。',
    description: 'ローカルの s3s エクスポートでスタイルを確認します。画面を隠さないよう、結果と必要な数値だけを表示します。',
    eyebrow: 'SPLAT 3 STREAMING WIDGET / STYLE PROTOTYPE',
    exported: '件の対戦データ · 読み取り専用',
    source: 'ソース',
    dataPath: 'データ構造',
    playerPath: 'プレイヤー検索',
    powerPath: 'パワー項目',
    xPower: 'Xパワー',
    win: 'WIN',
    lose: 'LOSE',
    open: 'オープン',
    challenge: 'チャレンジ',
    unknownTime: '時刻不明',
    errorNoRecord: 'プレビューできるバトルデータがありません。',
    errorData: 'ローカルのエクスポートデータを読み込めませんでした',
    variantClassic: 'A — クラシック',
    variantPower: 'B — パワー重視',
    variantRibbon: 'C — ミニマル',
    compareHint: '← → で比較',
    previousVariant: '前のスタイル',
    nextVariant: '次のスタイル'
  }
};

const namespaceByType = {
  rule: 'CommonMsg/VS/VSRuleName',
  stage: 'CommonMsg/VS/VSStageName',
  weapon: 'CommonMsg/Weapon/WeaponName_Main'
};

const modeKeyByCode = {
  X_MATCH: 'XMatch',
  BANKARA: 'Bankara',
  REGULAR: 'Regular',
  FEST: 'FestRegular'
};

const catalogs = new Map();
const reverseMaps = new Map();
let activeLocale = 'zh-CN';

function stripMarkup(value) {
  return String(value ?? '')
    .replaceAll(/\[size=[^\]]+\]/g, '')
    .replaceAll(/\[color=[^\]]+\]/g, '')
    .replaceAll(/\[group=[^\]]+\]/g, '')
    .replaceAll(/\[page break\]/g, ' ')
    .replaceAll(/\s+/g, ' ')
    .trim();
}

function reverseMapFor(locale, type) {
  const cacheKey = `${locale}:${type}`;
  if (reverseMaps.has(cacheKey)) return reverseMaps.get(cacheKey);

  const namespace = namespaceByType[type];
  const values = catalogs.get(locale)?.[namespace] || {};
  const map = new Map();
  for (const [key, value] of Object.entries(values)) {
    const normalized = stripMarkup(value);
    if (!map.has(normalized)) map.set(normalized, key);
  }
  reverseMaps.set(cacheKey, map);
  return map;
}

async function fetchCatalog(locale) {
  if (catalogs.has(locale)) return catalogs.get(locale);

  const response = await fetch(localeSources[locale].source, { cache: 'no-store' });
  if (!response.ok) throw new Error(`language catalog HTTP ${response.status}`);
  const catalog = await response.json();
  catalogs.set(locale, catalog);
  return catalog;
}

function localFallback(type, value, locale) {
  const maps = {
    mode: {
      'zh-CN': { X_MATCH: 'X比赛', BANKARA: '蛮颓比赛', REGULAR: '一般比赛', FEST: '祭典比赛' },
      'en-US': { X_MATCH: 'X Battle', BANKARA: 'Anarchy Battle', REGULAR: 'Regular Battle', FEST: 'Splatfest Battle' },
      'ja-JP': { X_MATCH: 'Xマッチ', BANKARA: 'バンカラマッチ', REGULAR: 'レギュラーマッチ', FEST: 'フェスマッチ' }
    },
    rule: {
      'zh-CN': { 'Clam Blitz': '真格蛤蜊', 'Turf War': '占地对战', 'Splat Zones': '真格区域', 'Tower Control': '真格塔楼', Rainmaker: '真格鱼虎' },
      'en-US': {},
      'ja-JP': { 'Clam Blitz': 'ガチアサリ', 'Turf War': 'ナワバリバトル', 'Splat Zones': 'ガチエリア', 'Tower Control': 'ガチヤグラ', Rainmaker: 'ガチホコバトル' }
    }
  };
  return maps[type]?.[locale]?.[value] || value;
}

function localizedByKey(type, value, locale) {
  const catalog = catalogs.get(locale);
  if (!catalog) return localFallback(type, value, locale);
  const namespace = namespaceByType[type];
  const key = reverseMapFor('en-US', type).get(stripMarkup(value));
  return key ? stripMarkup(catalog[namespace]?.[key] || value) : localFallback(type, value, locale);
}

export function availableLocales() {
  return Object.entries(localeSources).map(([id, value]) => ({ id, ...value }));
}

export function currentLocale() {
  return activeLocale;
}

export function t(key) {
  return fallback[activeLocale]?.[key] || fallback['en-US'][key] || key;
}

export async function setLocale(locale) {
  const next = localeSources[locale] ? locale : 'zh-CN';
  try {
    await Promise.all([fetchCatalog(next), next === 'en-US' ? Promise.resolve() : fetchCatalog('en-US')]);
  } catch (error) {
    console.warn(`Failed to load ${next} language catalog; using local fallback.`, error);
  }
  activeLocale = next;
  return activeLocale;
}

export function localizeRecord(record) {
  const modeKey = modeKeyByCode[record.mode];
  const modeCatalog = catalogs.get(activeLocale)?.['CommonMsg/MatchMode'];
  const modeLabel = modeKey
    ? stripMarkup(modeCatalog?.[modeKey] || localFallback('mode', record.mode, activeLocale))
    : localFallback('mode', record.mode, activeLocale);

  return {
    ...record,
    modeLabel,
    modeClass: modeKeyByCode[record.mode]
      ? `mode-${record.mode.toLowerCase().replaceAll('_', '-')}`
      : 'mode-default',
    ruleLabel: localizedByKey('rule', record.rule, activeLocale),
    stageLabel: localizedByKey('stage', record.stage, activeLocale),
    weaponLabel: localizedByKey('weapon', record.weapon, activeLocale)
  };
}
