use std::{collections::BTreeSet, time::Duration};

use crate::{
    config::{DiscoveredProject, ProjectDiff, diff_projects},
    protocol::{
        CenterEventKindDto, ConfigCandidateDto, ServiceActionDto, ServiceSelectorDto,
        ServiceStatusDto, ServiceViewDto,
    },
    source::{DefinitionCandidate, LocalFileSource},
};

use super::{Center, CenterError};
use crate::daemon::{
    ServiceHost,
    managed::{ActiveDefinition, PendingConfig},
};

/// 文件事件合并后等待编辑器写入稳定的静默窗口。
const CONFIG_DEBOUNCE: Duration = Duration::from_millis(250);

impl Center {
    /// 启动、重启或停止指定服务，同时保证候选失败不破坏旧宿主。
    pub(super) fn manage(
        &mut self,
        action: ServiceActionDto,
        selector: &ServiceSelectorDto,
    ) -> Result<ServiceViewDto, CenterError> {
        let name = self.resolve_name(selector)?;
        if action == ServiceActionDto::Stop {
            return self.stop_service(&name);
        }
        let candidate = self.preview_name(&name)?;
        if !candidate.valid {
            return Err(CenterError::InvalidCandidate {
                name,
                message: candidate
                    .message
                    .unwrap_or_else(|| "未知配置错误".to_owned()),
            });
        }
        let revision = candidate
            .revision
            .filter(|_| candidate.valid)
            .ok_or_else(|| CenterError::CandidateUnavailable(name.clone()))?;
        self.ensure_pending_revision(&name, &revision)?;
        self.commit_pending(&name, true)
    }

    /// 重新读取配置并返回不会产生运行时副作用的候选预览。
    pub(super) fn preview_config(
        &mut self,
        selector: &ServiceSelectorDto,
    ) -> Result<ConfigCandidateDto, CenterError> {
        let name = self.resolve_name(selector)?;
        self.preview_name(&name)
    }

    /// 重新核对磁盘修订，并显式提交用户已经确认的候选。
    pub(super) fn apply_config(
        &mut self,
        selector: &ServiceSelectorDto,
        requested_revision: &str,
    ) -> Result<ServiceViewDto, CenterError> {
        let name = self.resolve_name(selector)?;
        let candidate = self.preview_name(&name)?;
        let actual = candidate
            .revision
            .clone()
            .unwrap_or_else(|| "<unreadable>".to_owned());
        if actual != requested_revision {
            return Err(CenterError::RevisionMismatch {
                requested: requested_revision.to_owned(),
                actual,
            });
        }
        if !candidate.valid {
            return Err(CenterError::InvalidCandidate {
                name,
                message: candidate
                    .message
                    .unwrap_or_else(|| "未知配置错误".to_owned()),
            });
        }
        self.ensure_pending_revision(&name, requested_revision)?;
        self.commit_pending(&name, false)
    }

    /// 为恢复和新注册服务安装目录级监听器。
    pub(super) fn install_all_monitors(&mut self) {
        let names = self.services.keys().cloned().collect::<Vec<_>>();
        for name in names {
            self.install_monitor(&name);
        }
    }

    /// 安装单服务监听器；平台监听失败不会阻止显式预览和生命周期操作。
    pub(super) fn install_monitor(&mut self, name: &str) {
        let source = LocalFileSource::new(self.services[name].config_path.clone());
        match source.monitor(CONFIG_DEBOUNCE) {
            Ok(monitor) => {
                self.monitors.insert(name.to_owned(), monitor);
            }
            Err(error) => {
                tracing::warn!(service = name, %error, "配置文件监听器安装失败");
            }
        }
    }

    /// 轮询全部防抖监听器，把新磁盘状态暂存为候选而不自动应用。
    pub(super) fn poll_config_monitors(&mut self) {
        let names = self.monitors.keys().cloned().collect::<Vec<_>>();
        for name in names {
            let candidate = self
                .monitors
                .get_mut(&name)
                .and_then(crate::source::LocalFileMonitor::poll);
            if let Some(candidate) = candidate {
                let changed = self.stage_candidate(&name, candidate);
                if changed {
                    let view = self.services[&name].view();
                    self.push_event(CenterEventKindDto::ConfigCandidateChanged, Some(view));
                }
            }
        }
    }

