import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../ui/analysis.js', import.meta.url), 'utf8');
function setup() {
  const node = (tag, props = {}, children = []) => ({
    tag, ...props, children, listeners: {}, scrollTop: 0, clientHeight: 320, scrollHeight: 960,
    addEventListener(type, listener) { this.listeners[type] = listener; },
    replaceChildren(...children) { this.children = children; },
  });
  const box = node('div');
  const pager = node('div');
  const context = vm.createContext({
    $: (id) => id === 'rotation-table' ? box : pager,
    el: node, analysis: { rotationGroups: (groups) => groups },
    document: { createTextNode: (text) => text }, COOP_RULE_LABEL: {},
    fmtDateTime: String, pct: String, fixed: String,
    gradeText: (name, point) => `${name} ${point}`,
    noShifts: () => { box.rows = []; },
    renderTable: (_, columns, rows) => { box.columns = columns; box.rows = rows; },
  });
  vm.runInContext(`${source.slice(source.indexOf('  function suppliedCell('), source.indexOf('  function renderSalmon('))}\nthis.render = renderRotations;`, context);
  const wheel = (timeStamp, deltaY, extra = {}) => {
    let prevented = false;
    box.listeners.wheel({ timeStamp, deltaY, deltaX: 0, deltaMode: 0, preventDefault: () => { prevented = true; }, ...extra });
    // Match browser scrollTop clamping.
    box.scrollTop = Math.max(0, Math.min(box.scrollTop, box.scrollHeight - box.clientHeight));
    return prevented;
  };
  return { box, pager, render: context.render, wheel };
}
const groups = (n) => Array.from({ length: n }, (_, i) => ({
  start: 1000 - i, end: 1001 - i, rule: 'REGULAR', stage: `场地 ${i}`, ongoing: i === 0,
  supplied: [], n: 1, cleared: 1, scales: {}, avgGoldenEggs: 25, grade: { name: '传说', point: 200 },
}));

test('rotations page through all groups, reset scroll and reset when the source changes', () => {
  const { box, pager, render } = setup();
  render(groups(45));
  assert.equal(box.rows.length, 20);
  assert.equal(pager.children[0].text, '共 45 期 · 第 1–20 期');
  assert.equal(pager.children[1].children[0].disabled, true);
  box.scrollTop = 100;
  pager.children[1].children[2].listeners.click();
  assert.equal(box.scrollTop, 0);
  assert.equal(box.rows[0][1].children[0], '场地 20');
  pager.children[1].children[2].listeners.click();
  assert.equal(box.rows.length, 5);
  assert.equal(pager.children[1].children[2].disabled, true);
  render(groups(2));
  assert.equal(box.rows.length, 2);
  assert.equal(pager.children[1].children[1].text, '1 / 1');
  render([]);
  assert.equal(pager.children.length, 0);
});

test('grade and egg columns swap and ongoing status stays alongside the stage', () => {
  const { box, render } = setup();
  render(groups(1));
  assert.equal(box.columns[5].label, '评级');
  assert.equal(box.rows[0][5], '传说 200');
  assert.equal(box.columns[8].label, '场均金鲑鱼卵');
  assert.equal(box.rows[0][8], '25');
  assert.equal(box.rows[0][1].children[0], '场地 0');
  assert.equal(box.rows[0][1].children[1]['aria-label'], '进行中');
});

test('boundaries stop inertia and release the next gesture after a 50ms gap', () => {
  const { box, render, wheel } = setup();
  render(groups(20));
  box.scrollTop = 620;
  assert.equal(wheel(100, 40), true);
  assert.equal(box.scrollTop, 640);
  assert.equal(wheel(149, 30), true);
  assert.equal(wheel(199, 10), false);
  assert.equal(wheel(200, 30), false);
  assert.equal(wheel(210, 30), false);
  assert.equal(wheel(560, -50), true);
  box.scrollTop = 10;
  assert.equal(wheel(600, -20), true);
  assert.equal(wheel(649, -10), true);
  assert.equal(wheel(699, -30), false);
  assert.equal(wheel(700, -30), false);
  assert.equal(wheel(1060, 20), true);
});

test('horizontal scrolling, zoom, short tables and wheel units retain expected behavior', () => {
  const { box, render, wheel } = setup();
  render(groups(20));
  assert.equal(wheel(100, 3, { deltaX: 10 }), false);
  assert.equal(wheel(200, 10, { ctrlKey: true }), false);
  assert.equal(wheel(300, 2, { deltaMode: 1 }), true);
  assert.equal(box.scrollTop, 64);
  assert.equal(wheel(400, 1, { deltaMode: 2 }), true);
  assert.equal(box.scrollTop, 384);
  box.scrollHeight = box.clientHeight;
  assert.equal(wheel(500, 20), false);
});
