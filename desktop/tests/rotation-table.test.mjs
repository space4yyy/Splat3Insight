import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../ui/analysis.js', import.meta.url), 'utf8');
function setup(savedSize = null) {
  const node = (tag, props = {}, children = []) => ({
    tag, ...props, children, listeners: {}, scrollTop: 0, clientHeight: 320, scrollHeight: 960,
    addEventListener(type, listener) { this.listeners[type] = listener; },
    replaceChildren(...children) { this.children = children; },
  });
  const box = node('div');
  const pager = node('div');
  const stored = new Map(savedSize == null ? [] : [['splat3-rotation-page-size', savedSize]]);
  const context = vm.createContext({
    localStorage: { getItem: (key) => stored.get(key) ?? null, setItem: (key, value) => stored.set(key, value) },
    $: (id) => id === 'rotation-table' ? box : pager,
    el: node, analysis: { rotationGroups: (groups) => groups },
    document: { createTextNode: (text) => text }, COOP_RULE_LABEL: {},
    fmtDateTime: String, pct: String, fixed: String,
    gradeText: (name, point) => `${name} ${point}`,
    noShifts: () => { box.rows = []; },
    renderTable: (_, columns, rows) => { box.columns = columns; box.rows = rows; },
  });
  vm.runInContext(`${source.slice(source.indexOf('  function suppliedCell('), source.indexOf('  function renderSalmon('))}\nthis.render = renderRotations;`, context);
  return { box, pager, stored, render: context.render };
}
const groups = (n) => Array.from({ length: n }, (_, i) => ({
  start: 1000 - i, end: 1001 - i, rule: 'REGULAR', stage: `场地 ${i}`, ongoing: i === 0,
  supplied: [], n: 1, clear: 1, scales: {}, avgGoldenEggs: 25, grade: { name: '传说', point: 200 },
}));

test('rotations page through all groups, reset scroll and reset when the source changes', () => {
  const { box, pager, render } = setup();
  render(groups(13));
  assert.equal(box.rows.length, 5);
  assert.equal(pager.children[0].children[0].text, '共 13 期 · 每页');
  assert.equal(pager.children[1].children[0].disabled, true);
  box.scrollTop = 100;
  pager.children[1].children[2].listeners.click();
  assert.equal(box.scrollTop, 0);
  assert.equal(box.rows[0][1].children[0], '场地 5');
  pager.children[1].children[2].listeners.click();
  assert.equal(box.rows.length, 3);
  assert.equal(pager.children[1].children[2].disabled, true);
  render(groups(2));
  assert.equal(box.rows.length, 2);
  assert.equal(pager.children[1].children[1].text, '1 / 1');
  render([]);
  assert.equal(pager.children.length, 0);
});

test('rotations retain grade and ongoing status without the average golden egg column', () => {
  const { box, render } = setup();
  render(groups(1));
  assert.equal(box.columns[5].label, '评级');
  assert.equal(box.rows[0][5], '传说 200');
  assert.equal(box.columns.length, 8);
  assert.equal(box.rows[0].length, 8);
  assert.equal(box.columns.some((column) => column.label === '场均金鲑鱼卵'), false);
  assert.equal(box.rows[0][1].children[0], '场地 0');
  assert.equal(box.rows[0][1].children[1]['aria-label'], '进行中');
});

test('rotation kings show an image with counts, falling back to text for unknown images', () => {
  const { box, render } = setup();
  const list = groups(3);
  list[0].king = { name: '横纲', image: 'images/boss1_color.png', defeated: 2, appeared: 3 };
  list[1].king = { name: '未知头目', image: null, defeated: 0, appeared: 1 };
  render(list);
  const cell = box.rows[0][6];
  assert.equal(cell.children[0].tag, 'img');
  assert.equal(cell.children[0].src, 'images/boss1_color.png');
  assert.equal(cell.children[0].alt, '横纲');
  assert.equal(cell.children[1].text, '2 / 3');
  assert.equal(cell.title, '横纲 · 击倒 2 / 出现 3');
  assert.equal(box.rows[1][6].children[0], '未知头目');
  assert.equal(box.rows[2][6], '—');
});


test('page size dropdown offers 5/10/15, persists choices and resets to the first page', () => {
  const { box, pager, stored, render } = setup();
  render(groups(21));
  pager.children[1].children[2].listeners.click();
  let select = pager.children[0].children[1].children[0];
  assert.equal(select.tag, 'select');
  assert.deepEqual(Array.from(select.children, (option) => option.value), ['5', '10', '15']);
  select.value = '10';
  select.listeners.change();
  assert.equal(box.rows.length, 10);
  assert.equal(box.rows[0][1].children[0], '场地 0');
  assert.equal(pager.children[1].children[1].text, '1 / 3');
  assert.equal(stored.get('splat3-rotation-page-size'), '10');
  for (const value of ['', '0', '7', '20']) {
    select = pager.children[0].children[1].children[0];
    select.value = value;
    select.listeners.change();
    assert.equal(select.value, '10');
    assert.equal(box.rows.length, 10);
  }
  const restored = setup('15');
  restored.render(groups(21));
  assert.equal(restored.box.rows.length, 15);
  const invalid = setup('7');
  invalid.render(groups(21));
  assert.equal(invalid.box.rows.length, 5);
});
