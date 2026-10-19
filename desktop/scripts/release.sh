#!/bin/sh
# 准备发布：检查更新内容、改版本号、提交并打标签。推送标签后 GitHub Actions 打包并生成
# Release 草稿，说明取自 CHANGELOG.md 里这个版本的一节。
#
#   release.sh 0.3.0
#
# 只在 main 分支执行。先在 CHANGELOG.md 里写好这次的更新内容，可以先不提交，会和版本号
# 一起提交；其他改动要先提交。不会自动推送，最后会打印推送命令。
set -eu

version="${1:-}"
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "用法：$0 <版本号，例如 0.3.0>" >&2
  exit 2
fi
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

if [ "$(git rev-parse --abbrev-ref HEAD)" != main ]; then
  echo "请在 main 分支上发布。" >&2
  exit 1
fi
if [ -n "$(git status --porcelain -- . ':(exclude)CHANGELOG.md')" ]; then
  echo "工作区还有 CHANGELOG.md 以外的未提交改动，请先提交或撤销。" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  echo "标签 v$version 已经存在。" >&2
  exit 1
fi
# 发布流程会把 CHANGELOG.md 里这个版本的一节放进 Release 说明，没写就不发布。
notes=$(node desktop/scripts/changelog.mjs notes "$version")
[ -n "$notes" ] || { echo "CHANGELOG.md 里还没有 $version 的更新内容。" >&2; exit 1; }

# 三处版本号保持一致：应用配置、桌面端和共享核心的 Cargo.toml。
set_package_version() {
  sed -i.bak -e "1,/^version = \".*\"$/s/^version = \".*\"$/version = \"$version\"/" "$1"
  rm -f "$1.bak"
}
sed -i.bak -e "s/^  \"version\": \".*\",$/  \"version\": \"$version\",/" desktop/src-tauri/tauri.conf.json
rm -f desktop/src-tauri/tauri.conf.json.bak
set_package_version desktop/src-tauri/Cargo.toml
set_package_version desktop-core/Cargo.toml
# 更新两个 Cargo.lock 里本项目自己的版本，不改其他依赖。cargo 要读到所有平台的依赖信息，
# 只在 macOS 上编译过时，Windows 等平台的包可能还没下载，所以要联网。
cargo metadata --format-version 1 --manifest-path desktop-core/Cargo.toml >/dev/null
cargo metadata --format-version 1 --manifest-path desktop/src-tauri/Cargo.toml >/dev/null

git add CHANGELOG.md desktop/src-tauri/tauri.conf.json desktop/src-tauri/Cargo.toml desktop-core/Cargo.toml \
  desktop/src-tauri/Cargo.lock desktop-core/Cargo.lock
git commit -q -m "Release $version"
git tag -a "v$version" -m "Splat3 Insight $version"
echo "已提交并打上标签 v${version}。确认无误后推送："
echo "  git push origin main v$version"
