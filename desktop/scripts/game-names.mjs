// 从游戏文本生成 ui/game-names.js，面板用它把繁体、英文名称对照成简体中文：
//   node scripts/game-names.mjs <目录>
// 目录里放 https://leanny.github.io/splat3/data/language/ 的 CNzh.json、TWzh.json、EUen.json。
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const KINDS = [
  { kind: 'stages', table: 'CommonMsg/VS/VSStageName' },
  { kind: 'rules', table: 'CommonMsg/VS/VSRuleName', skip: /_2L$/ },
  { kind: 'weapons', table: 'CommonMsg/Weapon/WeaponName_Main', skip: /_(Coop|Msn|Sdodr)$/ },
  { kind: 'coopStages', table: 'CommonMsg/Coop/CoopStageName' },
  { kind: 'grades', table: 'CommonMsg/Coop/CoopGrade', skip: /^Grade_Arbeiter_/ },
  { kind: 'bosses', table: 'CommonMsg/Coop/CoopEnemy' },
  { kind: 'events', table: 'CommonMsg/Glossary', only: /^CoopEvent_/ },
];
// 假名、换行、排版标记和问号是未翻译的占位文本，不是游戏里显示的名称。
const PLACEHOLDER = /[ぁ-ゖァ-ヺ]|\n|\[|^[-?？]+$/;

const directory = process.argv[2];
if (!directory) {
  console.error('用法：node scripts/game-names.mjs <语言文件目录>');
  process.exit(1);
}
const load = async (name) => JSON.parse(await readFile(path.join(directory, `${name}.json`), 'utf8'));
const [simplified, traditional, english] = await Promise.all(['CNzh', 'TWzh', 'EUen'].map(load));

const output = {};
for (const { kind, table, skip, only } of KINDS) {
  const rows = [];
  const seen = new Set();
  for (const [key, text] of Object.entries(simplified[table] ?? {})) {
    if (skip?.test(key) || (only && !only.test(key))) continue;
    const row = [text, traditional[table]?.[key], english[table]?.[key]].map((value) => (typeof value === 'string' ? value.trim() : ''));
    if (row.some((value) => !value || PLACEHOLDER.test(value))) continue;
    const signature = row.join('\u0000');
    if (seen.has(signature)) continue;
    seen.add(signature);
    rows.push(row);
  }
  if (!rows.length) throw new Error(`${table} 里没有可用的名称`);

  // 一个写法只能对应一个简体名称（包括恰好和另一条的简体名称相同）；有歧义就不对照，原样显示。
  const targets = new Map();
  for (const row of rows) {
    for (const value of row) targets.set(value, (targets.get(value) ?? new Set()).add(row[0]));
  }
  for (const row of rows) {
    for (const column of [1, 2]) {
      if (targets.get(row[column]).size > 1) {
        console.warn(`${kind}：“${row[column]}”对应多个简体名称，不做对照`);
        row[column] = null;
      }
    }
  }
  output[kind] = rows;
}

const lines = [
  '// 由 desktop/scripts/game-names.mjs 生成，请勿手改。',
  '// 来源：Leanny 整理的游戏文本 https://leanny.github.io/splat3/data/language/（CNzh、TWzh、EUen）。',
  '// 每行是同一条游戏文本的 [简体中文, 繁体中文, 英文]；null 表示这个写法有歧义，不做对照。',
  'window.Splat3GameNames = Object.freeze({',
  ...Object.entries(output).flatMap(([kind, rows]) => [`  ${kind}: [`, ...rows.map((row) => `    ${JSON.stringify(row)},`), '  ],']),
  '});',
  '',
];
const target = new URL('../ui/game-names.js', import.meta.url);
await writeFile(target, lines.join('\n'));
console.log(Object.entries(output).map(([kind, rows]) => `${kind} ${rows.length}`).join('，'));
