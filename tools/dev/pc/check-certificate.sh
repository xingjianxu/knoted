#!/usr/bin/env bash
# 证书启动前校验：文件可读、已生效、未过期、hostname 匹配。
# 只在失败时输出原因（不打印私钥或证书正文），成功静默并返回 0。
#
# 用法：check-certificate.sh [fullchain.pem] [key.pem] [hostname]
#   默认取测试实例的 /var/lib/acme/wlab.132140.xyz 与 wlab.132140.xyz。
set -euo pipefail

cert="${1:-/var/lib/acme/wlab.132140.xyz/fullchain.pem}"
key="${2:-/var/lib/acme/wlab.132140.xyz/key.pem}"
name="${3:-wlab.132140.xyz}"

fail() {
  printf 'knoted-check-certificate: %s\n' "$1" >&2
  exit 1
}

command -v openssl >/dev/null || fail "缺少 openssl（由 /etc/nixos 提供：pkgs.openssl），无法校验证书"

test -r "$cert" || fail "证书不可读：$cert"
test -r "$key" || fail "私钥不可读：$key"

openssl x509 -in "$cert" -noout >/dev/null 2>&1 || fail "不是有效 PEM 证书：$cert"

not_before=$(openssl x509 -in "$cert" -noout -startdate | sed 's/^notBefore=//')
not_before_epoch=$(date -d "$not_before" +%s) || fail "无法解析证书生效时间"
now=$(date +%s)
test "$now" -ge "$not_before_epoch" || fail "证书尚未生效（notBefore=$not_before）"

openssl x509 -in "$cert" -noout -checkend 0 >/dev/null 2>&1 ||
  fail "证书已过期或即将过期：$cert"

# -checkhost 的措辞随版本变化，用退出码判定，避免依赖具体输出。
openssl x509 -in "$cert" -noout -checkhost "$name" >/dev/null 2>&1 ||
  fail "证书 hostname 不匹配：$name"

exit 0
