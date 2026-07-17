use std::{
    collections::BTreeMap,
    fmt,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
    ser::{SerializeMap, SerializeSeq},
};

use super::{RawRestartPolicy, command::RawCommand};
use crate::config::health::RawHealthCheck;

/// 配置前端反序列化使用的原始 Task DTO。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawTask {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) command: Option<RawCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) env: BTreeMap<String, String>,
    #[serde(skip, default)]
    pub(super) inline_env_before_file: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) env_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) healthcheck: Option<RawHealthCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) success_exit_codes: Option<Vec<i32>>,
    #[serde(default, skip_serializing_if = "RawDependencies::is_empty")]
    pub(super) depends_on: RawDependencies,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) restart: Option<RawRestartPolicy>,
    #[serde(
        rename = "restart_delay",
        alias = "restart_delay_ms",
        default,
        deserialize_with = "crate::config::deserialize_optional_duration",
        serialize_with = "crate::config::serialize_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) restart_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_restarts: Option<u32>,
    #[serde(
        rename = "restart_reset_after",
        alias = "restart_reset_after_ms",
        default,
        deserialize_with = "crate::config::deserialize_optional_duration",
        serialize_with = "crate::config::serialize_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) restart_reset_after_ms: Option<u64>,
    #[serde(
        rename = "shutdown_timeout",
        alias = "shutdown_timeout_ms",
        default,
        deserialize_with = "crate::config::deserialize_optional_duration",
        serialize_with = "crate::config::serialize_optional_duration",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) shutdown_timeout_ms: Option<u64>,
}

/// 原始配置中的依赖边 DTO。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDependency {
    #[serde(default)]
    pub(super) condition: RawDependencyCondition,
}

/// 原始配置支持的依赖条件拼写。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RawDependencyCondition {
    /// 上游进程已创建。
    #[default]
    #[serde(alias = "process_started")]
    Started,
    /// 上游达到健康阈值。
    #[serde(alias = "process_healthy")]
    Healthy,
    /// 上游以成功退出码结束。
    #[serde(alias = "process_completed_successfully")]
    CompletedSuccessfully,
}

/// 原始依赖集合，输入支持名称列表、条件标量和旧版条件对象。
#[derive(Clone, Debug, Default)]
pub(super) struct RawDependencies(BTreeMap<String, RawDependency>);

impl RawDependencies {
    /// 返回依赖集合是否为空。
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Deref for RawDependencies {
    type Target = BTreeMap<String, RawDependency>;

    /// 让合并与校验逻辑继续按确定性 map 操作依赖边。
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RawDependencies {
    /// 允许模板覆盖按依赖名称合并。
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for RawDependencies {
    type Item = (String, RawDependency);
    type IntoIter = std::collections::btree_map::IntoIter<String, RawDependency>;

    /// 消费原始依赖集合并保持名称排序。
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'de> Deserialize<'de> for RawDependencies {
    /// 同时接受 `[db]`、`{db: healthy}` 与 `{db: {condition: healthy}}`。
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawDependenciesVisitor)
    }
}

impl Serialize for RawDependencies {
    /// 全部默认条件时输出列表，否则输出名称到条件的紧凑 map。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self
            .0
            .values()
            .all(|dependency| dependency.condition == RawDependencyCondition::Started)
        {
            let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
            for name in self.0.keys() {
                sequence.serialize_element(name)?;
            }
            sequence.end()
        } else {
            let mut map = serializer.serialize_map(Some(self.0.len()))?;
            for (name, dependency) in &self.0 {
                map.serialize_entry(name, &dependency.condition)?;
            }
            map.end()
        }
    }
}

/// 依赖集合的严格序列或 map 访问器。
struct RawDependenciesVisitor;

impl<'de> Visitor<'de> for RawDependenciesVisitor {
    type Value = RawDependencies;

    /// 描述合法的依赖集合输入形状。
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Task 名称数组，或 Task 名称到依赖条件/条件对象的映射")
    }

    /// 把名称数组转换为默认 `started` 条件，并拒绝重复项。
    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut dependencies = BTreeMap::new();
        let mut index = 0_usize;
        while let Some(name) = sequence.next_element::<StrictString>()? {
            if dependencies
                .insert(name.0.clone(), RawDependency::default())
                .is_some()
            {
                return Err(A::Error::custom(format!(
                    "依赖列表第 {index} 项 `{}` 重复出现",
                    name.0
                )));
            }
            index += 1;
        }
        Ok(RawDependencies(dependencies))
    }

    /// 读取紧凑条件标量或兼容的旧版条件对象。
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut dependencies = BTreeMap::new();
        while let Some(name) = map.next_key::<StrictString>()? {
            let value = map.next_value::<RawDependencyValue>()?.0;
            if dependencies.insert(name.0.clone(), value).is_some() {
                return Err(A::Error::custom(format!("依赖 `{}` 重复声明", name.0)));
            }
        }
        Ok(RawDependencies(dependencies))
    }
}

/// 严格字符串，避免 YAML 数字或布尔值被隐式当成 Task 名称。
struct StrictString(String);

impl<'de> Deserialize<'de> for StrictString {
    /// 只接受真正的字符串值。
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictStringVisitor)
    }
}

/// 严格字符串访问器。
struct StrictStringVisitor;

impl Visitor<'_> for StrictStringVisitor {
    type Value = StrictString;

    /// 描述依赖名称必须是字符串。
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Task 名称字符串")
    }

    /// 接收借用字符串。
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictString(value.to_owned()))
    }

    /// 接收所有权字符串。
    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictString(value))
    }
}

/// map 中一条依赖的紧凑条件或旧版对象值。
struct RawDependencyValue(RawDependency);

impl<'de> Deserialize<'de> for RawDependencyValue {
    /// 按输入类型选择条件标量或对象解析。
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RawDependencyValueVisitor)
    }
}

/// 单条依赖值访问器。
struct RawDependencyValueVisitor;

impl<'de> Visitor<'de> for RawDependencyValueVisitor {
    type Value = RawDependencyValue;

    /// 描述合法的依赖条件输入。
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("依赖条件字符串或包含 condition 的对象")
    }

    /// 解析紧凑条件字符串及 process-compose 兼容别名。
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let condition = match value {
            "started" | "process_started" => RawDependencyCondition::Started,
            "healthy" | "process_healthy" => RawDependencyCondition::Healthy,
            "completed_successfully" | "process_completed_successfully" => {
                RawDependencyCondition::CompletedSuccessfully
            }
            _ => {
                return Err(E::custom(format!(
                    "未知依赖条件 `{value}`；应为 started、healthy 或 completed_successfully"
                )));
            }
        };
        Ok(RawDependencyValue(RawDependency { condition }))
    }

    /// 解析旧版 `{condition: ...}` 对象。
    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        RawDependency::deserialize(MapAccessDeserializer::new(map)).map(RawDependencyValue)
    }
}
