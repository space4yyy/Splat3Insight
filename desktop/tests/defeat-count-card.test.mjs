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
  return { cards: window.Splat3OverlayCard, root: node('main'), runFrame };
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
  assert.match(pageHtml, /Splat3OverlayCard\.updateDefeatCounts\(root, record\)/);
  assert.ok(pageHtml.indexOf('/game-names.js') < pageHtml.indexOf('/overlay-card-renderer.js'));
  assert.match(appJs, /cards\?\.updateDefeatCounts\(scale, dashboard\.coopRecord\(\), records\.imageSrc\)/);
  // 应用里的预览按目录里的尺寸排版。
  const { width, height } = catalog.find((card) => card.id === 'defeat-counts');
  assert.ok(previewCss.includes(`.overlay-preview__scale > .defeat-count-card { --obs-card-width: ${width}px; --obs-card-height: ${height}px; }`));
});
