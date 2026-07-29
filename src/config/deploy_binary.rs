//! 裸机部署使用的目标平台与本地二进制变体声明。

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::core::HealthCheckProbe;

use super::{CompiledProject, ConfigDiagnostic};

mod platform;

use platform::{
    normalize_arch, normalize_environment, normalize_os, normalize_selector_arch, safe_target,
    target_environment,
};

/// 远端Procora报告的规范化运行平台。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeployPlatform {
    /// 规范化操作系统名称。
    pub os: String,
    /// 规范化CPU架构名称。
    pub arch: String,
    /// 可选ABI或工具链环境。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
}

impl DeployPlatform {
    /// 返回当前Procora二进制自身的编译目标平台。
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            environment: target_environment(),
        }
    }

    /// 校验并规范化远端报告的平台字段。
    ///
    /// # Errors
    ///
    /// 当操作系统、架构或环境名称不受支持时返回诊断。
    pub fn normalized(self) -> Result<Self, String> {
        let os = normalize_os(&self.os)
            .ok_or_else(|| format!("远端报告了不支持的操作系统 `{}`", self.os))?;
        let arch = normalize_arch(&self.arch)
            .ok_or_else(|| format!("远端报告了不支持的架构 `{}`", self.arch))?;
        let environment = self
            .environment
            .as_deref()
            .map(normalize_environment)
            .transpose()?;
        Ok(Self {
            os: os.to_owned(),
            arch: arch.to_owned(),
            environment: environment.map(str::to_owned),
        })
    }

    /// 返回适合诊断和配置键的稳定平台文本。
    pub fn key(&self) -> String {
        self.environment.as_ref().map_or_else(
            || format!("{}-{}", self.os, self.arch),
            |environment| format!("{}-{}-{environment}", self.os, self.arch),
        )
    }
}

/// 单个平台变体的本地构建产物。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeployBinaryVariantSpec {
    /// 适用的远端平台选择器。
    pub selector: DeployPlatformSelector,
    /// 开发机上的本地构建产物。
    pub source: PathBuf,
    /// 该平台需要覆盖的release目标，例如Windows使用`.exe`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
}

/// 一个逻辑二进制在release内的目标及其平台矩阵。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeployBinarySpec {
    /// 二进制在不可变release内的稳定相对路径。
    pub target: PathBuf,
    /// 可供远端平台选择的本地产物矩阵。
    pub variants: Vec<DeployBinaryVariantSpec>,
}

/// 已校验的全部部署二进制。
pub type DeployBinaries = BTreeMap<String, DeployBinarySpec>;

/// 为一个远端平台选出的单个本地产物。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SelectedDeployBinary {
    /// 逻辑二进制名称。
    pub name: String,
    /// 被选中的本地产物路径。
    pub source: PathBuf,
    /// release内的稳定目标路径。
    pub target: PathBuf,
    /// 规范化后的命中平台键。
    pub selector: String,
}

/// 配置文件中的单个二进制声明。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDeployBinary {
    target: PathBuf,
    variants: BTreeMap<String, RawDeployBinaryVariant>,
}

/// 变体既支持路径简写，也保留未来扩展的对象形式。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum RawDeployBinaryVariant {
    Source(PathBuf),
    Detailed(RawDeployBinaryVariantFields),
}

/// 单个二进制变体的完整字段。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawDeployBinaryVariantFields {
    source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<PathBuf>,
}

/// 已规范化的平台选择器。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeployPlatformSelector {
    os: String,
    arch: String,
    environment: Option<String>,
}

impl RawDeployBinary {
    /// 把include内的本地产物路径重定位到声明文件目录。
    pub(crate) fn rebase(&mut self, base: &Path) {
        for variant in self.variants.values_mut() {
            let source = match variant {
                RawDeployBinaryVariant::Source(source) => source,
                RawDeployBinaryVariant::Detailed(fields) => &mut fields.source,
            };
            if !source.is_absolute() {
                *source = crate::platform::simplify_path(&base.join(&*source));
            }
        }
    }

