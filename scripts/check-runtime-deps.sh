#!/bin/sh
# 校验 Linux 与 macOS 发布二进制没有意外的动态库依赖。
set -eu

binary=${1:-}

# 输出校验错误并退出。
fail() {
  printf '错误：%s\n' "$*" >&2
  exit 1
}

[ -n "$binary" ] || fail "用法：check-runtime-deps.sh <binary>"
[ -f "$binary" ] || fail "二进制不存在：$binary"

case "$(uname -s)" in
  Linux)
    command -v readelf >/dev/null 2>&1 || fail "缺少 readelf"
    dynamic=$(readelf -d "$binary")
    program_headers=$(readelf -l "$binary")
    printf '%s\n' "$dynamic"
    if printf '%s\n' "$dynamic" | grep -q '(NEEDED)'; then
      fail "Linux 发布二进制仍包含动态库依赖"
    fi
    if printf '%s\n' "$program_headers" | grep -q 'INTERP'; then
      fail "Linux 发布二进制仍依赖动态加载器"
    fi
    ;;
  Darwin)
    command -v otool >/dev/null 2>&1 || fail "缺少 otool"
    dependencies=$(otool -L "$binary")
    printf '%s\n' "$dependencies"
    unexpected=$(
      printf '%s\n' "$dependencies" |
        awk 'NR > 1 { print $1 }' |
        grep -Ev '^(/usr/lib/|/System/Library/)' || true
    )
    [ -z "$unexpected" ] ||
      fail "macOS 发布二进制依赖非系统动态库：$unexpected"
    ;;
  *)
    fail "不支持的校验平台：$(uname -s)"
    ;;
esac
