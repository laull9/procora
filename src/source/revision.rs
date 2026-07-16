use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use sha2::{Digest, Sha256};

use crate::config::{CompiledProject, ConfigError, ConfigLoadCapture, load_path_capture};

use super::LocalFileSource;

/// 配置入口与 include 闭包内容的稳定 SHA-256 修订标识。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DefinitionRevision(String);

impl DefinitionRevision {
    /// 返回适合协议传输和人工复制的十六进制修订值。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DefinitionRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 一次完整读取产生的有效或无效候选修订。
#[derive(Debug)]
pub struct DefinitionCandidate {
    /// 本次读取内容的稳定修订值；文件不可读时为空。
    pub revision: Option<DefinitionRevision>,
    /// 完整编译结果或可诊断错误。
    pub compiled: Result<CompiledProject, ConfigError>,
    /// 当前读取发现的入口和 include 闭包路径，包括暂时缺失的目标。
    pub watched_paths: Vec<PathBuf>,
    /// include 安全边界和修订相对路径使用的服务根目录。
    pub root: PathBuf,
}

/// 带有界事件合并和静默窗口的本地配置监听器。
pub struct LocalFileMonitor {
    source: LocalFileSource,
    _watcher: RecommendedWatcher,
    events: Receiver<()>,
    debounce: Duration,
    deadline: Option<Instant>,
    last_observed: Option<String>,
    watched_paths: Arc<RwLock<BTreeSet<PathBuf>>>,
}

impl fmt::Debug for LocalFileMonitor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalFileMonitor")
            .field("source", &self.source)
            .field("debounce", &self.debounce)
            .field("deadline", &self.deadline)
            .field("last_observed", &self.last_observed)
            .field("watched_paths", &self.watched_paths)
            .finish_non_exhaustive()
    }
}

impl LocalFileMonitor {
    /// 创建监听器，并把当前磁盘状态作为已提交基线。
    pub(super) fn new(source: LocalFileSource, debounce: Duration) -> notify::Result<Self> {
        let (sender, events) = sync_channel(1);
        let candidate = source.read_candidate();
        let watched_root = candidate.root.clone();
        let watched_paths = Arc::new(RwLock::new(
            candidate.watched_paths.iter().cloned().collect(),
        ));
        let callback_paths = Arc::clone(&watched_paths);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if event
                .as_ref()
                .map_or(true, |event| event_targets(event, &callback_paths))
            {
                coalesce_event(&sender);
            }
        })?;
        watcher.watch(&watched_root, RecursiveMode::Recursive)?;
        let last_observed = observation(&candidate);
        Ok(Self {
            source,
            _watcher: watcher,
            events,
            debounce,
            deadline: None,
            last_observed: Some(last_observed),
            watched_paths,
        })
    }

    /// 消费文件事件，并在静默窗口结束后最多返回一个完整候选。
    pub fn poll(&mut self) -> Option<DefinitionCandidate> {
        let now = Instant::now();
        while let Ok(()) = self.events.try_recv() {
            self.deadline = Some(now + self.debounce);
        }
        if self.deadline.is_none_or(|deadline| now < deadline) {
            return None;
        }
        self.deadline = None;
        let candidate = self.source.read_candidate();
        if let Ok(mut watched_paths) = self.watched_paths.write() {
            *watched_paths = candidate.watched_paths.iter().cloned().collect();
        }
        let observed = observation(&candidate);
        if self.last_observed.as_deref() == Some(&observed) {
            return None;
        }
        self.last_observed = Some(observed);
        Some(candidate)
    }
}

impl LocalFileSource {
    /// 原子读取入口闭包、计算内容修订并完整编译。
    pub fn read_candidate(&self) -> DefinitionCandidate {
        let capture = load_path_capture(self.path());
        let revision = closure_revision(&capture);
        DefinitionCandidate {
            revision,
            compiled: capture.result,
            watched_paths: capture.watched_paths,
            root: capture.root,
        }
    }

    /// 监听配置入口所在目录，以兼容编辑器的原子替换写入。
    ///
    /// # Errors
    ///
    /// 当平台监听器无法创建或服务根目录无法递归监听时返回错误。
    pub fn monitor(&self, debounce: Duration) -> notify::Result<LocalFileMonitor> {
        LocalFileMonitor::new(self.clone(), debounce)
    }
}

/// 把原始文件事件压入容量为一的通道，满载时直接合并。
fn coalesce_event(sender: &SyncSender<()>) {
    let _ = sender.try_send(());
}

/// 判断目录级事件是否涉及目标入口。
fn event_targets(event: &Event, targets: &RwLock<BTreeSet<PathBuf>>) -> bool {
    if event.paths.is_empty() {
        return true;
    }
    let Ok(targets) = targets.read() else {
        return true;
    };
    event
        .paths
        .iter()
        .any(|path| targets.contains(&absolute_path(path)))
}

/// 返回不会要求目标文件已经存在的绝对路径。
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// 为有效、无效和暂时缺失状态生成去重键。
fn observation(candidate: &DefinitionCandidate) -> String {
    candidate.revision.as_ref().map_or_else(
        || format!("missing:{}", candidate.compiled.as_ref().unwrap_err()),
        |revision| format!("revision:{revision}"),
    )
}

/// 按相对路径和字节计算整个已读取闭包的稳定 SHA-256。
fn closure_revision(capture: &ConfigLoadCapture) -> Option<DefinitionRevision> {
    if capture.inputs.is_empty() {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(b"procora-config-closure-v1\0");
    for input in &capture.inputs {
        let relative = input
            .path
            .strip_prefix(&capture.root)
            .unwrap_or(&input.path);
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update((input.bytes.len() as u64).to_le_bytes());
        digest.update(&input.bytes);
        digest.update([0]);
    }
    if let Err(error) = &capture.result {
        digest.update(b"invalid\0");
        digest.update(error.to_string().as_bytes());
    }
    let digest = digest.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("写入 String 不会失败");
    }
    Some(DefinitionRevision(output))
}
