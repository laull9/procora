//! 日志来源过滤器状态。

/// 日志页当前保留的内容来源。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LogSourceFilter {
    /// 同时展示 Procora 诊断与子进程输出。
    #[default]
    All,
    /// 仅展示 Procora 生成的诊断日志。
    Procora,
    /// 仅展示子进程 stdout/stderr。
    Child,
}

impl LogSourceFilter {
    /// 返回来源过滤器的界面标签。
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Procora => "Procora",
            Self::Child => "子进程",
        }
    }

    /// 切换到下一个来源过滤器。
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::All => Self::Procora,
            Self::Procora => Self::Child,
            Self::Child => Self::All,
        }
    }
}
