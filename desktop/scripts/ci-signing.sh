#!/bin/bash
# 将发布证书导入构建机的临时钥匙串。Tauri 仅通过 SHA-1/名称查找，
# 不走只识别 Apple 证书名称的 APPLE_CERTIFICATE 自动导入流程。
set +x
set -euo pipefail

restore_keychains() {
  local directory=$1
  local keychain
  local keychains=()
  if [[ -f "$directory/search-list" ]]; then
    while IFS= read -r keychain; do
      [[ -z "$keychain" ]] || keychains+=("$keychain")
    done < "$directory/search-list"
    security list-keychains -d user -s "${keychains[@]}"
  fi
}

cleanup() {
  local directory=${SPLAT3_CI_SIGNING_DIR:-}
  [[ -n "$directory" && -d "$directory" ]] || return 0
  if [[ "${directory%/*}" != "${RUNNER_TEMP:-}" || "${directory##*/}" != splat3-signing.* ]]; then
    echo "::error::临时签名目录不在 RUNNER_TEMP 中，无法清理。" >&2
    return 1
  fi
  local status=0
  security delete-keychain "$directory/signing.keychain-db" >/dev/null 2>&1 || status=$?
  restore_keychains "$directory" || status=$?
  rm -rf "$directory"
  return "$status"
}

prepare() {
  local variable
  for variable in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY GITHUB_ENV RUNNER_TEMP; do
    if [[ -z "${!variable:-}" ]]; then
      echo "::error::缺少发布签名配置：$variable" >&2
      return 1
    fi
  done

  umask 077
  SPLAT3_CI_SIGNING_DIR=$(mktemp -d "$RUNNER_TEMP/splat3-signing.XXXXXX")
  trap 'cleanup' EXIT
  local directory=$SPLAT3_CI_SIGNING_DIR
  local keychain="$directory/signing.keychain-db"
  local password
  password=$(openssl rand -hex 24)
  local identity
  identity=$(printf '%s' "$APPLE_SIGNING_IDENTITY" | tr -d '\r\n' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
  if [[ -z "$identity" ]]; then
    echo "::error::APPLE_SIGNING_IDENTITY 不能为空白。" >&2
    return 1
  fi

  # 密码不写入 GITHUB_ENV，防止后续步骤的环境摘要输出临时密码。
  echo "::add-mask::$password"
  echo "::add-mask::$identity"
  printf 'SPLAT3_CI_SIGNING_DIR=%s\n' "$directory" >> "$GITHUB_ENV"
  security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//' > "$directory/search-list"

  printf '%s' "$APPLE_CERTIFICATE" | tr -d '[:space:]' | base64 --decode > "$directory/certificate.p12"
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$directory/certificate.p12" -k "$keychain" \
    -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
    -k "$password" "$keychain" >/dev/null
  rm "$directory/certificate.p12"

  # Tauri 使用默认搜索列表查找证书；保留构建机原有的钥匙串。
  local existing
  local keychains=()
  while IFS= read -r existing; do
    [[ -z "$existing" ]] || keychains+=("$existing")
  done < "$directory/search-list"
  security list-keychains -d user -s "${keychains[@]}" "$keychain"

  # 提前验证证书与私钥可用，避免完整编译后才发现签名失败。
  cp /usr/bin/true "$directory/signing-check"
  codesign --force --sign "$identity" --keychain "$keychain" \
    "$directory/signing-check"
  codesign --verify --strict "$directory/signing-check"
  rm "$directory/signing-check"
  printf 'APPLE_SIGNING_IDENTITY=%s\n' "$identity" >> "$GITHUB_ENV"
  trap - EXIT
  echo "发布证书已导入，签名验证通过。"
}

case "${1:-}" in
  prepare) prepare ;;
  cleanup) cleanup ;;
  *) echo "用法：$0 prepare | cleanup" >&2; exit 2 ;;
esac
