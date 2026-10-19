// 从 CHANGELOG.md 取出某个版本的更新内容，发布时放进 GitHub Release 的说明：
//   node scripts/changelog.mjs notes [版本号] [--github-output]
// 版本号可以带 v，默认取 GITHUB_REF_NAME（推送的标签名）。带 --github-output 时写进
// GitHub Actions 的步骤输出 notes，否则直接打印。没有这个版本的更新内容时退出码为 1。
import { randomUUID } from 'node:crypto';
import { realpathSync } from 'node:fs';
import { appendFile, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const HEADING = /^##\s+v?(\d+\.\d+\.\d+)(?=$|[\s（(])/;

// 「## 版本号」下面到下一个「## 」标题之前的内容；没有这一节或这一节是空的时返回 null。
export function releaseNotes(changelog, version) {
  const wanted = version.replace(/^v/, '');
  const lines = changelog.split(/\r?\n/);
  const start = lines.findIndex((line) => line.match(HEADING)?.[1] === wanted);
  if (start < 0) return null;
  const next = lines.findIndex((line, index) => index > start && line.startsWith('## '));
  const notes = lines.slice(start + 1, next < 0 ? lines.length : next).join('\n').trim();
  return notes || null;
}

async function main([command, ...args]) {
  const version = args.find((arg) => !arg.startsWith('--')) ?? process.env.GITHUB_REF_NAME;
  if (command !== 'notes' || !version) {
    console.error('用法：node scripts/changelog.mjs notes [版本号] [--github-output]');
    return 2;
  }
  const changelog = await readFile(new URL('../../CHANGELOG.md', import.meta.url), 'utf8');
  const notes = releaseNotes(changelog, version);
  if (!notes) {
    console.error(`CHANGELOG.md 里还没有 ${version.replace(/^v/, '')} 的更新内容。`);
    return 1;
  }
  if (args.includes('--github-output')) {
    const delimiter = `NOTES_${randomUUID()}`;
    await appendFile(process.env.GITHUB_OUTPUT, `notes<<${delimiter}\n${notes}\n${delimiter}\n`);
  } else {
    console.log(notes);
  }
  return 0;
}

// 测试会导入 releaseNotes，只有直接运行这个文件时才执行命令。
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = await main(process.argv.slice(2));
}
