use serde::{Deserialize, Serialize};

use crate::config::UploadKind;

/// SSH 标准输入上传协议的当前主版本。
pub(crate) const TRANSFER_PROTOCOL_VERSION: u32 = 1;

/// 建立上传会话时由本机发送的单行 JSON 请求头。
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TransferInit {
    pub(crate) protocol: u32,
    pub(crate) target: Option<String>,
    pub(crate) source_kind: UploadKind,
    pub(crate) archive_bytes: u64,
    pub(crate) content_bytes: u64,
    pub(crate) sha256: String,
}

/// 远端提供给本机选择的兼容上传目标。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TransferTarget {
    pub(crate) selector: String,
    pub(crate) kind: UploadKind,
    pub(crate) max_bytes: u64,
}

/// 多目标协商时由本机返回的选择。
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TransferSelection {
    pub(crate) target: String,
}

/// 远端在同一 SSH 会话中返回的协商与完成消息。
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TransferResponse {
    Ready { target: String },
    Choose { targets: Vec<TransferTarget> },
    Complete { result: TransferResult },
}

/// 远端成功提交上传目标后的单行 JSON 结果。
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TransferResult {
    pub(crate) target: String,
    pub(crate) path: String,
    pub(crate) content_bytes: u64,
    pub(crate) sha256: String,
}
