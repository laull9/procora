use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    core::{HealthCheckProbe, HealthCheckSpec, HttpHealthCheckSpec, TaskId, TaskSpec},
    engine::{HealthState, RuntimeEvent, TaskRunIdentity},
    process::spawn_health_check,
};

/// 单次健康检查线程可观察的最终结果槽。
#[derive(Debug)]
struct ProbeResult {
    success: bool,
    detail: String,
}

/// 一个正在执行且可取消的健康检查。
#[derive(Debug)]
struct ActiveCheck {
    cancelled: Arc<AtomicBool>,
    result: Arc<Mutex<Option<ProbeResult>>>,
    thread: Option<JoinHandle<()>>,
    join_on_cancel: bool,
}

impl ActiveCheck {
    /// 请求取消检查；exec 等待整树回收，HTTP 请求交给有界超时线程退出。
    fn cancel(mut self) -> Option<Self> {
        self.cancelled.store(true, Ordering::Release);
        if self.join_on_cancel
            || self
                .thread
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished)
        {
            if self
                .thread
                .take()
                .is_some_and(|thread| thread.join().is_err())
            {
                tracing::warn!("健康检查线程异常退出");
            }
            None
        } else {
            Some(self)
        }
    }

    /// 回收已经自然结束的取消线程，仍在请求中的 HTTP 检查继续保留。
    fn reap(mut self) -> Option<Self> {
        if self
            .thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if self
                .thread
                .take()
                .is_some_and(|thread| thread.join().is_err())
            {
                tracing::warn!("健康检查线程异常退出");
            }
            None
        } else {
            Some(self)
        }
    }
}

/// 当前 Task run 的健康检查调度状态。
#[derive(Debug)]
struct HealthEntry {
    identity: TaskRunIdentity,
    task: TaskSpec,
    check: HealthCheckSpec,
    next_due: Instant,
    consecutive_successes: u32,
    consecutive_failures: u32,
    published: HealthState,
    active: Option<ActiveCheck>,
}

/// 每个 `ServiceHost` 独占的有界健康检查运行时。
#[derive(Debug, Default)]
pub(super) struct HealthRuntime {
    entries: BTreeMap<TaskId, HealthEntry>,
    retired: Vec<ActiveCheck>,
}

impl HealthRuntime {
    /// 为新 Task run 安装检查计划；没有检查配置时保持空操作。
    pub(super) fn start(&mut self, task_id: &TaskId, identity: TaskRunIdentity, task: &TaskSpec) {
        self.remove(task_id);
        let Some(check) = task.healthcheck.clone() else {
            return;
        };
        self.entries.insert(
            task_id.clone(),
            HealthEntry {
                identity,
                task: task.clone(),
                next_due: Instant::now() + Duration::from_millis(check.initial_delay_ms),
                check,
                consecutive_successes: 0,
                consecutive_failures: 0,
                published: HealthState::Starting,
                active: None,
            },
        );
    }

    /// 仅停止身份匹配的健康检查，避免旧 run 取消新 run。
    pub(super) fn stop(&mut self, task_id: &TaskId, identity: TaskRunIdentity) {
        if self
            .entries
            .get(task_id)
            .is_some_and(|entry| entry.identity == identity)
        {
            self.remove(task_id);
        }
    }

    /// 收集完成结果、应用连续阈值并启动已到期且不重叠的新检查。
    pub(super) fn refresh(&mut self) -> Vec<RuntimeEvent> {
        self.retired = std::mem::take(&mut self.retired)
            .into_iter()
            .filter_map(ActiveCheck::reap)
            .collect();
        let mut events = Vec::new();
        let now = Instant::now();
        let mut active_count = self.retired.len().saturating_add(
            self.entries
                .values()
                .filter(|entry| entry.active.is_some())
                .count(),
        );
        for (task_id, entry) in &mut self.entries {
            let result = entry.active.as_ref().and_then(|active| {
                active
                    .result
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
            });
            if let Some(result) = result {
                let active = entry.active.take().expect("完成结果必然属于活动检查");
                active_count = active_count.saturating_sub(1);
                if active.thread.is_some_and(|thread| thread.join().is_err()) {
                    tracing::warn!(task = %task_id, "健康检查线程异常退出");
                }
                entry.next_due = now + Duration::from_millis(entry.check.period_ms);
                let health = apply_result(entry, result.success);
                if let Some(health) = health {
                    if health == HealthState::Unhealthy {
                        tracing::warn!(task = %task_id, detail = %result.detail, "健康检查达到失败阈值");
                    }
                    events.push(RuntimeEvent::HealthChanged {
                        task_id: task_id.clone(),
                        identity: entry.identity,
                        health,
                    });
                }
            }
            if entry.active.is_none() && entry.next_due <= now && active_count < MAX_ACTIVE_CHECKS {
                entry.active = Some(spawn_check(task_id, &entry.check, &entry.task));
                active_count += 1;
            }
        }
        events
    }

    /// 删除并取消一个 Task 的活动检查。
    fn remove(&mut self, task_id: &TaskId) {
        if let Some(mut entry) = self.entries.remove(task_id)
            && let Some(active) = entry.active.take()
            && let Some(retired) = active.cancel()
        {
            self.retired.push(retired);
        }
    }
}

impl Drop for HealthRuntime {
    fn drop(&mut self) {
        let task_ids = self.entries.keys().cloned().collect::<Vec<_>>();
        for task_id in task_ids {
            self.remove(&task_id);
        }
    }
}