    /// 规范化单个逻辑二进制及其全部平台键。
    fn normalize(self, name: &str, diagnostics: &mut Vec<ConfigDiagnostic>) -> DeployBinarySpec {
        let field = format!("binaries.{name}");
        if !super::raw::valid_dependency_id(name) {
            diagnostics.push(super::raw::diagnostic(
                &field,
                "二进制名称只能包含 ASCII 字母、数字、点、短横线和下划线",
            ));
        }
        if !safe_target(&self.target) {
            diagnostics.push(super::raw::diagnostic(
                format!("{field}.target"),
                "必须是 Service 内不含 `.procora` 的普通相对文件路径",
            ));
        }
        if self.variants.is_empty() {
            diagnostics.push(super::raw::diagnostic(
                format!("{field}.variants"),
                "至少需要一个平台变体",
            ));
        }
        let mut normalized_keys = BTreeSet::new();
        let variants = self
            .variants
            .into_iter()
            .filter_map(|(key, raw)| {
                let selector = match DeployPlatformSelector::parse(&key) {
                    Ok(selector) => selector,
                    Err(message) => {
                        diagnostics.push(super::raw::diagnostic(
                            format!("{field}.variants.{key}"),
                            message,
                        ));
                        return None;
                    }
                };
                if !normalized_keys.insert(selector.key()) {
                    diagnostics.push(super::raw::diagnostic(
                        format!("{field}.variants.{key}"),
                        "与另一个平台键规范化后重复",
                    ));
                    return None;
                }
                let (source, target) = match raw {
                    RawDeployBinaryVariant::Source(source) => (source, None),
                    RawDeployBinaryVariant::Detailed(fields) => {
                        if let Some(target) = fields.target.as_ref()
                            && !safe_target(target)
                        {
                            diagnostics.push(super::raw::diagnostic(
                                format!("{field}.variants.{key}.target"),
                                "必须是 Service 内不含 `.procora` 的普通相对文件路径",
                            ));
                        }
                        (fields.source, fields.target)
                    }
                };
                if source.as_os_str().is_empty() {
                    diagnostics.push(super::raw::diagnostic(
                        format!("{field}.variants.{key}.source"),
                        "本地产物路径不能为空",
                    ));
                    return None;
                }
                Some(DeployBinaryVariantSpec {
                    selector,
                    source,
                    target,
                })
            })
            .collect();
        DeployBinarySpec {
            target: self.target,
            variants,
        }
    }
}

impl DeployPlatformSelector {
    /// 解析`os-arch[-environment]`平台键并接受常用无歧义别名。
    fn parse(value: &str) -> Result<Self, String> {
        let mut parts = value.split('-');
        let raw_os = parts.next().unwrap_or_default();
        let raw_arch = parts.next().unwrap_or_default();
        let raw_environment = parts.next();
        if raw_os.is_empty() || raw_arch.is_empty() || parts.next().is_some() {
            return Err("平台键必须是 `os-arch` 或 `os-arch-environment`".to_owned());
        }
        let os = normalize_os(raw_os).ok_or_else(|| {
            format!("不支持操作系统 `{raw_os}`；支持 linux、macos、windows、freebsd")
        })?;
        let arch = normalize_selector_arch(raw_arch).ok_or_else(|| {
            format!(
                "不支持架构 `{raw_arch}`；支持 x86_64/amd64、aarch64/arm64、x86、arm、macOS universal"
            )
        })?;
        let environment = raw_environment.map(normalize_environment).transpose()?;
        if arch == "universal" && (os != "macos" || environment.is_some()) {
            return Err("`universal` 只可用于不带 environment 的 macOS 变体".to_owned());
        }
        Ok(Self {
            os: os.to_owned(),
            arch: arch.to_owned(),
            environment: environment.map(str::to_owned),
        })
    }

    /// 判断选择器是否适用于远端平台。
    fn matches(&self, platform: &DeployPlatform) -> bool {
        self.os == platform.os
            && (self.arch == platform.arch
                || (self.os == "macos"
                    && self.arch == "universal"
                    && matches!(platform.arch.as_str(), "x86_64" | "aarch64")))
            && self
                .environment
                .as_ref()
                .is_none_or(|environment| Some(environment) == platform.environment.as_ref())
    }

    /// 环境精确匹配比通用OS/架构匹配优先。
    fn specificity(&self) -> usize {
        usize::from(self.arch != "universal") * 2 + usize::from(self.environment.is_some())
    }

