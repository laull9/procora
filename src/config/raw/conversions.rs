use crate::{
    config::{DependencyKind, UnpackMode},
    core::{DependencyCondition, RestartPolicy},
};

use super::{RawDependencyCondition, RawDependencyKind, RawRestartPolicy, RawUnpackMode};

impl From<RawDependencyCondition> for DependencyCondition {
    /// 把配置拼写映射为领域依赖条件。
    fn from(value: RawDependencyCondition) -> Self {
        match value {
            RawDependencyCondition::Started => Self::Started,
            RawDependencyCondition::Healthy => Self::Healthy,
            RawDependencyCondition::CompletedSuccessfully => Self::CompletedSuccessfully,
        }
    }
}

impl From<RawRestartPolicy> for RestartPolicy {
    /// 把配置拼写映射为领域重启策略。
    fn from(value: RawRestartPolicy) -> Self {
        match value {
            RawRestartPolicy::Never => Self::Never,
            RawRestartPolicy::OnFailure => Self::OnFailure,
            RawRestartPolicy::Always => Self::Always,
        }
    }
}

impl From<RawDependencyKind> for DependencyKind {
    /// 把配置拼写映射为依赖内容类型。
    fn from(value: RawDependencyKind) -> Self {
        match value {
            RawDependencyKind::Auto => Self::Auto,
            RawDependencyKind::Binary => Self::Binary,
            RawDependencyKind::File => Self::File,
            RawDependencyKind::Directory => Self::Directory,
        }
    }
}

impl From<RawUnpackMode> for UnpackMode {
    /// 把配置拼写映射为解包模式。
    fn from(value: RawUnpackMode) -> Self {
        match value {
            RawUnpackMode::Auto => Self::Auto,
            RawUnpackMode::Never => Self::Never,
        }
    }
}
