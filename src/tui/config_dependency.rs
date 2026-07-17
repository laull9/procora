//! 结构化编辑器中的管理依赖表单模型。

use std::collections::BTreeMap;

use serde::Serialize;

/// 表单中的管理依赖值对象。
#[derive(Clone, Debug)]
pub(crate) struct FormDependency {
    /// 下载或本地来源。
    pub(crate) source: String,
    /// 主来源不可用时依次尝试的镜像。
    pub(crate) mirrors: Vec<String>,
    /// 固定版本。
    pub(crate) version: String,
    /// 可选 SHA-256。
    pub(crate) checksum: Option<String>,
    /// 解包策略。
    pub(crate) unpack: String,
    /// 归档内相对路径。
    pub(crate) path: Option<String>,
    /// 最终内容类型。
    pub(crate) kind: String,
    /// 可选验证规则。
    pub(crate) verify: Option<FormVerify>,
    /// 下载可靠性与资源边界。
    pub(crate) download: FormDependencyDownload,
    /// SSH 显式认证和主机密钥文件。
    pub(crate) ssh: FormDependencySsh,
}

/// 表单中的依赖下载策略。
#[derive(Clone, Debug)]
pub(crate) struct FormDependencyDownload {
    /// 首次失败后的重试次数。
    pub(crate) retries: u8,
    /// 单次传输总超时。
    pub(crate) timeout_ms: u64,
    /// 最大下载字节数。
    pub(crate) max_bytes: u64,
    /// HTTP 请求头。
    pub(crate) headers: BTreeMap<String, String>,
}

impl FormDependencyDownload {
    /// 返回是否完全使用内建下载策略。
    pub(super) fn is_default(&self) -> bool {
        self.retries == 2
            && self.timeout_ms == 120_000
            && self.max_bytes == 2 * 1024 * 1024 * 1024
            && self.headers.is_empty()
    }
}

/// 表单中的 SSH 下载参数。
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct FormDependencySsh {
    /// 可选 OpenSSH 私钥路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) identity_file: Option<String>,
    /// 可选 OpenSSH `known_hosts` 路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) known_hosts_file: Option<String>,
}

impl FormDependencySsh {
    /// 返回表单是否没有声明 SSH 文件。
    pub(super) fn is_empty(&self) -> bool {
        self.identity_file.is_none() && self.known_hosts_file.is_none()
    }
}

impl FormDependency {
    /// 返回是否可无损保存为单个来源字符串。
    pub(super) fn is_compact(&self) -> bool {
        self.mirrors.is_empty()
            && self.version == "source"
            && self.checksum.is_none()
            && self.unpack == "auto"
            && self.path.is_none()
            && self.kind == "auto"
            && self.verify.is_none()
            && self.download.is_default()
            && self.ssh.is_empty()
    }
}

/// 表单中的依赖版本验证规则。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FormVerify {
    /// 验证程序。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    /// 验证参数。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    /// 预期输出片段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) contains: Option<String>,
}