    /// 返回规范化配置键。
    fn key(&self) -> String {
        self.environment.as_ref().map_or_else(
            || format!("{}-{}", self.os, self.arch),
            |environment| format!("{}-{}-{environment}", self.os, self.arch),
        )
    }
}

/// 规范化全部部署二进制并拒绝目标路径冲突。
pub(crate) fn normalize_deploy_binaries(
    binaries: BTreeMap<String, RawDeployBinary>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> DeployBinaries {
    let mut targets = BTreeMap::<PathBuf, String>::new();
    let mut output = BTreeMap::new();
    for (name, raw) in binaries {
        let spec = raw.normalize(&name, diagnostics);
        for target in std::iter::once(&spec.target).chain(
            spec.variants
                .iter()
                .filter_map(|variant| variant.target.as_ref()),
        ) {
            if let Some(existing) = targets.insert(target.clone(), name.clone())
                && existing != name
            {
                diagnostics.push(super::raw::diagnostic(
                    format!("binaries.{name}.target"),
                    format!("与二进制 `{existing}` 使用了相同 target"),
                ));
            }
        }
        output.insert(name, spec);
    }
    output
}

/// 为远端平台选择每个逻辑二进制的唯一最高优先级变体。
///
/// # Errors
///
/// 当任一逻辑二进制没有匹配远端平台的变体时返回支持矩阵。
pub fn select_deploy_binaries(
    binaries: &DeployBinaries,
    platform: &DeployPlatform,
) -> Result<Vec<SelectedDeployBinary>, String> {
    binaries
        .iter()
        .map(|(name, spec)| select_one(name, spec, platform))
        .collect()
}

/// 把`${binary.name}`替换为release内对应文件的绝对路径。
pub(crate) fn apply_deploy_binary_placeholders(compiled: &mut CompiledProject, root: &Path) {
    let platform = DeployPlatform::current().normalized().ok();
    let selected = platform
        .as_ref()
        .and_then(|platform| select_deploy_binaries(&compiled.deploy_binaries, platform).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|binary| (binary.name, binary.target))
        .collect::<BTreeMap<_, _>>();
    for task in compiled.spec.tasks.values_mut() {
        for (name, binary) in &compiled.deploy_binaries {
            let marker = format!("${{binary.{name}}}");
            let target = selected.get(name).unwrap_or(&binary.target);
            let path = crate::platform::simplify_path(&root.join(target));
            let value = path.to_string_lossy();
            task.command = task.command.replace(&marker, &value);
            for argument in &mut task.args {
                *argument = argument.replace(&marker, &value);
            }
            for env_value in task.env.values_mut() {
                *env_value = env_value.replace(&marker, &value);
            }
            if let Some(cwd) = task.cwd.as_mut() {
                *cwd = PathBuf::from(cwd.to_string_lossy().replace(&marker, &value));
            }
            if let Some(healthcheck) = task.healthcheck.as_mut()
                && let HealthCheckProbe::Exec { command, args, cwd } = &mut healthcheck.probe
            {
                *command = command.replace(&marker, &value);
                for argument in args {
                    *argument = argument.replace(&marker, &value);
                }
                if let Some(cwd) = cwd {
                    *cwd = PathBuf::from(cwd.to_string_lossy().replace(&marker, &value));
                }
            }
        }
    }
}

/// 选择单个逻辑二进制并报告支持矩阵。
fn select_one(
    name: &str,
    spec: &DeployBinarySpec,
    platform: &DeployPlatform,
) -> Result<SelectedDeployBinary, String> {
    let best = spec
        .variants
        .iter()
        .filter(|variant| variant.selector.matches(platform))
        .max_by_key(|variant| variant.selector.specificity());
    let Some(best) = best else {
        let supported = spec
            .variants
            .iter()
            .map(|variant| variant.selector.key())
            .collect::<Vec<_>>()
            .join("、");
        return Err(format!(
            "二进制 `{name}` 没有适用于远端平台 `{}` 的变体；已声明：{supported}",
            platform.key()
        ));
    };
    Ok(SelectedDeployBinary {
        name: name.to_owned(),
        source: best.source.clone(),
        target: best.target.clone().unwrap_or_else(|| spec.target.clone()),
        selector: best.selector.key(),
    })
}
