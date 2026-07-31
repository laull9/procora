"""仅使用标准库的 Procora MCP stdio 客户端。"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from typing import Any, Mapping, Optional, Union


class McpError(RuntimeError):
    """MCP 传输、协议或工具错误。"""


class McpClient:
    """面向 Python 自动化脚本的同步 Procora MCP 客户端。"""

    def __init__(self, procora_bin: Optional[Union[str, os.PathLike[str]]] = None) -> None:
        binary = os.fspath(procora_bin) if procora_bin is not None else (
            os.environ.get("PROCORA_BIN") or shutil.which("procora")
        )
        if not binary:
            raise McpError("找不到 procora；请安装 Procora 或设置 PROCORA_BIN")
        self._process = subprocess.Popen(
            [binary, "mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self._next_id = 1
        self._request(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "procora-python", "version": "1"},
            },
        )
        self._notify("notifications/initialized", {})

    def _write(self, message: Mapping[str, Any]) -> None:
        """发送一条换行分隔的 JSON-RPC 消息。"""
        if self._process.stdin is None:
            raise McpError("MCP stdin 已关闭")
        self._process.stdin.write(json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n")
        self._process.stdin.flush()

    def _request(self, method: str, params: Mapping[str, Any]) -> Any:
        """发送请求并等待匹配的响应。"""
        request_id = self._next_id
        self._next_id += 1
        self._write({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        if self._process.stdout is None:
            raise McpError("MCP stdout 已关闭")
        while True:
            line = self._process.stdout.readline()
            if not line:
                stderr = self._process.stderr.read() if self._process.stderr is not None else ""
                raise McpError(stderr.strip() or "Procora MCP 意外退出")
            message = json.loads(line)
            if message.get("id") != request_id:
                continue
            if "error" in message:
                raise McpError(str(message["error"]))
            return message.get("result")

    def _notify(self, method: str, params: Mapping[str, Any]) -> None:
        """发送不要求响应的 JSON-RPC 通知。"""
        self._write({"jsonrpc": "2.0", "method": method, "params": params})

    def tools(self) -> list[dict[str, Any]]:
        """列出 Procora MCP 当前暴露的全部工具。"""
        result = self._request("tools/list", {})
        return list(result.get("tools", []))

    def call(self, name: str, **arguments: Any) -> Any:
        """调用工具并返回结构化结果。"""
        result = self._request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            content = result.get("content", [])
            message = next((item.get("text") for item in content if item.get("type") == "text"), None)
            raise McpError(message or f"MCP 工具 `{name}` 失败")
        return result.get("structuredContent", result)

    def close(self) -> None:
        """关闭 MCP 子进程和管道。"""
        if self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait()

    def __enter__(self) -> "McpClient":
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()