    /// 直接读取指定服务入口并暂存本次候选。
    fn preview_name(&mut self, name: &str) -> Result<ConfigCandidateDto, CenterError> {
        let source = LocalFileSource::new(self.services[name].config_path.clone());
        self.stage_candidate(name, source.read_candidate());
        self.services[name]
            .candidate_view
            .clone()
            .ok_or_else(|| CenterError::CandidateUnavailable(name.to_owned()))
    }

    /// 把完整读取结果转换为可展示差异，并隔离无效候选。
    fn stage_candidate(&mut self, name: &str, candidate: DefinitionCandidate) -> bool {
        let revision = candidate
            .revision
            .as_ref()
            .map(|revision| revision.as_str().to_owned());
        let (view, pending) = match candidate.compiled {
            Ok(compiled) if compiled.spec.project != name => (
                ConfigCandidateDto {
                    revision,
                    valid: false,
                    diff: None,
                    message: Some(format!(
                        "配置中的服务名称已从 {name} 变为 {}，请显式迁移",
                        compiled.spec.project
                    )),
                },
                None,
            ),
            Ok(compiled) => {
                let mut diff = self.diff_candidate(name, &compiled);
                let upload_targets_changed = self.services[name]
                    .active_definition
                    .as_ref()
                    .is_none_or(|active| active.upload_targets != compiled.upload_targets);
                let message = if diff.is_empty() && upload_targets_changed {
                    Some("上传目标声明变化，不影响运行中 Task".to_owned())
                } else {
                    diff.is_empty().then(|| "配置语义没有变化".to_owned())
                };
                let revision = revision.expect("可读配置应当具有修订值");
                let pending = PendingConfig {
                    revision: revision.clone(),
                    compiled,
                    diff: diff.clone(),
                    upload_targets_changed,
                };
                (
                    ConfigCandidateDto {
                        revision: Some(revision),
                        valid: true,
                        diff: Some(std::mem::take(&mut diff)),
                        message,
                    },
                    Some(pending),
                )
            }
            Err(error) => (
                ConfigCandidateDto {
                    revision,
                    valid: false,
                    diff: None,
                    message: Some(error.to_string()),
                },
                None,
            ),
        };
        let service = self.services.get_mut(name).expect("名称已经解析");
        let changed = service.candidate_view.as_ref() != Some(&view);
        service.candidate_view = Some(view);
        service.pending_config = pending;
        changed
    }

    /// 比较候选和当前有效宿主，并把依赖制品变化保守提升为全量重启。
    fn diff_candidate(
        &self,
        name: &str,
        candidate: &crate::config::CompiledProject,
    ) -> ProjectDiff {
        let Some(active) = self.services[name].active_definition.as_ref() else {
            return ProjectDiff {
                added: candidate.spec.tasks.keys().cloned().collect(),
                ..ProjectDiff::default()
            };
        };
        let mut diff = diff_projects(&active.spec, &candidate.spec);
        if active.dependencies != candidate.dependencies {
            let all = candidate
                .spec
                .tasks
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            diff.restart = all.into_iter().collect();
            diff.update_in_place.clear();
            diff.unchanged.clear();
        }
        diff
    }

    /// 确认内存候选仍对应用户请求的修订。
    fn ensure_pending_revision(&self, name: &str, revision: &str) -> Result<(), CenterError> {
        self.services[name]
            .pending_config
            .as_ref()
            .filter(|pending| pending.revision == revision)
            .map(|_| ())
            .ok_or_else(|| CenterError::CandidateUnavailable(name.to_owned()))
    }

