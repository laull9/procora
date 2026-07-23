#!/bin/sh
# 从 GitHub Releases 安装当前平台的 Procora 二进制。
set -eu

repo=${PROCORA_REPO:-laull9/procora}
install_dir=${PROCORA_INSTALL_DIR:-"$HOME/.local/bin"}
version=${PROCORA_VERSION:-latest}

# 输出安装错误并退出。
fail() {
  printf '错误：%s\n' "$*" >&2
  exit 1
}

command -v curl >/dev/null 2>&1 || fail "缺少 curl"
command -v tar >/dev/null 2>&1 || fail "缺少 tar"
command -v install >/dev/null 2>&1 || fail "缺少 install"

case "$repo" in
  ""|/*|*/|*/*/*) fail "PROCORA_REPO 必须使用 owner/repo 格式" ;;
esac
repo_owner=${repo%%/*}
repo_name=${repo#*/}
case "$repo_owner$repo_name" in
  *[!A-Za-z0-9_.-]*) fail "PROCORA_REPO 包含无效字符" ;;
esac
case "$version" in
  latest) ;;
  ""|*[!A-Za-z0-9._-]*) fail "PROCORA_VERSION 包含无效字符" ;;
esac

case "$(uname -s)" in
  Linux) platform=unknown-linux-musl ;;
  Darwin) platform=apple-darwin ;;
  *) fail "不支持的系统：$(uname -s)" ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) fail "不支持的处理器架构：$(uname -m)" ;;
esac

target="${arch}-${platform}"
asset="procora-${target}.tar.gz"
if [ "$version" = latest ]; then
  base_url="https://github.com/${repo}/releases/latest/download"
else
  base_url="https://github.com/${repo}/releases/download/${version}"
fi

temporary=$(mktemp -d "${TMPDIR:-/tmp}/procora-install.XXXXXX")
staged=
trap 'rm -rf "$temporary"; if [ -n "$staged" ]; then rm -f "$staged"; fi' EXIT INT TERM

# 下载指定发布文件，并在失败时显示来源地址。
download() {
  source_url=$1
  destination=$2
  printf '下载 %s\n' "$source_url"
  curl --fail --location --proto '=https' --tlsv1.2 \
    "$source_url" --output "$destination" ||
    fail "下载失败：$source_url"
}

download "$base_url/$asset" "$temporary/$asset"
download "$base_url/$asset.sha256" "$temporary/$asset.sha256"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$temporary" && sha256sum -c "$asset.sha256")
elif command -v shasum >/dev/null 2>&1; then
  (cd "$temporary" && shasum -a 256 -c "$asset.sha256")
else
  fail "缺少 sha256sum 或 shasum，无法验证下载文件"
fi

archive_entries=$(tar -tzf "$temporary/$asset")
[ "$archive_entries" = procora ] || fail "发布归档内容异常"
tar -C "$temporary" -xzf "$temporary/$asset"

mkdir -p "$install_dir"
staged="$install_dir/.procora-install-$$"
install -m 0755 "$temporary/procora" "$staged"
mv -f "$staged" "$install_dir/procora"
staged=

printf 'Procora 已安装到 %s\n' "$install_dir/procora"
case ":${PATH:-}:" in
  *":$install_dir:"*) ;;
  *) printf '提示：请把 %s 加入 PATH。\n' "$install_dir" ;;
esac
