"""通过已安装 Procora CLI 构建确定性包。"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Optional, Union


@dataclass(frozen=True)
class BuildResult:
    """一次 Procora 包构建的稳定结果。"""

    path: Path
    changed: bool
    project: str
    package_digest: str
    package_bytes: int
    files: int
    binary_variants: int


def _binary() -> str:
    """定位承载 Python API 的 Procora CLI。"""
    configured = os.environ.get("PROCORA_BIN")
    if configured:
        return configured
    binary = shutil.which("procora")
    if binary is None:
        raise RuntimeError("找不到 procora；请安装 Procora 或设置 PROCORA_BIN")
    return binary


def _project_name(binary: str, source: Union[str, os.PathLike[str]]) -> str:
    """通过共享有效配置 API 读取 Service 名称。"""
    completed = subprocess.run(
        [binary, "config", os.fspath(source)],
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    if completed.stderr:
        sys.stderr.write(completed.stderr)
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or "无法读取 Procora 配置")
    return str(json.loads(completed.stdout)["project"])


def build(
    source: Union[str, os.PathLike[str]] = ".",
    *,
    output: Optional[Union[str, os.PathLike[str]]] = None,
    project: Optional[str] = None,
    platform: str = "all",
    prepare: Iterable[str] = (),
    force: bool = False,
    procora_bin: Optional[Union[str, os.PathLike[str]]] = None,
) -> BuildResult:
    """按 `dist/<service>.pcpkg` 约定构建并返回机器可读结果。"""
    binary = os.fspath(procora_bin) if procora_bin is not None else _binary()
    if output is None:
        name = project or _project_name(binary, source)
        output_path = Path("dist") / f"{name}.pcpkg"
    else:
        output_path = Path(output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    command = [
        binary,
        "package",
        "build",
        os.fspath(source),
        "--output",
        os.fspath(output_path),
        "--platform",
        platform,
        "--json",
    ]
    for item in prepare:
        command.extend(["--prepare", item])
    if force:
        command.append("--force")
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    if completed.stderr:
        sys.stderr.write(completed.stderr)
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or "Procora 包构建失败")
    value = json.loads(completed.stdout)
    return BuildResult(
        path=Path(value["path"]),
        changed=bool(value["changed"]),
        project=str(value["project"]),
        package_digest=str(value["package_digest"]),
        package_bytes=int(value["package_bytes"]),
        files=int(value["files"]),
        binary_variants=int(value["binary_variants"]),
    )
