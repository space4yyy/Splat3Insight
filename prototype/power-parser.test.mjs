import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolvePower } from './normalizer.mjs';

const openRaw = JSON.parse(await readFile(
  new URL('../data/s3s-exports/results/20260915T143129Z.json', import.meta.url),
));
const openPower = resolvePower(openRaw.data.vsHistoryDetail);

assert.equal(openPower.kind, 'team');
assert.equal(openPower.value, 1878.2832444727444);
assert.equal(openPower.path, 'bankaraMatch.bankaraPower.power');

const challengePower = resolvePower({
  vsMode: { mode: 'BANKARA' },
  bankaraMatch: {
    mode: 'CHALLENGE',
    bankaraPower: { power: 1999.9 },
    weaponPower: 1888.8,
  },
});

assert.equal(challengePower.kind, 'weapon');
assert.equal(challengePower.value, 1888.8);
assert.equal(challengePower.path, 'bankaraMatch.weaponPower');

const challengeObjectPower = resolvePower({
  vsMode: { mode: 'BANKARA' },
  bankaraMatch: { mode: 'CHALLENGE', weaponPower: { power: 1777.7 } },
});

assert.equal(challengeObjectPower.value, 1777.7);

console.log('power parser regression checks passed');
