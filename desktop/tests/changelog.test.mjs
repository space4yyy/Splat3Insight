import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { releaseNotes } from '../scripts/changelog.mjs';

const changelog = `# 更新日志

<!-- 写法说明 -->

## 0.3.30（2026-12-01）

- 不该被 0.3.3 取到。

## v0.3.3（2026-10-19）

### 新增

- 时钟卡片。

## 0.3.2

## 0.3.1
\r
- 修复 Windows 版。\r
`;

test('release notes are the lines between a version heading and the next one', () => {
  assert.equal(releaseNotes(changelog, '0.3.3'), '### 新增\n\n- 时钟卡片。');
  assert.equal(releaseNotes(changelog, 'v0.3.3'), '### 新增\n\n- 时钟卡片。');
  assert.equal(releaseNotes(changelog, '0.3.30'), '- 不该被 0.3.3 取到。');
  assert.equal(releaseNotes(changelog, '0.3.1'), '- 修复 Windows 版。');
});

test('a missing or empty version has no release notes', () => {
  assert.equal(releaseNotes(changelog, '0.3.2'), null);
  assert.equal(releaseNotes(changelog, '0.4.0'), null);
  assert.equal(releaseNotes(changelog, '0.3'), null);
});

test('the current app version has release notes in CHANGELOG.md', async () => {
  const [config, notes] = await Promise.all([
    readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
    readFile(new URL('../../CHANGELOG.md', import.meta.url), 'utf8'),
  ]);
  const { version } = JSON.parse(config);
  assert.ok(releaseNotes(notes, version), `CHANGELOG.md 里没有 ${version}`);
});
