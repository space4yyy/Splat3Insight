import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const ui = new URL('../ui/', import.meta.url);
const [cards, indexHtml, appJs, dashboardJs, dashboardDataJs, overlayCss, overlayHtml, salmonRunHtml, clockHtml, defeatCountsHtml, tauriConfig, rendererJs] = await Promise.all([
  readFile(new URL('obs-cards.json', ui), 'utf8').then(JSON.parse),
  readFile(new URL('index.html', ui), 'utf8'),
  readFile(new URL('app.js', ui), 'utf8'),
  readFile(new URL('dashboard.js', ui), 'utf8'),
  readFile(new URL('dashboard-data.js', ui), 'utf8'),
  readFile(new URL('overlay-card.css', ui), 'utf8'),
  readFile(new URL('overlay.html', ui), 'utf8'),
  readFile(new URL('salmon-run.html', ui), 'utf8'),
  readFile(new URL('clock.html', ui), 'utf8'),
  readFile(new URL('defeat-counts.html', ui), 'utf8'),
  readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8').then(JSON.parse),
  readFile(new URL('overlay-card-renderer.js', ui), 'utf8'),
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

test('OBS card checkboxes generate catalog-based URLs and copy actions', () => {
  assert.ok(indexHtml.includes('id="obs-options"'));
  assert.ok(indexHtml.includes('id="obs-url"'));
  assert.ok(indexHtml.includes('id="obs-preview"'));
  assert.match(appJs, /app\.obs\.catalog\.map\(\(card\) => el\('label'/);
  assert.match(appJs, /'data-obs-card-option'/);
  assert.match(appJs, /function selectedObsRoute\(\)/);
  assert.match(appJs, /data-copy-overlay-url/);
  assert.match(appJs, /fetch\('\.\/obs-cards\.json'/);
  for (const source of [appJs, indexHtml, dashboardJs, dashboardDataJs]) {
    assert.doesNotMatch(source, /127\.0\.0\.1:17353/);
  }
});

test('the dashboard loads the game name table before its data module runs', () => {
  const namesAt = indexHtml.indexOf('src="./game-names.js"');
  const dataAt = indexHtml.indexOf('src="./dashboard-data.js"');
  assert.ok(namesAt > 0 && namesAt < dataAt);
});

test('OBS card selection always keeps one option and defaults to automatic', () => {
  assert.match(appJs, /if \(!known\.length \|\| known\.includes\('overlay'\)\) return \['overlay'\];/);
  assert.match(appJs, /app\.obs\.selected = normalizeSelection\(app\.obs\.selected\.filter/);
});

test('cards with their own page are picked alone instead of joining the combined overlay address', () => {
  // 只有 /overlay?参数 形式的数据卡片能组合；自动、击倒数量和时钟的地址各自独立。
  const standalone = cards.filter((card) => !card.route.includes('?')).map((card) => card.id);
  assert.deepEqual(standalone, ['overlay', 'defeat-counts', 'clock']);
  assert.match(appJs, /const obsStandalone = \(id\) => !obsCardOf\(id\)\?\.route\.includes\('\?'\);/);
  assert.match(appJs, /if \(standalone\) return obsCardOf\(standalone\)\.route;/);
  assert.match(appJs, /obsStandalone\(id\) \? \[id\] :/);
});

test('the clock card shows local time as HH:mm:ss in the number font', () => {
  const context = vm.createContext({ window: {} });
  vm.runInContext(rendererJs, context);
  const { clockText } = context.window.Splat3OverlayCard;
  assert.equal(clockText(new Date(2026, 9, 19, 16, 31, 9)), '16:31:09');
  assert.equal(clockText(new Date(2027, 0, 5, 7, 3, 2)), '07:03:02');
  assert.equal(clockText(new Date(2027, 0, 6, 0, 0, 0)), '00:00:00');
  const clock = cards.find((card) => card.id === 'clock');
  assert.equal(clock.route, '/clock');
  assert.equal(clock.page, 'clock');
  const rule = overlayCss.match(/\n\.clock-card \{([^}]*)\}/)?.[1] ?? '';
  assert.match(rule, /font-family: "BlitzMain", "DFP GBZY7", sans-serif;/);
  // BlitzMain 没有等宽数字，每位数字放进等宽格子，秒数变化时整行不晃。
  assert.match(overlayCss, /\.clock-card__digit \{[^}]*width: \.6em;/);
  assert.match(clockHtml, /Splat3OverlayCard\.updateClock\(root\)/);
  assert.doesNotMatch(clockHtml, /\/api\/overlay/);
  assert.match(appJs, /preview\?\.querySelector\('\.overlay-card, \.salmon-run-card, \.clock-card, \.defeat-count-card'\)/);
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

test('battle, Salmon Run, defeat count and clock cards share one background that OBS custom CSS can override', () => {
  const rule = (selector) => overlayCss.match(new RegExp(`\\n${selector.replace('.', '\\.')} \\{([^}]*)\\}`))?.[1] ?? '';
  for (const selector of ['.overlay-card', '.salmon-run-card', '.defeat-count-card', '.clock-card']) {
    assert.match(rule(selector), /background: var\(--obs-card-background, #383838\);/, selector);
    assert.doesNotMatch(rule(selector), /border:/, selector);
  }
  // 默认值只写在 var() 里，OBS 自定义 CSS 设置的变量才能稳定生效。
  assert.doesNotMatch(overlayCss.replace(/\/\*[\s\S]*?\*\//g, ''), /--obs-card-background:/);
});

test('card transparency produces a copyable CSS line for the shared card background', () => {
  assert.ok(indexHtml.includes('id="obs-transparency"'));
  assert.ok(indexHtml.includes('id="obs-css"'));
  assert.match(appJs, /'data-copy-overlay-css'/);
  assert.match(appJs, /--obs-card-background: \$\{obsBackground\(transparency\)\};/);
  // 只改透明度，颜色和卡片默认背景 #383838 一致。
  assert.match(appJs, /const OBS_CARD_RGB = '56, 56, 56';/);
  assert.match(overlayCss, /var\(--obs-card-background, #383838\)/);
});

test('deleting an account keeps local data unless the user opts in, and offers an export first', () => {
  // 删除本地数据默认不勾选；勾选后才把 deleteLocalData 传给后端。
  assert.match(indexHtml, /<input type="checkbox" id="delete-local-data">/);
  assert.match(appJs, /deleteLocalData\.checked = false;/);
  assert.match(appJs, /invoke\('remove_account', \{ accountId: target\.id, deleteLocalData: withData \}\)/);
  assert.match(indexHtml, /id="delete-export-button"/);
  assert.match(appJs, /invoke\('export_history', \{ accountId: target\.id \}\)/);
});

test('the overview current shift reuses the OBS Salmon Run totals row', () => {
  // 同一期工在后端判断，面板直接用卡片第三行的总计，不再自己按连续同场地计算。
  assert.match(rendererJs, /\$\{SALMON_TOTALS_HTML\}/);
  assert.match(rendererJs, /updateSalmonTotals,\s+newestCardType,/);
  assert.match(dashboardJs, /Splat3OverlayCard\?\.updateSalmonTotals\(totals, \{ salmon_run: salmonRun \}\)/);
  assert.match(appJs, /dashboard\.setSalmonRun\(overlay\?\.salmon_run\)/);
  assert.match(indexHtml, /<p class="subhead">当前打工<\/p>/);
  assert.doesNotMatch(dashboardJs, /口径/);
});

test('the record log page loads after the dashboard and reads raw records on demand', () => {
  assert.match(indexHtml, /<button type="button" data-view="log">对战记录<\/button>/);
  const order = ['dashboard.js', 'record-detail.js', 'records.js', 'app.js'].map((name) => indexHtml.indexOf(`src="./${name}"`));
  assert.ok(order.every((at, index) => at > 0 && (index === 0 || at > order[index - 1])), String(order));
  assert.ok(indexHtml.includes('id="record-drawer"'));
  assert.match(appJs, /log: \['对战记录'/);
});

test('record images load through the local image cache protocol', async () => {
  const imageSource = tauriConfig.app.security.csp
    .split(';')
    .map((directive) => directive.trim())
    .find((directive) => directive.startsWith('img-src '));
  assert.match(imageSource, /(?:^|\s)splat3img:(?:\s|$)/);
  assert.match(imageSource, /http:\/\/splat3img\.localhost/);
  // 不直接加载任天堂的图片链接：链接约三个月后失效，统一走本地缓存。
  assert.doesNotMatch(imageSource, /nintendo/);
  const recordsJs = await readFile(new URL('records.js', ui), 'utf8');
  assert.match(recordsJs, /'splat3img:\/\/localhost\/'/);
  assert.match(recordsJs, /\?src=\$\{encodeURIComponent\(url\)\}/);
});

test('the main window accepts the click that activates it on macOS', () => {
  // 打开浏览器登录后回到应用，第一次点击（例如“取消”）不能只用来激活窗口。
  const main = tauriConfig.app.windows.find((window) => window.label === 'main');
  assert.equal(main.acceptFirstMouse, true);
});

test('OBS page templates receive dimensions from the card catalog', () => {
  assert.match(overlayCss, /width: var\(--obs-card-width\)/);
  assert.match(overlayCss, /height: var\(--obs-card-height\)/);
  for (const page of [overlayHtml, salmonRunHtml, clockHtml, defeatCountsHtml]) {
    assert.match(page, /__OBS_CARD_WIDTH__/);
    assert.match(page, /__OBS_CARD_HEIGHT__/);
  }
});
