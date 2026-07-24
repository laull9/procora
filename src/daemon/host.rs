use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::{Duration, Instant},
};

use crate::config::{CompiledProject, ManagedDependencies};
use crate::core::{ProjectSpec, TaskId};
use crate::engine::{Engine, EngineCommand, EngineEffect, RuntimeEvent, TaskRunIdentity};
use crate::log::{FileLogStore, LogStream, TailBuffer};
use crate::monitor::SystemMonitor;
use crate::process::{ManagedChild, spawn_task};
use crate::protocol::TaskDiagnosticKindDto;
use thiserror::Error;

use super::{
    health::HealthRuntime,
    host_logs::{LogReaderContext, LogWriter, OutputReader, join_readers, spawn_log_reader},
    resources::ResourceCache,
};

pub(super) mod diagnostics;
mod log_access;
mod reconfigure;
mod refresh;
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
    default_working_directory: Option<PathBuf>,
    engine: Engine,
    monitor: SystemMonitor,
    resource_cache: ResourceCache,
    logs: Arc<Mutex<TailBuffer>>,
    diagnostics: Arc<Mutex<diagnostics::TaskDiagnostics>>,
    diagnostic_sequence: u64,
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
        Self::build(compiled, None, None)
    }

    /// 根据已验证配置建立以服务目录保存日志并作为默认工作路径的宿主。
    pub fn from_compiled_at(compiled: CompiledProject, service_root: &std::path::Path) -> Self {
        let file_logs = Arc::new(FileLogStore::for_service(service_root));
        Self::build(compiled, Some(file_logs), Some(service_root.to_path_buf()))
    }

    /// 组合共享组件和可选的服务目录文件日志存储。
    fn build(
        compiled: CompiledProject,
        file_logs: Option<Arc<FileLogStore>>,
        default_working_directory: Option<PathBuf>,
    ) -> Self {
        let engine = Engine::new(&compiled.spec, compiled.graph);
        let log_writer = file_logs
            .as_ref()
            .map(|files| LogWriter::new(Arc::clone(files)));
        Self {
            spec: compiled.spec,
            dependencies: compiled.dependencies,
            default_working_directory,
            engine,
            monitor: SystemMonitor::new(),
            resource_cache: ResourceCache::default(),
            logs: Arc::new(Mutex::new(TailBuffer::new(4096))),
            diagnostics: Arc::new(Mutex::new(diagnostics::TaskDiagnostics::default())),
            diagnostic_sequence: 0,
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
        self.diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
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
                        let task = self.runtime_task(&task_id);
                        let (message, suggestion) = diagnostics::spawn_error(&task, &error);
                        self.record_task_diagnostic(
                            &task_id,
                            Some(identity),
                            TaskDiagnosticKindDto::Spawn,
                            message,
                            suggestion,
                        );
                        let retry = self.engine.event(RuntimeEvent::SpawnFailed {
                            task_id: task_id.clone(),
                            identity,
                        });
                        if self
                            .engine
                            .state(&task_id)
                            .is_some_and(|state| state.restart_exhausted)
                        {
                            self.record_task_diagnostic(
                                &task_id,
                                Some(identity),
                                TaskDiagnosticKindDto::Restart,
                                format!("已达到 {} 次自动重启上限", task.max_restarts),
                                Some("修复启动错误后手动重启，或检查 max_restarts 配置".to_owned()),
                            );
                        }
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
        let task = self.runtime_task(task_id);
        let mut child = spawn_task(&task)?;
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
                    diagnostics: Arc::clone(&self.diagnostics),
                    files: self.file_logs.as_ref().map(Arc::clone),
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
                    diagnostics: Arc::clone(&self.diagnostics),
                    files: self.file_logs.as_ref().map(Arc::clone),
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
        self.health.start(task_id, identity, &task);
        Ok(())
    }

    /// 返回应用服务目录默认工作路径后的单次运行 Task。
    fn runtime_task(&self, task_id: &TaskId) -> crate::core::TaskSpec {
        let mut task = self.spec.tasks[task_id].clone();
        if task.cwd.is_none() {
            task.cwd.clone_from(&self.default_working_directory);
        }
        task
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
        let outcome = match instance.child.stop(timeout) {
            Ok(outcome) => outcome,
            Err(source) => {
                let (message, suggestion) =
                    diagnostics::process_io_error("Task 进程停止失败", &source);
                self.record_task_diagnostic(
                    task_id,
                    Some(identity),
                    TaskDiagnosticKindDto::Process,
                    message,
                    suggestion,
                );
                return Err(ServiceHostError::Process {
                    task_id: task_id.clone(),
                    source,
                });
            }
        };
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
