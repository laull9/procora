//! 总览向导创建、注册并打开服务的编排。

use crate::{
    cli::api,
    daemon::CenterClient,
    protocol::{CenterRequest, CenterResponse, ServiceSelectorDto},
};

/// 创建或打开向导选中的服务，并直接进入内嵌配置管理。
pub(super) fn create(
    client: &CenterClient,
    choice: crate::tui::NewServiceChoice,
) -> anyhow::Result<()> {
    if let Some(name) = choice.new_service_name {
        api::initialize_managed_config(&choice.directory, &name)?;
    }

    let service = match client.request(&CenterRequest::Open {
        path: choice.directory,
    })? {
        CenterResponse::Service(service) => service,
        CenterResponse::Error { message } => anyhow::bail!(message),
        response => anyhow::bail!("意外服务注册响应: {response:?}"),
    };
    let selector = ServiceSelectorDto::Name(service.name.clone());
    let snapshot = super::request_snapshot(client, &selector)?;
    let hello = client.hello("procora-tui-new-service")?;
    super::run_center_tui_mode(
        client.clone(),
        selector,
        snapshot,
        hello.event_sequence,
        hello.control_allowed,
        true,
        true,
    )
}
