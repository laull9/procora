//! Linux 上与 systemd 的可选交互。

use sd_notify::NotifyState;
use zbus_systemd::systemd1::ManagerProxy;

/// 向 systemd 报告 Procora daemon 已经就绪。
///
/// # Errors
///
/// 当通知套接字不可用或消息发送失败时返回 I/O 错误。
pub fn notify_ready() -> std::io::Result<()> {
    sd_notify::notify(&[NotifyState::Ready])
}

/// 通过 systemd D-Bus 返回当前已加载单元的名称。
///
/// # Errors
///
/// 当系统总线连接、代理创建或 unit 查询失败时返回 D-Bus 错误。
pub async fn list_unit_names() -> zbus_systemd::zbus::Result<Vec<String>> {
    let connection = zbus_systemd::zbus::Connection::system().await?;
    let manager = ManagerProxy::new(&connection).await?;
    manager
        .list_units()
        .await
        .map(|units| units.into_iter().map(|(name, ..)| name).collect::<Vec<_>>())
}
