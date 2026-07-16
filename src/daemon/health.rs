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
    core::{HealthCheckSpec, TaskId, TaskSpec},
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
}

impl ActiveCheck {
    /// 请求回收检查进程树并等待短轮询线程退出。
    fn cancel(self) {
        self.cancelled.store(true, Ordering::Release);
        if self.thread.is_some_and(|thread| thread.join().is_err()) {
            tracing::warn!("健康检查线程异常退出");
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
        let mut events = Vec::new();
        let now = Instant::now();
        let mut active_count = self
            .entries
            .values()
            .filter(|entry| entry.active.is_some())
            .count();
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
        {
            active.cancel();
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
    }
}

/// 单个 Service 同时执行的健康检查上限。
const MAX_ACTIVE_CHECKS: usize = 32;

/// 执行一次检查，并在取消或超时时回收整个检查进程树。
fn run_check(check: &HealthCheckSpec, task: &TaskSpec, cancelled: &AtomicBool) -> ProbeResult {
    let mut child = match spawn_health_check(check, task) {
        Ok(child) => child,
        Err(error) => {
            return ProbeResult {
                success: false,
                detail: format!("检查程序创建失败：{error}"),
            };
        }
    };
    let deadline = Instant::now() + Duration::from_millis(check.timeout_ms);
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
                    detail: format!("检查超过 {} 毫秒", check.timeout_ms),
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
