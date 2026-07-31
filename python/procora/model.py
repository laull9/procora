"""约定优先的 Procora Service 配置模型。"""

from __future__ import annotations

import atexit
import json
import os
import shlex
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Mapping, Optional, Sequence, Union

Command = Union[str, Sequence[Union[str, os.PathLike[str]]]]
Dependency = Union[str, Mapping[str, Any]]


def _strings(values: Iterable[Any]) -> list[str]:
    """把路径等参数稳定转换为字符串。"""
    return [os.fspath(value) for value in values]


def _command(command: Optional[Command]) -> tuple[Optional[str], Optional[list[str]]]:
    """把字符串或 argv 风格命令转换为 Procora 字段。"""
    if command is None:
        return None, None
    if isinstance(command, str):
        return command, None
    values = _strings(command)
    if not values:
        raise ValueError("Task command 不能为空")
    return values[0], values[1:]


def _clean(value: Any) -> Any:
    """递归移除未声明值，同时保留显式空集合。"""
    if isinstance(value, dict):
        return {key: _clean(item) for key, item in value.items() if item is not None}
    if isinstance(value, list):
        return [_clean(item) for item in value]
    if isinstance(value, Path):
        return os.fspath(value)
    return value


@dataclass
class Task:
    """一个可增量组合的 Procora Task 声明。"""

    name: str
    command: Optional[Command] = None
    args: Optional[Sequence[Union[str, os.PathLike[str]]]] = None
    cwd: Optional[Union[str, os.PathLike[str]]] = None
    env: Mapping[str, Any] = field(default_factory=dict)
    env_file: Optional[Union[str, os.PathLike[str]]] = None
    depends_on: Union[Sequence[str], Mapping[str, Dependency]] = field(default_factory=list)
    healthcheck: Optional[Mapping[str, Any]] = None
    success_exit_codes: Optional[Sequence[int]] = None
    restart: Optional[str] = None
    restart_delay: Optional[Union[str, int]] = None
    max_restarts: Optional[int] = None
    restart_reset_after: Optional[Union[str, int]] = None
    shutdown_timeout: Optional[Union[str, int]] = None
    extends: Optional[str] = None
    uploads: Mapping[str, Mapping[str, Any]] = field(default_factory=dict)
    extra: Mapping[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """生成由 Rust 共享校验管线消费的原始 Task。"""
        command, command_args = _command(self.command)
        args = _strings(self.args) if self.args is not None else command_args
        data: dict[str, Any] = {
            "extends": self.extends,
            "command": command,
            "args": args,
            "cwd": os.fspath(self.cwd) if self.cwd is not None else None,
            "env": {str(key): str(value) for key, value in self.env.items()},
            "env_file": os.fspath(self.env_file) if self.env_file is not None else None,
            "depends_on": self.depends_on,
            "healthcheck": self.healthcheck,
            "success_exit_codes": list(self.success_exit_codes)
            if self.success_exit_codes is not None
            else None,
            "restart": self.restart,
            "restart_delay": self.restart_delay,
            "max_restarts": self.max_restarts,
            "restart_reset_after": self.restart_reset_after,
            "shutdown_timeout": self.shutdown_timeout,
            "uploads": dict(self.uploads),
        }
        data.update(self.extra)
        return _clean(data)


class Project:
    """Procora Service 的 Python 声明入口。"""

    def __init__(
        self,
        name: str,
        *,
        env: Optional[Mapping[str, Any]] = None,
        vars: Optional[Mapping[str, Any]] = None,
        task_defaults: Optional[Mapping[str, Any]] = None,
        profile: Optional[str] = None,
        **extra: Any,
    ) -> None:
        if not name or not isinstance(name, str):
            raise ValueError("Project name 必须是非空字符串")
        self.name = name
        self.env = {str(key): str(value) for key, value in (env or {}).items()}
        self.vars = {str(key): str(value) for key, value in (vars or {}).items()}
        self.task_defaults = dict(task_defaults or {})
        self.active_profile = profile
        self.tasks: dict[str, Task] = {}
        self.task_templates: dict[str, dict[str, Any]] = {}
        self.profiles: dict[str, dict[str, Any]] = {}
        self.dependencies: dict[str, dict[str, Any]] = {}
        self.binaries: dict[str, dict[str, Any]] = {}
        self.uploads: dict[str, dict[str, Any]] = {}
        self.extra = extra
        self._emitted = False
        _PROJECTS.append(self)

    def task(
        self,
        name: str,
        command: Optional[Command] = None,
        **options: Any,
    ) -> Task:
        """声明 Task；重复名称会立即失败。"""
        if name in self.tasks:
            raise ValueError(f"Task `{name}` 已声明")
        known = {
            key: options.pop(key)
            for key in list(options)
            if key in Task.__dataclass_fields__ and key not in {"name", "command", "extra"}
        }
        task = Task(name=name, command=command, extra=options, **known)
        self.tasks[name] = task
        return task

    def template(self, name: str, command: Optional[Command] = None, **options: Any) -> "Project":
        """声明可被 Task extends 的命名模板。"""
        self.task_templates[name] = Task(name=name, command=command, extra=options).to_dict()
        return self

    def profile(self, name: str, **config: Any) -> "Project":
        """声明一个 profile 原始覆盖层。"""
        self.profiles[name] = _clean(config)
        return self

    def dependency(self, name: str, **config: Any) -> "Project":
        """声明管理依赖，字段直接交给 Rust 严格校验。"""
        self.dependencies[name] = _clean(config)
        return self

    def binary(
        self,
        name: str,
        *,
        target: Union[str, os.PathLike[str]],
        variants: Mapping[str, Union[str, os.PathLike[str], Mapping[str, Any]]],
    ) -> "Project":
        """声明逻辑二进制及多平台产物。"""
        normalized: dict[str, Any] = {}
        for platform, variant in variants.items():
            normalized[platform] = (
                dict(variant) if isinstance(variant, Mapping) else os.fspath(variant)
            )
        self.binaries[name] = {"target": os.fspath(target), "variants": normalized}
        return self

    def upload(
        self,
        name: str,
        *,
        path: Union[str, os.PathLike[str]],
        kind: str = "file",
        **options: Any,
    ) -> "Project":
        """声明项目级上传目标。"""
        value = {"path": os.fspath(path), "kind": kind, **options}
        self.uploads[name] = _clean(value)
        return self

    def to_dict(self) -> dict[str, Any]:
        """生成稳定、可 JSON 序列化的 Procora v1 配置。"""
        data: dict[str, Any] = {
            "version": 1,
            "project": self.name,
            "vars": self.vars,
            "env": self.env,
            "task_defaults": self.task_defaults,
            "profile": self.active_profile,
            "task_templates": self.task_templates,
            "profiles": self.profiles,
            "dependencies": self.dependencies,
            "binaries": self.binaries,
            "uploads": self.uploads,
            "tasks": {name: task.to_dict() for name, task in self.tasks.items()},
        }
        data.update(self.extra)
        return _clean(data)

    def emit(self) -> None:
        """向 stdout 输出单个严格 JSON 文档。"""
        json.dump(self.to_dict(), sys.stdout, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        sys.stdout.write("\n")
        self._emitted = True

    def build(self, **options: Any):
        """直接运行脚本时按约定构建包；由 Procora 加载时只输出配置。"""
        if os.environ.get("PROCORA_CONFIG") == "1":
            if not self._emitted:
                self.emit()
            return None
        from .package import build

        return build(project=self.name, **options)

    def command(self, name: str) -> str:
        """返回一个 Task 的 shell 可读命令，仅用于诊断显示。"""
        task = self.tasks[name]
        command, args = _command(task.command)
        return shlex.join([command or "", *(args or [])])


Service = Project
_PROJECTS: list[Project] = []


def project(name: str, **options: Any) -> Project:
    """创建并返回默认 Project。"""
    return Project(name, **options)


def _emit_default_project() -> None:
    """在配置模式下自动输出唯一声明的 Project。"""
    if os.environ.get("PROCORA_CONFIG") != "1":
        return
    pending = [item for item in _PROJECTS if not item._emitted]
    if len(pending) == 1:
        pending[0].emit()
    elif len(pending) > 1:
        sys.stderr.write("Procora Python 配置必须只声明一个 Project\n")


atexit.register(_emit_default_project)
