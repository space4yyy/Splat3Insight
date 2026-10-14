import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const ui = new URL('../ui/', import.meta.url);
const [cards, indexHtml, appJs, overlayCss, overlayHtml, salmonRunHtml, tauriConfig] = await Promise.all([
  readFile(new URL('obs-cards.json', ui), 'utf8').then(JSON.parse),
  readFile(new URL('index.html', ui), 'utf8'),
  readFile(new URL('app.js', ui), 'utf8'),
  readFile(new URL('overlay-card.css', ui), 'utf8'),
  readFile(new URL('overlay.html', ui), 'utf8'),
  readFile(new URL('salmon-run.html', ui), 'utf8'),
  readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8').then(JSON.parse),
]);

test('OBS card catalog has unique routes and positive integer dimensions', () => {
  const ids = cards.map((card) => card.id);
  const routes = cards.map((card) => card.route);
  assert.equal(new Set(ids).size, cards.length);
  assert.equal(new Set(routes).size, cards.length);
  for (const card of cards) {
    assert.ok(Number.isInteger(card.width) && card.width > 0, `${card.id} width`);
    assert.ok(Number.isInteger(card.height) && card.height > 0, `${card.id} height`);
  }
});

test('OBS card links, size labels, and copy buttons use catalog IDs', () => {
  for (const card of cards) {
    assert.ok(indexHtml.includes(`data-obs-card-url="${card.id}"`), `${card.id} URL`);
    assert.ok(indexHtml.includes(`data-obs-card-size="${card.id}"`), `${card.id} dimensions`);
    assert.ok(indexHtml.includes(`data-obs-card-copy="${card.id}"`), `${card.id} copy action`);
  }
  assert.match(appJs, /fetch\('\.\/obs-cards\.json'/);
  assert.doesNotMatch(appJs, /http:\/\/127\.0\.0\.1:17353\//);
  assert.doesNotMatch(indexHtml, /http:\/\/127\.0\.0\.1:17353\//);
});

test('card catalog is loaded by the app without hardcoded service addresses', () => {
  assert.match(appJs, /fetch\('\.\/obs-cards\.json'/);
  assert.doesNotMatch(appJs, /install_card_package|set_card_package_enabled|remove_card_package/);
  assert.doesNotMatch(indexHtml, /导入卡片包|card-package/);
});

test('Tauri CSP allows the app to fetch its same-origin card catalog', () => {
  const connectSource = tauriConfig.app.security.csp
    .split(';')
    .map((directive) => directive.trim())
    .find((directive) => directive.startsWith('connect-src '));
  assert.ok(connectSource, 'connect-src directive exists');
  assert.match(connectSource, /(?:^|\s)'self'(?:\s|$)/);
});

test('OBS page templates receive dimensions from the card catalog', () => {
  assert.match(overlayCss, /width: var\(--obs-card-width\)/);
  assert.match(overlayCss, /height: var\(--obs-card-height\)/);
  for (const page of [overlayHtml, salmonRunHtml]) {
    assert.match(page, /__OBS_CARD_WIDTH__/);
    assert.match(page, /__OBS_CARD_HEIGHT__/);
  }
});
