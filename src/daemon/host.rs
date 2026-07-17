use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::{Duration, Instant},
};

use crate::config::{CompiledProject, ManagedDependencies};
use crate::core::{ProjectSpec, TaskId};
use crate::engine::{Engine, EngineCommand, EngineEffect, RuntimeEvent, TaskRunIdentity};
use crate::log::{FileLogStore, LogStream, TailBuffer};
use crate::monitor::SystemMonitor;
use crate::process::{ManagedChild, spawn_task};
use crate::protocol::{LogBatchDto, LogCursorDto};
use thiserror::Error;

use super::{
    health::HealthRuntime,
    host_logs::{LogReaderContext, LogWriter, OutputReader, join_readers, spawn_log_reader},
    resources::ResourceCache,
};

mod reconfigure;
mod snapshot;

/// 单个 `ServiceHost` 的真实 Task 运行错误。
#[derive(Debug, Error)]
pub enum ServiceHostError {
    /// Task 进程无法创建。
    #[error("Task {task_id} 启动失败: {source}")]
    Spawn {
        /// 启动失败的 Task。
        task_id: TaskId,
        /// 平台进程创建错误。
        source: std::io::Error,
    },
    /// Task 进程无法停止或查询。
    #[error("Task {task_id} 进程操作失败: {source}")]
    Process {
        /// 操作失败的 Task。
        task_id: TaskId,
        /// 平台进程操作错误。
        source: std::io::Error,
    },
    /// 候选应用失败且旧配置恢复也未完成。
    #[error("候选配置应用失败：{apply}；旧配置恢复失败：{rollback}")]
    ReconfigureRollback {
        /// 候选配置的原始失败。
        apply: String,
        /// 恢复旧配置时的失败。
        rollback: String,
    },
}

/// 当前正在运行的单个 Task 进程及输出读取线程。
#[derive(Debug)]
struct TaskInstance {
    child: ManagedChild,
    identity: TaskRunIdentity,
    pid: u32,
    started_at: Instant,
    readers: Vec<OutputReader>,
}

/// 等待自动重启退避到期的创建意图。
#[derive(Clone, Debug)]
struct PendingSpawn {
    task_id: TaskId,
    identity: TaskRunIdentity,
    due: Instant,
}

/// 组合引擎、真实进程、日志和资源监测的单服务宿主。
#[derive(Debug)]
pub struct ServiceHost {
    spec: ProjectSpec,
    dependencies: ManagedDependencies,
    engine: Engine,
    monitor: SystemMonitor,
    resource_cache: ResourceCache,
    logs: Arc<Mutex<TailBuffer>>,
    file_logs: Option<Arc<FileLogStore>>,
    log_writer: Option<LogWriter>,
    dropped_log_chunks: Arc<AtomicU64>,
    instances: BTreeMap<TaskId, TaskInstance>,
    pending_spawns: Vec<PendingSpawn>,
    health: HealthRuntime,
}

impl ServiceHost {
    /// 根据已验证的项目配置建立服务宿主。
    pub fn from_compiled(compiled: CompiledProject) -> Self {
        Self::build(compiled, None)
    }

    /// 根据已验证配置建立把日志保存在所属服务目录的宿主。
    pub fn from_compiled_at(compiled: CompiledProject, service_root: &std::path::Path) -> Self {
        let file_logs = Arc::new(FileLogStore::for_service(service_root));
        Self::build(compiled, Some(file_logs))
    }

    /// 组合共享组件和可选的服务目录文件日志存储。
    fn build(compiled: CompiledProject, file_logs: Option<Arc<FileLogStore>>) -> Self {
        let engine = Engine::new(&compiled.spec, compiled.graph);
        let log_writer = file_logs
            .as_ref()
            .map(|files| LogWriter::new(Arc::clone(files)));
        Self {
            spec: compiled.spec,
            dependencies: compiled.dependencies,
            engine,
            monitor: SystemMonitor::new(),
            resource_cache: ResourceCache::default(),
            logs: Arc::new(Mutex::new(TailBuffer::new(4096))),
            file_logs,
            log_writer,
            dropped_log_chunks: Arc::new(AtomicU64::new(0)),
            instances: BTreeMap::new(),
            pending_spawns: Vec::new(),
            health: HealthRuntime::default(),
        }
    }

    /// 返回当前项目标识。
    pub fn project(&self) -> &str {
        self.engine.project()
    }

    /// 返回当前宿主最后一次成功提交的规范化项目配置。
    pub fn spec(&self) -> &ProjectSpec {
        &self.spec
    }

    /// 返回当前有效修订采用的项目级管理依赖声明。
    pub fn dependencies(&self) -> &ManagedDependencies {
        &self.dependencies
    }

