import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const ui = new URL('../ui/', import.meta.url);
const [catalog, rendererJs, gameNamesJs, pageHtml, appJs, previewCss] = await Promise.all([
  readFile(new URL('obs-cards.json', ui), 'utf8').then(JSON.parse),
  readFile(new URL('overlay-card-renderer.js', ui), 'utf8'),
  readFile(new URL('game-names.js', ui), 'utf8'),
  readFile(new URL('defeat-counts.html', ui), 'utf8'),
  readFile(new URL('app.js', ui), 'utf8'),
  readFile(new URL('app.css', ui), 'utf8'),
]);

// 只实现卡片用到的那部分 DOM；动画帧和计时器由测试手动推进。
function setup() {
  const frames = new Map();
  let lastFrame = 0;
  const node = (tag) => {
    const classes = new Set();
    const attributes = {};
    return {
      tag, children: [], className: '', title: '', alt: '', hidden: false, style: {}, dataset: {}, textContent: '', listeners: {},
      get src() { return attributes.src; },
      set src(value) { attributes.src = value; },
      getAttribute: (name) => attributes[name] ?? null,
      append(...nodes) { this.children.push(...nodes); },
      replaceChildren(...nodes) { this.children = nodes; },
      addEventListener(type, listener) { this.listeners[type] = listener; },
      // OBS 页面用到的：按类名找直接子节点；卡片的宽度取内联样式，高度是样式表里的 64。
      querySelector(selector) { return this.children.find((child) => `.${child.className}` === selector) ?? null; },
      get offsetWidth() { return this.hidden ? 0 : parseFloat(this.style.width) || 0; },
      get offsetHeight() { return this.hidden || this.className !== 'defeat-count-card' ? 0 : 64; },
      classList: {
        add: (name) => classes.add(name),
        remove: (name) => classes.delete(name),
        toggle: (name, on) => (on ? classes.add(name) : classes.delete(name)),
        contains: (name) => classes.has(name),
      },
    };
  };
  const window = {
    requestAnimationFrame: (step) => frames.set(++lastFrame, step) && lastFrame,
    cancelAnimationFrame: (id) => frames.delete(id),
  };
  const context = vm.createContext({
    window,
    document: { createElement: node },
    setTimeout: () => 0,
    clearTimeout: () => {},
  });
  vm.runInContext(gameNamesJs, context);
  vm.runInContext(rendererJs, context);
  const runFrame = (time) => {
    const due = [...frames.values()];
    frames.clear();
    due.forEach((step) => step(time));
  };
  return { cards: window.Splat3OverlayCard, root: node('main'), runFrame, context };
}

// OBS 页面自己的脚本（没有 src 的那段），放进同一个上下文，按给定的窗口大小和接口数据跑。
const pageScript = pageHtml.match(/<script>([\s\S]*?)<\/script>/)[1];
function openPage({ width, height, data }) {
  const { context, root } = setup();
  Object.assign(context.window, { innerWidth: width, innerHeight: height, addEventListener: () => {}, setInterval: () => 0 });
  Object.assign(context.document, { querySelector: (selector) => (selector === '#root' ? root : null), fonts: { load: () => Promise.resolve() } });
  context.fetch = async () => ({ ok: true, json: async () => data() });
  vm.runInContext(pageScript, context);
  return { root, update: () => context.update() };
}

const record = (steelhead = 4812) => ({
  enemies: [
    { id: 4, name: 'Steelhead', image: '/images/coop-enemy/4', count: steelhead },
    { id: 17, name: 'Griller', image: '/images/coop-enemy/17', count: 0 },
  ],
  bosses: [
    { id: 23, name: 'Cohozuna', image: '/images/coop-enemy/23', count: 171 },
    { id: 30, name: 'Triumvirate', image: null, count: 12 },
    { id: 99, name: 'Someone New', image: null, count: 3 },
  ],
});

test('defeat count items keep the SplatNet order with official names and bundled King images', () => {
  const { cards } = setup();
  const items = cards.defeatCountItems(record(), (url) => `cache:${url}`);
  // 渲染器在另一个 vm 上下文里，数组先换成本上下文的再比较。
  assert.deepEqual(Array.from(items.enemies, (item) => [item.name, item.image, item.count]), [
    ['炸弹鱼', 'cache:/images/coop-enemy/4', 4812],
    ['烤架鱼', 'cache:/images/coop-enemy/17', 0],
  ]);
  // 头目鲑鱼没有图片时用卡片自带的彩色图；查不到官方名称的原样显示。
  assert.deepEqual(Array.from(items.bosses, (item) => [item.name, item.image, item.fallback]), [
    ['横纲', 'cache:/images/coop-enemy/23', './images/boss1_color.png'],
    ['头目联合', './images/boss_all_color.png', './images/boss_all_color.png'],
    ['Someone New', null, null],
  ]);
  assert.equal(cards.defeatCountItems({ enemies: [{ id: 4, name: 'Steelhead', count: null }] }).enemies.length, 0);
});

