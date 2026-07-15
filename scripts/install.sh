#!/bin/sh
# 从 GitHub Releases 安装当前平台的 Procora 二进制。
set -eu

repo=${PROCORA_REPO:-laull/procora}
install_dir=${PROCORA_INSTALL_DIR:-"$HOME/.local/bin"}
version=${PROCORA_VERSION:-latest}

case "$(uname -s)" in
  Linux) platform=unknown-linux-gnu ;;
  Darwin) platform=apple-darwin ;;
  *) echo "不支持的系统：$(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "不支持的处理器架构：$(uname -m)" >&2; exit 1 ;;
esac

target="${arch}-${platform}"
asset="procora-${target}.tar.gz"
if [ "$version" = latest ]; then
  url="https://github.com/${repo}/releases/latest/download/${asset}"
else
  url="https://github.com/${repo}/releases/download/${version}/${asset}"
fi

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT INT TERM
curl --fail --location --proto '=https' --tlsv1.2 "$url" --output "$temporary/$asset"
curl --fail --location --proto '=https' --tlsv1.2 "$url.sha256" --output "$temporary/$asset.sha256"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$temporary" && sha256sum -c "$asset.sha256")
elif command -v shasum >/dev/null 2>&1; then
  (cd "$temporary" && shasum -a 256 -c "$asset.sha256")
else
  echo "缺少 sha256sum 或 shasum，无法验证下载文件" >&2
  exit 1
fi
tar -C "$temporary" -xzf "$temporary/$asset"
mkdir -p "$install_dir"
install -m 0755 "$temporary/procora" "$install_dir/procora"
echo "Procora 已安装到 $install_dir/procora"