    /// 返回确定性的首轮启动计划。
    pub fn start_plan(&self) -> Vec<String> {
        self.engine
            .initial_start_plan()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// 返回已经装配的监测和日志组件数量。
    pub const fn adapter_count(&self) -> usize {
        2
    }

    /// 启动全部 Task，并按依赖条件逐步执行创建意图。
    ///
    /// # Errors
    ///
    /// 当任一已就绪 Task 无法创建且没有自动重试策略时返回错误。
    pub fn start(&mut self) -> Result<(), ServiceHostError> {
        self.pending_spawns.clear();
        let effects = self.engine.command(EngineCommand::StartAll);
        self.execute_effects(effects)
    }

    /// 按反向依赖顺序停止全部 Task，并排空输出线程。
    ///
    /// # Errors
    ///
    /// 当任一进程无法停止或等待时返回错误。
    pub fn stop(&mut self) -> Result<(), ServiceHostError> {
        self.pending_spawns.clear();
        let effects = self.engine.command(EngineCommand::StopAll);
        let mut first_error = None;
        for effect in effects {
            if let Err(error) = self.execute_effects(vec![effect])
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        self.flush_logs();
        first_error.map_or(Ok(()), Err)
    }

    /// 轮询退出事件与重启退避，返回 Task 状态是否发生变化。
    ///
    /// # Panics
    ///
    /// 仅当内部 Task 索引在一次同步轮询中违反一致性不变量时 panic。
    pub fn refresh(&mut self) -> bool {
        let mut changed = self.start_due_spawns();
        let task_ids = self.instances.keys().cloned().collect::<Vec<_>>();
        for task_id in task_ids {
            let result = self
                .instances
                .get_mut(&task_id)
                .expect("Task 实例仍存在")
                .child
                .try_wait();
            match result {
                Ok(Some(status)) => {
                    let mut instance = self.instances.remove(&task_id).expect("实例仍存在");
                    self.resource_cache.invalidate();
                    self.health.stop(&task_id, instance.identity);
                    if let Err(error) = instance.child.cleanup_after_exit() {
                        tracing::warn!(task = %task_id, %error, "顶层进程退出后清理剩余进程树失败");
                    }
                    join_readers(instance.readers);
                    let effects = self.engine.event(RuntimeEvent::Exited {
                        task_id: task_id.clone(),
                        identity: instance.identity,
                        exit_code: status.code(),
                        success: exit_succeeded(&self.spec.tasks[&task_id], status),
                        run_duration_ms: elapsed_millis(instance.started_at),
                    });
                    if let Err(error) = self.execute_effects(effects) {
                        tracing::warn!(%error, "Task 退出后的调度失败");
                    }
                    changed = true;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(task = %task_id, %error, "Task 退出状态查询失败");
                }
            }
        }
        let health_events = self.health.refresh();
        changed |= !health_events.is_empty();
        for event in health_events {
            let effects = self.engine.event(event);
            if let Err(error) = self.execute_effects(effects) {
                tracing::warn!(%error, "健康状态变化后的调度失败");
            }
        }
        if changed {
            self.flush_logs();
        }
        changed
    }

    /// 向所属服务目录中的服务级日志追加内容。
    ///
    /// # Errors
    ///
    /// 当持久日志已配置但文件写入或压缩失败时返回错误。
    pub fn append_service_log(&self, bytes: &[u8]) -> Result<(), crate::log::FileLogError> {
        self.file_logs
            .as_ref()
            .map_or(Ok(()), |logs| logs.append_service(bytes))
    }

    /// 从 Service 本地文件续读指定 Task 日志。
    ///
    /// # Errors
    ///
    /// 当嵌入模式没有文件存储，或文件与索引无法读取时返回错误。
    pub fn read_task_log(
        &self,
        task_id: &TaskId,
        cursor: Option<LogCursorDto>,
        max_bytes: usize,
    ) -> Result<LogBatchDto, crate::log::FileLogError> {
        let files = self.file_logs.as_ref().ok_or_else(|| {
            crate::log::FileLogError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "嵌入模式没有持久文件日志",
            ))
        })?;
        let batch = files.read_task(
            task_id,
            cursor.map(|cursor| crate::log::FileLogCursor {
                generation: cursor.generation,
                offset: cursor.offset,
            }),
            max_bytes,
        )?;
        Ok(LogBatchDto {
            task_id: task_id.clone(),
            bytes: batch.bytes,
            next_cursor: LogCursorDto {
                generation: batch.next_cursor.generation,
                offset: batch.next_cursor.offset,
            },
            gap: batch.gap,
        })
    }

    /// 执行一组引擎副作用并把结果事件送回同一写者。
    fn execute_effects(&mut self, effects: Vec<EngineEffect>) -> Result<(), ServiceHostError> {
        let mut effects = VecDeque::from(effects);
        while let Some(effect) = effects.pop_front() {
            match effect {
                EngineEffect::Spawn {
                    task_id,
                    identity,
                    delay_ms,
                } if delay_ms > 0 => self.pending_spawns.push(PendingSpawn {
                    task_id,
                    identity,
                    due: Instant::now() + Duration::from_millis(delay_ms),
                }),
                EngineEffect::Spawn {
                    task_id, identity, ..
                } => match self.spawn(&task_id, identity) {
                    Ok(()) => effects.extend(
                        self.engine
                            .event(RuntimeEvent::Spawned { task_id, identity }),
                    ),
                    Err(error) => {
                        let retry = self.engine.event(RuntimeEvent::SpawnFailed {
                            task_id: task_id.clone(),
                            identity,
                        });
                        if retry.is_empty() {
                            return Err(ServiceHostError::Spawn {
                                task_id,
                                source: error,
                            });
                        }
                        tracing::warn!(task = %task_id, %error, "Task 创建失败，已进入自动重试");
                        effects.extend(retry);
                    }
                },
                EngineEffect::Stop { task_id, identity } => {
                    effects.extend(self.stop_instance(&task_id, identity)?);
                }
            }
        }
        Ok(())
    }