    /// 提交已准备候选；任何准备失败都发生在停止旧宿主之前。
    fn commit_pending(
        &mut self,
        name: &str,
        force_restart: bool,
    ) -> Result<ServiceViewDto, CenterError> {
        let pending = self
            .services
            .get_mut(name)
            .expect("名称已经解析")
            .pending_config
            .take()
            .ok_or_else(|| CenterError::CandidateUnavailable(name.to_owned()))?;
        let active_definition = ActiveDefinition::from_compiled(&pending.compiled);
        if pending.diff.is_empty() && !pending.upload_targets_changed && !force_restart {
            self.services
                .get_mut(name)
                .expect("名称已经解析")
                .candidate_view = None;
            return Ok(self.services[name].view());
        }
        if !force_restart
            && pending.diff.added.is_empty()
            && pending.diff.removed.is_empty()
            && pending.diff.restart.is_empty()
        {
            let service = self.services.get_mut(name).expect("名称已经解析");
            let host = service
                .host
                .as_mut()
                .ok_or_else(|| CenterError::Unavailable(name.to_owned()))?;
            host.update_runtime_policies(pending.compiled)?;
            service.candidate_view = None;
            service.active_definition = Some(active_definition);
            self.persist_service(name)?;
            let view = self.services[name].view();
            self.push_event(CenterEventKindDto::StatusChanged, Some(view.clone()));
            return Ok(view);
        }
        let root = self.services[name].root.clone();
        let config_path = self.services[name].config_path.clone();
        let mut discovered = DiscoveredProject {
            root: root.clone(),
            config_path,
            compiled: pending.compiled,
        };
        if let Err(error) = super::super::project::prepare(&mut discovered) {
            return Err(error.into());
        }
        let service = self.services.get_mut(name).expect("名称已经解析");
        let should_run = force_restart || service.desired_running;
        let mut affected = pending
            .diff
            .added
            .iter()
            .chain(&pending.diff.removed)
            .chain(&pending.diff.restart)
            .cloned()
            .collect::<BTreeSet<_>>();
        if force_restart {
            affected.extend(discovered.compiled.spec.tasks.keys().cloned());
            if let Some(active) = &service.active_definition {
                affected.extend(active.spec.tasks.keys().cloned());
            }
        }
        if let Some(host) = service.host.as_mut() {
            if let Err(error) = host.reconfigure(discovered.compiled, &affected, should_run) {
                if matches!(
                    error,
                    crate::daemon::ServiceHostError::ReconfigureRollback { .. }
                ) {
                    service.status = ServiceStatusDto::Failed;
                    service.message = Some(error.to_string());
                    self.persist_service(name)?;
                    self.write_status_log(name);
                }
                return Err(CenterError::Unavailable(error.to_string()));
            }
        } else {
            let mut host = ServiceHost::from_compiled_at(discovered.compiled, &root);
            if should_run {
                host.start()
                    .map_err(|error| CenterError::Unavailable(error.to_string()))?;
            }
            service.host = Some(host);
        }
        service.desired_running = should_run;
        service.status = if should_run {
            ServiceStatusDto::Running
        } else {
            ServiceStatusDto::Stopped
        };
        service.message = None;
        service.candidate_view = None;
        service.active_definition = Some(active_definition);
        self.persist_service(name)?;
        self.write_status_log(name);
        let view = self.services[name].view();
        self.push_event(CenterEventKindDto::StatusChanged, Some(view.clone()));
        Ok(view)
    }

    /// 停止服务并保留注册和最后有效配置。
    fn stop_service(&mut self, name: &str) -> Result<ServiceViewDto, CenterError> {
        let service = self.services.get_mut(name).expect("名称已经解析");
        let stop_error = service
            .host
            .as_mut()
            .and_then(|host| host.stop().err())
            .map(|error| error.to_string());
        service.status = if stop_error.is_some() {
            ServiceStatusDto::Failed
        } else {
            ServiceStatusDto::Stopped
        };
        service.message = stop_error;
        service.desired_running = false;
        self.persist_service(name)?;
        self.write_status_log(name);
        let view = self.services[name].view();
        self.push_event(CenterEventKindDto::StatusChanged, Some(view.clone()));
        Ok(view)
    }
}
