use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ConfigDiagnostic;

/// 上传目标允许接收的本地来源类型。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UploadKind {
    /// 只接受单个普通文件。
    File,
    /// 只接受目录内容。
    Directory,
}

/// 已校验的服务端上传目标。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadTargetSpec {
    /// 相对于 Service 根目录的目标路径。
    pub path: PathBuf,
    /// 允许上传的来源类型。
    pub kind: UploadKind,
    /// 单次上传允许包含的未压缩文件总字节数。
    #[serde(default = "default_upload_max_bytes")]
    pub max_bytes: u64,
}

/// 配置前端反序列化使用的上传目标声明。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawUploadTarget {
    pub(crate) path: PathBuf,
    pub(crate) kind: UploadKind,
    #[serde(default = "default_upload_max_bytes")]
    pub(crate) max_bytes: u64,
}

impl RawUploadTarget {
    /// 校验目标保持在 Service 根目录内且不会覆盖运行时目录。
    pub(crate) fn normalize(
        self,
        field: &str,
        diagnostics: &mut Vec<ConfigDiagnostic>,
    ) -> Option<UploadTargetSpec> {
        if !safe_upload_path(&self.path) {
            diagnostics.push(ConfigDiagnostic {
                path: format!("{field}.path"),
                message: "必须是 Service 根目录内的非空相对路径，且不能进入 `.procora`".to_owned(),
            });
            return None;
        }
        if self.max_bytes == 0 {
            diagnostics.push(ConfigDiagnostic {
                path: format!("{field}.max_bytes"),
                message: "必须大于 0".to_owned(),
            });
            return None;
        }
        Some(UploadTargetSpec {
            path: self.path,
            kind: self.kind,
            max_bytes: self.max_bytes,
        })
    }
}

/// 判断上传目标是否为不触及运行时目录的普通相对路径。
pub(crate) fn safe_upload_path(path: &Path) -> bool {
    let mut components = path.components();
    let Some(Component::Normal(first)) = components.next() else {
        return false;
    };
    first != ".procora" && components.all(|component| matches!(component, Component::Normal(_)))
}

/// 默认单目标允许传输 2 GiB 未压缩内容。
const fn default_upload_max_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}