    /// 创建一个 Task 进程并启动 stdout/stderr 排空线程。
    fn spawn(&mut self, task_id: &TaskId, identity: TaskRunIdentity) -> std::io::Result<()> {
        let mut child = spawn_task(&self.spec.tasks[task_id])?;
        let pid = child.id();
        let sequence = Arc::new(AtomicU64::new(0));
        let mut readers = Vec::new();
        if let Some(stdout) = child.take_stdout() {
            readers.push(spawn_log_reader(
                stdout,
                LogStream::Stdout,
                LogReaderContext {
                    task_id: task_id.clone(),
                    identity,
                    sequence: Arc::clone(&sequence),
                    tail: Arc::clone(&self.logs),
                    disk: self.log_writer.as_ref().map(LogWriter::sender),
                    dropped_chunks: Arc::clone(&self.dropped_log_chunks),
                },
            ));
        }
        if let Some(stderr) = child.take_stderr() {
            readers.push(spawn_log_reader(
                stderr,
                LogStream::Stderr,
                LogReaderContext {
                    task_id: task_id.clone(),
                    identity,
                    sequence,
                    tail: Arc::clone(&self.logs),
                    disk: self.log_writer.as_ref().map(LogWriter::sender),
                    dropped_chunks: Arc::clone(&self.dropped_log_chunks),
                },
            ));
        }
        self.resource_cache.invalidate();
        self.instances.insert(
            task_id.clone(),
            TaskInstance {
                child,
                identity,
                pid,
                started_at: Instant::now(),
                readers,
            },
        );
        self.health
            .start(task_id, identity, &self.spec.tasks[task_id]);
        Ok(())
    }

    /// 停止匹配身份的实例，并把最终退出事件送回引擎。
    fn stop_instance(
        &mut self,
        task_id: &TaskId,
        identity: TaskRunIdentity,
    ) -> Result<Vec<EngineEffect>, ServiceHostError> {
        self.pending_spawns
            .retain(|pending| pending.identity != identity);
        let Some(mut instance) = self.instances.remove(task_id) else {
            return Ok(self.engine.event(RuntimeEvent::Exited {
                task_id: task_id.clone(),
                identity,
                exit_code: None,
                success: false,
                run_duration_ms: 0,
            }));
        };
        if instance.identity != identity {
            self.instances.insert(task_id.clone(), instance);
            return Ok(Vec::new());
        }
        self.resource_cache.invalidate();
        self.health.stop(task_id, identity);
        let timeout = Duration::from_millis(self.spec.tasks[task_id].shutdown_timeout_ms);
        let outcome = instance
            .child
            .stop(timeout)
            .map_err(|source| ServiceHostError::Process {
                task_id: task_id.clone(),
                source,
            })?;
        join_readers(instance.readers);
        Ok(self.engine.event(RuntimeEvent::Exited {
            task_id: task_id.clone(),
            identity,
            exit_code: outcome.status.code(),
            success: exit_succeeded(&self.spec.tasks[task_id], outcome.status),
            run_duration_ms: elapsed_millis(instance.started_at),
        }))
    }

    /// 启动所有已经达到自动重启截止时间的意图。
    fn start_due_spawns(&mut self) -> bool {
        let now = Instant::now();
        let mut due = Vec::new();
        self.pending_spawns.retain(|pending| {
            if pending.due <= now {
                due.push(pending.clone());
                false
            } else {
                true
            }
        });
        let changed = !due.is_empty();
        for pending in due {
            let effect = EngineEffect::Spawn {
                task_id: pending.task_id,
                identity: pending.identity,
                delay_ms: 0,
            };
            if let Err(error) = self.execute_effects(vec![effect]) {
                tracing::warn!(%error, "Task 自动重启失败");
            }
        }
        changed
    }

    /// 在限定时间内刷新此前成功进入有界队列的文件日志。
    fn flush_logs(&self) {
        if let Some(writer) = &self.log_writer {
            writer.flush();
        }
    }
}

/// 把单调时钟运行时长有界转换为协议使用的毫秒数。
fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 按 Task 声明判断平台退出状态是否属于成功结束。
fn exit_succeeded(task: &crate::core::TaskSpec, status: std::process::ExitStatus) -> bool {
    status.success()
        || status
            .code()
            .is_some_and(|code| task.success_exit_codes.contains(&code))
}

impl Drop for ServiceHost {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::warn!(%error, "ServiceHost 退出时停止 Task 失败");
        }
        if let Some(writer) = &mut self.log_writer {
            writer.shutdown();
        }
    }
}