test('the defeat count card is one strip with a divider between Boss and King Salmonids', () => {
  const { cards, root } = setup();
  assert.equal(cards.updateDefeatCounts(root, null), false);
  assert.equal(cards.updateDefeatCounts(root, record()), true);
  const [card] = root.children;
  assert.equal(card.className, 'defeat-count-card');
  assert.deepEqual(card.children.map((child) => child.className), [
    'defeat-count-card__item', 'defeat-count-card__item', 'defeat-count-card__divider',
    'defeat-count-card__item', 'defeat-count-card__item', 'defeat-count-card__item',
  ]);
  const counts = card.children.filter((child) => child.tag === 'span' && child.children.length).map((item) => item.children[1]);
  assert.deepEqual(counts.map((count) => count.textContent), ['4,812', '0', '171', '12', '3']);
  assert.deepEqual(counts.map((count) => count.classList.contains('is-zero')), [false, true, false, false, false]);
  const [steelhead] = card.children;
  assert.equal(steelhead.title, '炸弹鱼');
  assert.equal(steelhead.children[0].src, '/images/coop-enemy/4');
  // 没有图片的鲑鱼留出空位；头目鲑鱼的图片读不到时换成自带的图，再读不到也留空位。
  const unknown = card.children[5].children[0];
  assert.equal(unknown.style.visibility, 'hidden');
  const cohozuna = card.children[3].children[0];
  cohozuna.listeners.error();
  assert.equal(cohozuna.src, './images/boss1_color.png');
  cohozuna.listeners.error();
  assert.equal(cohozuna.style.visibility, 'hidden');

  // 没有记录时隐藏卡片，有记录后再显示。
  assert.equal(cards.updateDefeatCounts(root, { enemies: [], bosses: [] }), false);
  assert.equal(card.hidden, true);
  cards.updateDefeatCounts(root, record());
  assert.equal(root.children[0], card);
  assert.equal(card.hidden, false);
});

test('new defeat counts roll in place and keep their thousands separators', () => {
  const { cards, root, runFrame } = setup();
  cards.updateDefeatCounts(root, record(4812));
  const card = root.children[0];
  const steelhead = card.children[0].children[1];
  cards.updateDefeatCounts(root, record(4830));
  // 只改数字，不重建卡片，图片不会重新加载。
  assert.equal(root.children[0], card);
  runFrame(0);
  runFrame(400);
  assert.equal(steelhead.textContent, '4,828');
  assert.equal(steelhead.classList.contains('is-number-changed'), true);
  runFrame(800);
  assert.equal(steelhead.textContent, '4,830');

  cards.updateDefeatCounts(root, record(990));
  runFrame(1000);
  cards.updateDefeatCounts(root, record(1010));
  runFrame(2000);
  runFrame(2400);
  assert.equal(steelhead.textContent, '1,008');
  runFrame(2800);
  assert.equal(steelhead.textContent, '1,010');

  // 鲑鱼种类变了（游戏更新加了新鲑鱼）才重建。
  const next = record(1010);
  next.enemies.push({ id: 20, name: 'Mudmouth', image: '/images/coop-enemy/20', count: 412 });
  cards.updateDefeatCounts(root, next);
  assert.notEqual(root.children[0], card);
});

