//! 中心服务器注册表删除操作。

use crate::protocol::{
    CenterEventKindDto, CenterHello, CenterResponse, ClientHello, PROTOCOL_VERSION,
    ServiceSelectorDto, ServiceStatusDto, ServiceViewDto,
};

use super::{Center, CenterError};

impl Center {
    /// 校验协议版本并返回中心实例身份及 Procora 版本。
    pub(super) fn hello(&self, hello: &ClientHello) -> CenterResponse {
        if hello.protocol_version != PROTOCOL_VERSION {
            return CenterResponse::Error {
                message: format!(
                    "协议版本不兼容：客户端 {}，中心服务器 {}",
                    hello.protocol_version, PROTOCOL_VERSION
                ),
            };
        }
        CenterResponse::Hello(CenterHello {
            protocol_version: PROTOCOL_VERSION,
            procora_version: env!("CARGO_PKG_VERSION").to_owned(),
            instance_id: self.instance_id,
            service_count: self.services.len(),
            event_sequence: self.event_sequence,
            control_allowed: true,
        })
    }

    /// 停止服务宿主并删除内存、当前状态和状态历史注册记录。
    pub(super) fn remove(
        &mut self,
        selector: &ServiceSelectorDto,
    ) -> Result<ServiceViewDto, CenterError> {
        let name = self.resolve_name(selector)?;
        let service = self.services.get_mut(&name).expect("名称已经解析");
        if let Some(host) = service.host.as_mut() {
            host.stop()
                .map_err(|error| CenterError::Unavailable(error.to_string()))?;
        }
        service.status = ServiceStatusDto::Stopped;
        service.desired_running = false;
        let view = service.view();
        self.repository.remove_service(&name)?;
        self.services.remove(&name);
        self.monitors.remove(&name);
        self.push_event(CenterEventKindDto::Removed, Some(view.clone()));
        Ok(view)
    }
}
