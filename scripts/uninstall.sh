#!/bin/sh
# 停止 Procora 自动托管并删除当前用户安装的命令。
set -eu

install_dir=${PROCORA_INSTALL_DIR:-"$HOME/.local/bin"}
binary="$install_dir/procora"
force=${PROCORA_FORCE_UNINSTALL:-0}

if [ ! -e "$binary" ]; then
  printf '未找到 Procora：%s\n' "$binary"
  exit 0
fi

if [ -x "$binary" ] && ! "$binary" disable; then
  if [ "$force" != 1 ]; then
    printf '%s\n' \
      "停用 Procora 开机自启动失败，尚未删除程序。" \
      "确认无需保留后台托管后，可设置 PROCORA_FORCE_UNINSTALL=1 强制删除。" >&2
    exit 1
  fi
  printf '%s\n' "警告：未能停用开机自启动，正在强制删除程序。" >&2
fi

rm -f "$binary"
printf '已删除 %s\n' "$binary"
printf '%s\n' "运行状态、数据库和服务日志均已保留。"
