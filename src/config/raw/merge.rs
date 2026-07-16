use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use super::RawProject;

impl RawProject {
    /// 创建不携带任何声明的合并起点。
    pub(crate) fn empty() -> Self {
        Self {
            include: Vec::new(),
            version: None,
            project: None,
            dependencies: BTreeMap::default(),
            tasks: BTreeMap::default(),
        }
    }

    /// 取走当前文档声明的 include 列表。
    pub(crate) fn take_includes(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.include)
    }

    /// 判断当前内存文档是否声明了需要路径上下文的 include。
    pub(crate) fn has_includes(&self) -> bool {
        !self.include.is_empty()
    }

    /// 返回文档显式声明的模式版本。
    pub(crate) const fn declared_version(&self) -> Option<u32> {
        self.version
    }

    /// 返回文档显式声明的项目名称。
    pub(crate) fn declared_project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// 把当前文档的相对运行路径改写为相对声明文件的稳定路径。
    pub(crate) fn rebase(&mut self, base: &Path) {
        for dependency in self.dependencies.values_mut() {
            dependency.source = dependency
                .source
                .take()
                .map(|source| rebase_source(&source, base));
        }
        for task in self.tasks.values_mut() {
            task.cwd = task.cwd.take().map(|path| rebase_path(&path, base));
            if let Some(healthcheck) = task.healthcheck.as_mut() {
                healthcheck.rebase(base);
            }
        }
    }

    /// 以完整 map 条目覆盖方式合并一个更高优先级文档。
    pub(crate) fn overlay(&mut self, higher: Self) {
        if higher.version.is_some() {
            self.version = higher.version;
        }
        if higher.project.is_some() {
            self.project = higher.project;
        }
        self.dependencies.extend(higher.dependencies);
        self.tasks.extend(higher.tasks);
    }
}

/// 把相对本地依赖来源改写为声明文件目录下的绝对路径。
fn rebase_source(source: &str, base: &Path) -> String {
    if let Some(path) = source.strip_prefix("file://") {
        let path = Path::new(path);
        return if path.is_absolute() {
            source.to_owned()
        } else {
            format!("file://{}", rebase_path(path, base).display())
        };
    }
    let path = Path::new(source);
    if path.is_absolute() || source.contains("://") || is_scp_source(source) {
        source.to_owned()
    } else {
        rebase_path(path, base).to_string_lossy().into_owned()
    }
}

/// 判断来源是否为 `user@host:/path` 形式的 SCP 地址。
fn is_scp_source(source: &str) -> bool {
    source
        .split_once(':')
        .is_some_and(|(host, path)| !host.contains('/') && path.starts_with('/'))
}

/// 组合并按词法消除点和父目录分量，不要求目标已经存在。
fn rebase_path(path: &Path, base: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
