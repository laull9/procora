use std::collections::BTreeMap;

use serde::Serialize;

/// 有效配置字段最终值来自哪一层。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueOrigin {
    /// 配置模式内建默认值。
    BuiltIn,
    /// 项目级默认环境。
    ProjectEnv,
    /// 项目级 Task 默认声明。
    TaskDefaults,
    /// 当前选中 profile 的项目环境或 Task 默认覆盖。
    Profile,
    /// 命名 Task 模板。
    TaskTemplate,
    /// Task 显式环境文件。
    EnvFile,
    /// Task 自身显式声明。
    Task,
}

/// 单个 Task 的字段和最终环境变量来源。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TaskConfigOrigins {
    /// 以有效配置字段路径为键的来源。
    pub fields: BTreeMap<String, ValueOrigin>,
    /// 以最终环境变量名为键的来源。
    pub env: BTreeMap<String, ValueOrigin>,
    /// 模板来源字段到最终获胜模板名称的映射。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub templates: BTreeMap<String, String>,
    /// 模板来源环境键到最终获胜模板名称的映射。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub template_env: BTreeMap<String, String>,
    /// 依赖边到最终生效层的映射。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub depends_on: BTreeMap<String, ValueOrigin>,
    /// 模板来源依赖边到最终获胜模板名称的映射。
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub template_depends_on: BTreeMap<String, String>,
}

impl TaskConfigOrigins {
    /// 返回字段来源；缺失路径视为内建默认。
    pub fn field(&self, path: &str) -> ValueOrigin {
        self.fields
            .get(path)
            .copied()
            .unwrap_or(ValueOrigin::BuiltIn)
    }

    /// 返回模板来源字段的具体模板名称。
    pub fn template(&self, path: &str) -> Option<&str> {
        self.templates.get(path).map(String::as_str)
    }
}
