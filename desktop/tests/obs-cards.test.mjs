import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const ui = new URL('../ui/', import.meta.url);
const [cards, indexHtml, appJs, extensionJs, overlayCss, overlayHtml, tauriConfig] = await Promise.all([
  readFile(new URL('obs-cards.json', ui), 'utf8').then(JSON.parse),
  readFile(new URL('index.html', ui), 'utf8'),
  readFile(new URL('app.js', ui), 'utf8'),
  readFile(new URL('edition-extension.js', ui), 'utf8'),
  readFile(new URL('overlay-card.css', ui), 'utf8'),
  readFile(new URL('overlay.html', ui), 'utf8'),
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

test('app links, size labels, previews, and copy buttons all reference catalog IDs', () => {
  for (const card of cards) {
    assert.ok(indexHtml.includes(`data-obs-card-url="${card.id}"`), `${card.id} URL`);
    assert.ok(indexHtml.includes(`data-obs-card-size="${card.id}"`), `${card.id} dimensions`);
    assert.ok(indexHtml.includes(`data-obs-card-copy="${card.id}"`), `${card.id} copy action`);
    if (card.id !== 'overlay') {
      assert.ok(indexHtml.includes(`data-obs-card-preview="${card.id}"`), `${card.id} preview`);
    }
  }
  assert.match(appJs, /fetch\('\.\/obs-cards\.json'/);
  assert.doesNotMatch(appJs, /http:\/\/127\.0\.0\.1:17353\//);
  assert.doesNotMatch(indexHtml, /http:\/\/127\.0\.0\.1:17353\//);
});

test('public edition excludes private weapon and clock cards', () => {
  assert.deepEqual(cards.map((card) => card.id), ['overlay']);
  assert.doesNotMatch(indexHtml, /私房随武|时钟/);
  assert.doesNotMatch(appJs, /random-weapons|refresh_private_weapons/);
  assert.match(indexHtml, /id="edition-extension-slot"/);
  assert.match(indexHtml, /src="\.\/edition-extension\.js"/);
  assert.doesNotMatch(extensionJs, /random-weapons|私房随武|时钟/);
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
  for (const page of [overlayHtml]) {
    assert.match(page, /__OBS_CARD_WIDTH__/);
    assert.match(page, /__OBS_CARD_HEIGHT__/);
  }
});
