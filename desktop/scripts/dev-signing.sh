#!/bin/sh
# 本机开发签名：开发版和本机安装的应用都用同一张固定的自签名证书签名。
# 不签名时，每次重新编译的程序在钥匙串看来都是新程序，会再要一次登录密码；
# 用固定证书签名后，“始终允许”在重新编译、重新安装后依然有效。
#
#   dev-signing.sh setup              创建证书（只需一次，不需要输入密码）
#   dev-signing.sh run <程序> [参数]   Cargo 的 runner：给应用签名后启动；没有证书或签名失败时直接启动
#   dev-signing.sh sign-app <.app>    给打包好的应用签名；没有证书时返回 1
#
# 证书放在单独的钥匙串里，不加入系统钥匙串的搜索列表；钥匙串密码随机生成，
# 只保存在同一目录（仅本人可读）。这张证书只在本机使用。
set -eu

IDENTIFIER=ink.space4.Splat3Insight
NAME="Splat3 Insight Dev"
DIR="${SPLAT3_DEV_SIGNING_DIR:-$HOME/Library/Application Support/Splat3Insight/dev-signing}"
KEYCHAIN="$DIR/signing.keychain-db"
PASSWORD_FILE="$DIR/keychain-password"

# 签名证书的 SHA-1；没有配置时输出为空。
identity() {
  [ -f "$KEYCHAIN" ] && [ -f "$PASSWORD_FILE" ] || return 0
  security unlock-keychain -p "$(cat "$PASSWORD_FILE")" "$KEYCHAIN" >/dev/null 2>&1 || return 0
  security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -F "\"$NAME\"" | awk '{ print $2; exit }'
}

# codesign 最多等 60 秒，万一系统弹窗也不会卡住编译。
codesign_with_identity() {
  hash=$(identity)
  [ -n "$hash" ] || return 1
  perl -e 'alarm shift; exec @ARGV' 60 codesign --force --sign "$hash" --keychain "$KEYCHAIN" "$@"
}

restore_search_list() {
  old_ifs=$IFS
  IFS='
'
  # shellcheck disable=SC2086
  set -- $1
  IFS=$old_ifs
  security list-keychains -d user -s "$@"
}

setup() {
  if [ "$(uname -s)" != Darwin ]; then
    echo "只有 macOS 需要开发签名。" >&2
    exit 1
  fi
  if [ -n "$(identity)" ]; then
    echo "开发签名证书已经配置好了。"
    exit 0
  fi
  mkdir -p "$DIR"
  chmod 700 "$DIR"
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  password=$(openssl rand -hex 24)
  bundle_password=$(openssl rand -hex 16)

  # create-keychain 会把新钥匙串加进搜索列表；建好后恢复原来的列表。
  search_list=$(security list-keychains -d user | sed -e 's/^ *"//' -e 's/" *$//')
  rm -f "$KEYCHAIN"
  security create-keychain -p "$password" "$KEYCHAIN" >/dev/null
  restore_search_list "$search_list"
  security set-keychain-settings "$KEYCHAIN"
  security unlock-keychain -p "$password" "$KEYCHAIN"

  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=$NAME" \
    -keyout "$work/key.pem" -out "$work/cert.pem" \
    -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=critical,codeSigning" \
    -addext "basicConstraints=critical,CA:false" 2>/dev/null
  # macOS 只能导入旧格式的 PKCS#12；OpenSSL 3 用 -legacy，LibreSSL 直接指定算法。
  openssl pkcs12 -export -legacy -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/identity.p12" -passout "pass:$bundle_password" 2>/dev/null \
    || openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/identity.p12" -passout "pass:$bundle_password" \
      -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1
  security import "$work/identity.p12" -k "$KEYCHAIN" -P "$bundle_password" -T /usr/bin/codesign >/dev/null
  # 允许 codesign 直接使用这把私钥，签名时不弹窗。
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$KEYCHAIN" >/dev/null

  umask 077
  printf '%s' "$password" > "$PASSWORD_FILE"
  if [ -z "$(identity)" ]; then
    echo "开发签名证书创建失败。" >&2
    exit 1
  fi
  echo "已创建开发签名证书。之后第一次启动应用时，钥匙串会再问一次：输入登录密码并选择“始终允许”，以后重新编译就不会再问。"
}

case "${1:-}" in
  setup)
    setup
    ;;
  run)
    shift
    # 只给应用本身签名；测试程序等其他可执行文件直接运行。
    if [ "$(uname -s)" = Darwin ] && [ "$(basename "$1")" = Splat3Insight ] && [ "${SPLAT3_DEV_SIGNING:-1}" != 0 ]; then
      if [ -f "$KEYCHAIN" ] && ! codesign_with_identity --identifier "$IDENTIFIER" "$1" >/dev/null 2>&1; then
        echo "开发签名失败，按原样启动。" >&2
      fi
    fi
    # 只签名不启动，用来检查配置。
    if [ "${SPLAT3_DEV_SIGN_ONLY:-0}" = 1 ]; then
      exit 0
    fi
    exec "$@"
    ;;
  sign-app)
    codesign_with_identity --deep "$2"
    ;;
  *)
    echo "用法：dev-signing.sh setup | run <程序> [参数] | sign-app <.app>" >&2
    exit 2
    ;;
esac
