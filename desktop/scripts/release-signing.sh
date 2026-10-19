#!/bin/sh
# 发布签名：生成一次，之后每次发布都用同一套密钥和证书。
#
#   release-signing.sh setup   生成自动更新签名密钥和 macOS 发布证书（已有的不会覆盖）
#
# 文件放在 ~/Library/Application Support/Splat3Insight/release-signing（仅本人可读），
# 内容要填进 GitHub 仓库的 Actions secrets（对应关系见 setup 的输出）：
#   updater.key、updater-password            自动更新签名私钥和它的密码
#   updater.key.pub                          公钥，已写进 tauri.conf.json
#   release.p12.base64、release-p12-password  macOS 发布证书（自签名）
#   release-identity                         证书的 SHA-1，codesign 用它找证书
#
# 请另外备份这个目录。丢了更新私钥，已安装的版本就收不到后续更新；
# 换了发布证书，macOS 用户更新后要重新允许一次钥匙串访问。
set -eu

NAME="Splat3 Insight"
DIR="${SPLAT3_RELEASE_SIGNING_DIR:-$HOME/Library/Application Support/Splat3Insight/release-signing}"
ROOT=$(cd "$(dirname "$0")/../.." && pwd)

setup_updater_key() {
  if [ -f "$DIR/updater.key" ] && [ -f "$DIR/updater.key.pub" ] && [ -f "$DIR/updater-password" ]; then
    return 0
  fi
  password=$(openssl rand -hex 24)
  printf '%s' "$password" > "$DIR/updater-password"
  chmod 600 "$DIR/updater-password"
  # 输出里有密钥信息，不显示；公钥写在 updater.key.pub。
  "$ROOT/desktop/node_modules/.bin/tauri" signer generate --ci --force \
    -w "$DIR/updater.key" -p "$password" >/dev/null 2>&1
  chmod 600 "$DIR/updater.key"
}

setup_certificate() {
  if [ -f "$DIR/release.p12.base64" ] && [ -f "$DIR/release-p12-password" ] && [ -f "$DIR/release-identity" ]; then
    return 0
  fi
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  password=$(openssl rand -hex 16)
  openssl req -x509 -newkey rsa:2048 -nodes -days 7300 -subj "/CN=$NAME" \
    -keyout "$work/key.pem" -out "$work/cert.pem" \
    -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=critical,codeSigning" \
    -addext "basicConstraints=critical,CA:false" 2>/dev/null
  # macOS 只能导入旧格式的 PKCS#12；OpenSSL 3 用 -legacy，LibreSSL 直接指定算法。
  openssl pkcs12 -export -legacy -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/release.p12" -passout "pass:$password" 2>/dev/null \
    || openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/release.p12" -passout "pass:$password" \
      -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1
  openssl base64 -A -in "$work/release.p12" > "$DIR/release.p12.base64"
  printf '%s' "$password" > "$DIR/release-p12-password"
  openssl x509 -in "$work/cert.pem" -noout -fingerprint -sha1 | sed -e 's/^.*=//' -e 's/://g' > "$DIR/release-identity"
  chmod 600 "$DIR/release.p12.base64" "$DIR/release-p12-password" "$DIR/release-identity"
}

setup() {
  mkdir -p "$DIR"
  chmod 700 "$DIR"
  setup_updater_key
  setup_certificate
  echo "发布签名文件在：$DIR"
  echo "在 GitHub 仓库 Settings → Secrets and variables → Actions 里新建这些 secrets："
  echo "  TAURI_SIGNING_PRIVATE_KEY           ← updater.key 的内容"
  echo "  TAURI_SIGNING_PRIVATE_KEY_PASSWORD  ← updater-password 的内容"
  echo "  APPLE_CERTIFICATE                   ← release.p12.base64 的内容"
  echo "  APPLE_CERTIFICATE_PASSWORD          ← release-p12-password 的内容"
  echo "  APPLE_SIGNING_IDENTITY              ← release-identity 的内容"
  echo "请把这个目录另外备份一份。"
}

case "${1:-}" in
  setup) setup ;;
  *)
    echo "用法：$0 setup" >&2
    exit 2
    ;;
esac