test('the defeat count page polls its own data and the app previews the dashboard record', () => {
  assert.match(pageHtml, /fetch\('\/api\/defeat-counts'/);
  // 显示哪些鲑鱼跟着数据一起轮询，不从地址里读。
  assert.match(pageHtml, /Splat3OverlayCard\.updateDefeatCounts\(root, record, undefined, record\?\.show\)/);
  assert.doesNotMatch(pageHtml, /location\.search/);
  assert.ok(pageHtml.indexOf('/game-names.js') < pageHtml.indexOf('/overlay-card-renderer.js'));
  assert.match(appJs, /cards\?\.updateDefeatCounts\(scale, dashboard\.coopRecord\(\), records\.imageSrc\)/);
  // 应用预览保留逻辑尺寸，OBS 使用高分辨率。
  const { width, height } = catalog.find((card) => card.id === 'defeat-counts');
  assert.equal(width, 3072);
  assert.equal(height, 256);
  assert.ok(previewCss.includes(`.overlay-preview__scale > .defeat-count-card { --obs-card-width: 768px; --obs-card-height: 64px; }`));
});


test('defeat selections keep the SplatNet order and unknown selections retain all entries', () => {
  const { cards } = setup();
  const all = cards.defeatCountItems(record());
  const selected = cards.selectedDefeatItems(all, ['bosses:23', 'enemies:4']);
  assert.deepEqual(Array.from(selected.enemies, (item) => item.key), ['enemies:4']);
  assert.deepEqual(Array.from(selected.bosses, (item) => item.key), ['bosses:23']);
  assert.equal(cards.defeatCountWidth(selected), 121);
  assert.equal(cards.defeatCountWidth(cards.selectedDefeatItems(all, ['enemies:4'])), 58);
  // 没选过或一个都对不上（比如记录里没有这些鲑鱼）时全部显示。
  assert.equal(cards.selectedDefeatItems(all, null), all);
  assert.equal(cards.selectedDefeatItems(all, undefined), all);
  assert.equal(cards.selectedDefeatItems(all, ['unknown']), all);
});

test('picking Salmonids in the preview saves the choice instead of changing the OBS address', () => {
  // 地址固定是目录里的 /defeat-counts，选择存进应用设置，OBS 页面轮询时一起取。
  assert.equal(catalog.find((card) => card.id === 'defeat-counts').route, '/defeat-counts');
  assert.doesNotMatch(appJs, /defeat-counts\?|URLSearchParams\(\{ show/);
  assert.doesNotMatch(appJs, /splat3-obs-defeat-selection/);
  assert.match(appJs, /invoke\('get_obs_defeat_selection'\)/);
  assert.match(appJs, /invoke\('save_obs_defeat_selection', \{ selection: keys \}\)/);
  // 切换鲑鱼不会让已经复制的地址失效。
  const setter = appJs.slice(appJs.indexOf('function setObsDefeatSelection'), appJs.indexOf('function toggleObsDefeat'));
  assert.doesNotMatch(setter, /copiedUrl/);
});

test('filtered OBS cards compact their width and omit the divider for a single group', () => {
  const { cards, root } = setup();
  cards.updateDefeatCounts(root, record(), undefined, ['enemies:4']);
  assert.equal(root.children[0].children.length, 1);
  assert.equal(root.children[0].children[0].title, '炸弹鱼');
  assert.equal(root.children[0].style.width, '58px');
  cards.updateDefeatCounts(root, record(), undefined, ['enemies:4', 'bosses:23']);
  assert.equal(root.children[0].children.length, 3);
  assert.equal(root.children[0].style.width, '121px');
  // 不管显示哪些，都记下全部显示时的宽度（两种巨大鲑鱼加三种头目鲑鱼）。
  assert.equal(root.children[0].dataset.fullWidth, '259');
});

test('the OBS page keeps the same height while Salmonids are hidden or shown in the app', async () => {
  // 在普通浏览器窗口里打开：卡片受宽度限制，以前隐藏鲑鱼后会被放大、变高。
  for (const [width, height] of [[1024, 768], [2876, 256], [1280, 720]]) {
    let show = null;
    const page = openPage({ width, height, data: () => ({ ...record(), show }) });
    await page.update();
    const full = { scale: page.root.children[0].style.transform, height: page.root.style.height };
    for (const keys of [['enemies:4'], ['bosses:23', 'enemies:4'], ['enemies:17', 'bosses:30', 'bosses:99'], null]) {
      show = keys;
      await page.update();
      const card = page.root.children[0];
      assert.deepEqual({ scale: card.style.transform, height: page.root.style.height }, full, `${width}×${height} ${keys}`);
      // 只有宽度跟着显示的鲑鱼变：卡片靠左，右边空出来。
      const scale = Number(/scale\(([\d.]+)\)/.exec(card.style.transform)[1]);
      assert.equal(page.root.style.width, `${parseFloat(card.style.width) * scale}px`);
      assert.ok(parseFloat(card.style.width) * scale <= width + 1e-6);
    }
  }
});

test('the app recommends one OBS source size for every Salmonid selection', () => {
  // 按全部鲑鱼算建议宽度，在预览里隐藏或显示鲑鱼时不用改 OBS 来源的尺寸。
  const sizeFn = appJs.slice(appJs.indexOf('function obsSourceSize'), appJs.indexOf('function renderObsUrl'));
  assert.match(sizeFn, /obsDefeatItems\(\)/);
  assert.doesNotMatch(sizeFn, /obsSelectedDefeats/);
  assert.match(pageHtml, /card\.dataset\.fullWidth/);
});
