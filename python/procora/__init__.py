"""Procora 的 Python 配置、包构建和 MCP API。"""

from .model import Project, Service, Task, project
from .package import BuildResult, build
from .mcp import McpClient, McpError

__all__ = [
    "BuildResult",
    "McpClient",
    "McpError",
    "Project",
    "Service",
    "Task",
    "build",
    "project",
]


def version() -> str:
    """返回承载当前 Python 包的 Procora 版本。"""
    import os
    import subprocess

    try:
        from ._version import VERSION

        return VERSION
    except ImportError:
        pass
    embedded = os.environ.get("PROCORA_VERSION")
    if embedded:
        return embedded
    try:
        completed = subprocess.run(
            [os.environ.get("PROCORA_BIN", "procora"), "--version"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return "unknown"
    return completed.stdout.strip().removeprefix("procora ")


__version__ = version()
