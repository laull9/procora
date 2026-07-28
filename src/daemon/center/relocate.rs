//! 全托管 release 在同一服务身份下的事务式目录迁移。

use std::path::Path;

use crate::protocol::{CenterEventKindDto, ServiceSelectorDto, ServiceStatusDto, ServiceViewDto};

use super::{ActiveDefinition, Center, CenterError, ManagedService, ServiceHost};

impl Center {
    /// 在旧根精确匹配时准备、停止并切换 Service，失败时恢复旧宿主。
    pub(super) fn relocate_service(
        &mut self,
        selector: &ServiceSelectorDto,
        expected_root: &Path,
        path: &Path,
    ) -> Result<ServiceViewDto, CenterError> {
        let name = self.resolve_name(selector)?;
        let expected_root = crate::platform::canonicalize(expected_root).map_err(|source| {
            CenterError::InvalidSelectorPath {
                path: expected_root.to_path_buf(),
                source,
            }
        })?;
        let actual_root = self.services[&name].root.clone();
        if actual_root != expected_root {
            return Err(CenterError::RelocationRootMismatch {
                name,
                expected: expected_root,
                actual: actual_root,
            });
        }

        let mut discovered = crate::config::discover_path(path)?;
        if discovered.compiled.spec.project != name {
            return Err(CenterError::InvalidCandidate {
                name,
                message: format!(
                    "新 release 配置声明了不同 Service `{}`",
                    discovered.compiled.spec.project
                ),
            });
        }
        if let Some(existing) = self
            .services
            .values()
            .find(|service| service.name != name && service.root == discovered.root)
        {
            return Err(CenterError::DuplicateRoot {
                root: discovered.root,
                existing: existing.name.clone(),
                requested: name,
            });
        }
        let active_definition = ActiveDefinition::from_compiled(&discovered.compiled);
        super::super::project::prepare(&mut discovered)?;
        let new_root = discovered.root;
        let new_config_path = discovered.config_path;
        let mut new_host = ServiceHost::from_compiled_at(discovered.compiled, &new_root);

        let mut previous = self.services.remove(&name).expect("名称已经解析");
        if let Some(host) = previous.host.as_mut()
            && let Err(error) = host.stop()
        {
            self.services.insert(name, previous);
            return Err(CenterError::Unavailable(format!(
                "切换 release 前停止旧宿主失败：{error}"
            )));
        }
        if let Err(error) = new_host.start() {
            return Err(self.restore_after_relocation_failure(
                &name,
                previous,
                format!("新 release 启动失败：{error}"),
            ));
        }

        let mut replacement = ManagedService {
            name: name.clone(),
            root: new_root,
            config_path: new_config_path,
            status: ServiceStatusDto::Running,
            host: Some(new_host),
            message: None,
            desired_running: true,
            pending_config: None,
            candidate_view: None,
            active_definition: Some(active_definition),
        };
        if let Err(error) = self.repository.save_service(&replacement.stored()) {
            if let Some(host) = replacement.host.as_mut() {
                let _ = host.stop();
            }
            return Err(self.restore_after_relocation_failure(
                &name,
                previous,
                format!("新 release 状态持久化失败：{error}"),
            ));
        }
        self.services.insert(name.clone(), replacement);
        self.install_monitor(&name);
        self.write_status_log(&name);
        let view = self.services[&name].view();
        self.push_event(CenterEventKindDto::StatusChanged, Some(view.clone()));
        Ok(view)
    }

    /// 在切换失败后重新启动并放回旧宿主。
    fn restore_after_relocation_failure(
        &mut self,
        name: &str,
        mut previous: ManagedService,
        failure: String,
    ) -> CenterError {
        let restore = if previous.desired_running {
            previous.host.as_mut().map_or(Ok(()), ServiceHost::start)
        } else {
            Ok(())
        };
        if let Err(error) = restore {
            let combined = format!("{failure}；旧宿主恢复失败：{error}");
            previous.status = ServiceStatusDto::Failed;
            previous.message = Some(combined.clone());
            self.services.insert(name.to_owned(), previous);
            let _ = self.persist_service(name);
            self.write_status_log(name);
            self.install_monitor(name);
            return CenterError::Unavailable(combined);
        }
        previous.status = if previous.desired_running {
            ServiceStatusDto::Running
        } else {
            ServiceStatusDto::Stopped
        };
        previous.message = None;
        self.services.insert(name.to_owned(), previous);
        self.install_monitor(name);
        CenterError::Unavailable(failure)
    }
}
