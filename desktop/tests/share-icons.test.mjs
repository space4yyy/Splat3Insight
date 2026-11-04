import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../ui/share-image.js', import.meta.url), 'utf8');
const context = vm.createContext({
  el: (tag, props, children) => ({ tag, ...props, children }),
  card: (title, color, body) => body,
  fixed1: String, grouped: String,
});
vm.runInContext(`${source.slice(source.indexOf('  function contribution('), source.indexOf('  function buildShare('))}\nthis.render = contribution;`, context);

test('share rescue icons select light variants for both species and retain dark originals', () => {
  for (const [species, asset] of [['OCTOLING', 'octo'], ['INKLING', 'squid'], [null, 'squid']]) {
    for (const look of ['light', 'dark']) {
      const body = context.render({}, species, look);
      const suffix = look === 'light' ? '-light' : '';
      assert.equal(body.children[2].children[0].src, `images/helpen-${asset}${suffix}.svg`);
      assert.equal(body.children[3].children[0].src, `images/helped-${asset}${suffix}.svg`);
      assert.equal(body.children[0].children[0].src, 'images/gold-ikura.svg');
    }
  }
});

test('light rescue assets change only white fills and preserve geometry and other colors', async () => {
  for (const kind of ['helpen', 'helped']) {
    for (const species of ['squid', 'octo']) {
      const original = await readFile(new URL(`../ui/images/${kind}-${species}.svg`, import.meta.url), 'utf8');
      const light = await readFile(new URL(`../ui/images/${kind}-${species}-light.svg`, import.meta.url), 'utf8');
      assert.notEqual(light, original);
      assert.equal(light, original.replace(/fill="(?:white|#fff|#ffffff)"/gi, 'fill="#44424f"'));
    }
  }
});
