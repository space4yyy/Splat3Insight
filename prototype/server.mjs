import { createServer } from 'node:http';
import { readFile, readdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeVs } from './normalizer.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const publicDir = path.join(here, 'public');
const exportsDir = path.join(root, 'data', 's3s-exports');
const port = Number(process.env.PORT || 4173);

const contentTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.ttf': 'font/ttf',
  '.svg': 'image/svg+xml'
};

async function readJson(filePath) {
  return JSON.parse(await readFile(filePath, 'utf8'));
}

async function loadPreviewData() {
  const resultsPath = path.join(exportsDir, 'results');
  const names = (await readdir(resultsPath))
    .filter((name) => name.endsWith('.json'))
    .sort()
    .reverse();

  const records = [];
  const allRecords = [];
  const selectedByMode = {};
  const fallbackByMode = {};

  for (const name of names) {
    const record = normalizeVs(await readJson(path.join(resultsPath, name)), name);
    if (!record) continue;
    allRecords.push(record);
    if (record.stats && !fallbackByMode[record.mode]) fallbackByMode[record.mode] = record;
    if (record.stats && Number.isFinite(record.power) && !selectedByMode[record.mode]) {
      selectedByMode[record.mode] = record;
    }
    if (records.length < 8) records.push(record);
  }

  for (const [mode, record] of Object.entries(fallbackByMode)) {
    if (!selectedByMode[mode]) selectedByMode[mode] = record;
  }

  for (const record of new Set(Object.values(selectedByMode))) {
    if (!Number.isFinite(record.power)) continue;
    const selectedIndex = allRecords.indexOf(record);
    const previousChangedPower = allRecords
      .slice(selectedIndex + 1)
      .find((candidate) => candidate.powerKey === record.powerKey
        && Number.isFinite(candidate.power)
        && Math.abs(candidate.power - record.power) > 0.0001);
    record.powerDelta = previousChangedPower
      ? record.power - previousChangedPower.power
      : null;
  }

  const recentBattles = allRecords
    .filter((record) => ['X_MATCH', 'BANKARA'].includes(record.mode)
      && ['WIN', 'LOSE'].includes(record.judgement))
    .slice(0, 50);
  const recentWinCount = recentBattles.filter((record) => record.judgement === 'WIN').length;
  const recentLoseCount = recentBattles.filter((record) => record.judgement === 'LOSE').length;

  const selected = selectedByMode.X_MATCH || selectedByMode.BANKARA || Object.values(selectedByMode)[0] || null;

  const overviewPath = path.join(exportsDir, 'overview-20260921T053759Z.json');
  let overview = null;
  if (existsSync(overviewPath)) {
    const rawOverview = await readJson(overviewPath);
    overview = rawOverview?.[0]?.data?.regularBattleHistories?.summary || null;
  }

  return {
    selected,
    samples: {
      xMatch: selectedByMode.X_MATCH || null,
      bankara: selectedByMode.BANKARA || null
    },
    recentSummary: {
      sampleSize: recentBattles.length,
      win: recentWinCount,
      lose: recentLoseCount,
      winRate: recentBattles.length ? (recentWinCount / recentBattles.length) * 100 : 0
    },
    recent: records,
    overview,
    source: {
      directory: 'data/s3s-exports/results',
      totalFiles: names.length,
      note: 'Prototype only: read-only local s3s export data.'
    }
  };
}

async function serveStatic(request, response) {
  const requested = (request.url || '/').split('?')[0];
  const safePath = path.normalize(requested === '/' ? '/index.html' : requested).replace(/^([.][.][/\\])+/, '');
  const filePath = path.join(publicDir, safePath);

  if (!filePath.startsWith(publicDir)) {
    response.writeHead(403);
    response.end('Forbidden');
    return;
  }

  try {
    const body = await readFile(filePath);
    response.writeHead(200, {
      'Content-Type': contentTypes[path.extname(filePath)] || 'application/octet-stream',
      'Cache-Control': 'no-store'
    });
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end('Not found');
  }
}

const server = createServer(async (request, response) => {
  try {
    if (request.url?.startsWith('/api/preview')) {
      const data = await loadPreviewData();
      response.writeHead(200, {
        'Content-Type': 'application/json; charset=utf-8',
        'Cache-Control': 'no-store'
      });
      response.end(JSON.stringify(data));
      return;
    }
    await serveStatic(request, response);
  } catch (error) {
    response.writeHead(500, { 'Content-Type': 'text/plain; charset=utf-8' });
    response.end(error instanceof Error ? error.message : String(error));
  }
});

server.listen(port, '127.0.0.1', () => {
  console.log(`Splat 3 style prototype: http://127.0.0.1:${port}/?variant=a`);
  console.log(`OBS-style transparent preview: http://127.0.0.1:${port}/?variant=a&obs=1`);
});