/// 应用一次检查结果，只在跨过连续阈值且状态变化时返回事件状态。
fn apply_result(entry: &mut HealthEntry, success: bool) -> Option<HealthState> {
    let next = if success {
        entry.consecutive_successes = entry.consecutive_successes.saturating_add(1);
        entry.consecutive_failures = 0;
        (entry.consecutive_successes >= entry.check.success_threshold)
            .then_some(HealthState::Healthy)
    } else {
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        entry.consecutive_successes = 0;
        (entry.consecutive_failures >= entry.check.failure_threshold)
            .then_some(HealthState::Unhealthy)
    };
    next.filter(|health| *health != entry.published)
        .inspect(|health| entry.published = *health)
}

/// 启动一个结果槽容量固定为一的检查线程。
fn spawn_check(task_id: &TaskId, check: &HealthCheckSpec, task: &TaskSpec) -> ActiveCheck {
    let cancelled = Arc::new(AtomicBool::new(false));
    let result = Arc::new(Mutex::new(None));
    let thread_cancelled = Arc::clone(&cancelled);
    let thread_result = Arc::clone(&result);
    let check = check.clone();
    let task = task.clone();
    let join_on_cancel = matches!(check.probe, HealthCheckProbe::Exec { .. });
    let thread_name = format!("procora-health-{task_id}");
    let thread = thread::Builder::new().name(thread_name).spawn(move || {
        let outcome = run_check(&check, &task, &thread_cancelled);
        *thread_result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    });
    let thread = match thread {
        Ok(thread) => Some(thread),
        Err(error) => {
            *result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ProbeResult {
                success: false,
                detail: format!("检查线程创建失败：{error}"),
            });
            None
        }
    };
    ActiveCheck {
        cancelled,
        result,
        thread,
        join_on_cancel,
    }
}

/// 单个 Service 同时执行的健康检查上限。
const MAX_ACTIVE_CHECKS: usize = 32;

/// 执行一次检查，并在取消或超时时回收整个检查进程树。
fn run_check(check: &HealthCheckSpec, task: &TaskSpec, cancelled: &AtomicBool) -> ProbeResult {
    match &check.probe {
        HealthCheckProbe::Exec { command, args, cwd } => run_exec_check(
            command,
            args,
            cwd.as_deref(),
            check.timeout_ms,
            task,
            cancelled,
        ),
        HealthCheckProbe::HttpGet { http_get } => {
            run_http_check(http_get, check.timeout_ms, cancelled)
        }
    }
}

/// 执行一次 exec 检查，并在取消或超时时回收整个检查进程树。
fn run_exec_check(
    command: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    timeout_ms: u64,
    task: &TaskSpec,
    cancelled: &AtomicBool,
) -> ProbeResult {
    let mut child = match spawn_health_check(command, args, cwd, task) {
        Ok(child) => child,
        Err(error) => {
            return ProbeResult {
                success: false,
                detail: format!("检查程序创建失败：{error}"),
            };
        }
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if cancelled.load(Ordering::Acquire) {
            let _ = child.kill();
            return ProbeResult {
                success: false,
                detail: "检查已取消".to_owned(),
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return ProbeResult {
                    success: status.success(),
                    detail: status.code().map_or_else(
                        || "检查被信号终止".to_owned(),
                        |code| format!("退出码 {code}"),
                    ),
                };
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                return ProbeResult {
                    success: false,
                    detail: format!("检查超过 {timeout_ms} 毫秒"),
                };
            }
            Err(error) => {
                let _ = child.kill();
                return ProbeResult {
                    success: false,
                    detail: format!("检查状态读取失败：{error}"),
                };
            }
        }
    }
}

/// 执行一次有总超时、无重定向且精确匹配状态码的 HTTP GET 检查。
fn run_http_check(
    check: &HttpHealthCheckSpec,
    timeout_ms: u64,
    cancelled: &AtomicBool,
) -> ProbeResult {
    if cancelled.load(Ordering::Acquire) {
        return ProbeResult {
            success: false,
            detail: "检查已取消".to_owned(),
        };
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(timeout_ms))
        .redirects(0)
        .build();
    let mut request = agent.get(&http_probe_url(check));
    for (name, value) in &check.headers {
        request = request.set(name, value);
    }
    let outcome = request.call();
    if cancelled.load(Ordering::Acquire) {
        return ProbeResult {
            success: false,
            detail: "检查已取消".to_owned(),
        };
    }
    let status = match outcome {
        Ok(response) => response.status(),
        Err(ureq::Error::Status(status, _)) => status,
        Err(ureq::Error::Transport(error)) => {
            return ProbeResult {
                success: false,
                detail: format!("HTTP 请求失败：{error}"),
            };
        }
    };
    ProbeResult {
        success: status == check.status_code,
        detail: format!("HTTP 状态码 {status}，预期 {}", check.status_code),
    }
}

/// 从已验证字段构造不携带用户信息的 HTTP 探针 URL。
fn http_probe_url(check: &HttpHealthCheckSpec) -> String {
    let host = if check.host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{}]", check.host)
    } else {
        check.host.clone()
    };
    let authority = check
        .port
        .map_or_else(|| host.clone(), |port| format!("{host}:{port}"));
    format!("{}://{authority}{}", check.scheme.as_str(), check.path)
}
