use crate::protocol::{ServiceSelectorDto, UploadTargetDto, UploadTargetViewDto};

use super::{Center, CenterError};

impl Center {
    /// 按完整选择器列出当前已生效定义中的上传目标。
    pub(super) fn list_upload_targets(&self) -> Vec<UploadTargetViewDto> {
        self.services
            .iter()
            .flat_map(|(service_name, service)| {
                service
                    .active_definition
                    .iter()
                    .flat_map(move |definition| {
                        definition.upload_targets.iter().map(move |(key, target)| {
                            UploadTargetViewDto {
                                selector: format!("{service_name}::{key}"),
                                kind: target.kind,
                                max_bytes: target.max_bytes,
                            }
                        })
                    })
            })
            .collect()
    }

    /// 从当前已生效定义解析上传目标，磁盘候选不会提前获得写入能力。
    pub(super) fn resolve_upload_target(
        &self,
        selector: &ServiceSelectorDto,
        target: &str,
    ) -> Result<UploadTargetDto, CenterError> {
        let name = self.resolve_name(selector)?;
        let service = &self.services[&name];
        let definition = service
            .active_definition
            .as_ref()
            .ok_or_else(|| CenterError::Unavailable(name.clone()))?;
        let upload = definition.upload_targets.get(target).ok_or_else(|| {
            CenterError::UploadTargetNotFound {
                service: name.clone(),
                target: target.to_owned(),
            }
        })?;
        Ok(UploadTargetDto {
            root: service.root.clone(),
            path: service.root.join(&upload.path),
            kind: upload.kind,
            max_bytes: upload.max_bytes,
        })
    }
}
