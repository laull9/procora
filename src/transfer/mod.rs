//! 通过 OpenSSH 把本机文件安全提交到远端声明式上传目标。

mod archive;
mod deploy_health;
mod deploy_package;
mod deploy_package_management;
mod deploy_platform;
mod deploy_prepare;
mod deploy_protocol;
mod deploy_receive;
mod deploy_remote;
mod deploy_remote_package;
mod deploy_report;
mod deploy_state;
mod deploy_wire;
mod protocol;
mod receive;
mod remote;
mod remote_auth;
mod remote_binary;
mod remote_error;
mod remote_exec;
mod remote_list;
mod remote_selection;
mod target;

/// 把本机 Procora 包安装到不可变 release 并执行验活回滚。
pub(crate) use deploy_package::install as install_package;
pub(crate) use deploy_package_management::{
    recover_installed as recover_installed_package,
    rollback_installed as rollback_installed_package,
    uninstall_installed as uninstall_installed_package,
};
/// 运行只供 SSH 子进程调用的全托管部署接收器。
pub(crate) use deploy_receive::run as receive_deploy;
/// 从本机向远端全托管部署完整 Service。
pub(crate) use deploy_remote::{deploy, preview_deploy};
pub(crate) use deploy_remote_package::{
    deploy as deploy_package, preview as preview_package_deploy,
};
pub(crate) use deploy_report::{DeployEvent, DeployOutcome, DeployPreview};
/// 运行只供 SSH 子进程调用的远端接收器。
pub(crate) use receive::run as receive;
/// 从本机向远端声明式目标上传文件或目录。
pub(crate) use remote::{human_bytes, push, resolve_ssh_target};
pub(crate) use remote_auth::answer_askpass_if_requested;
pub(crate) use remote_exec::run_remote_cli;
pub(crate) use remote_list::list_remote;

/// 返回本机 Center 当前活动上传目标。
pub(crate) fn list_local() -> anyhow::Result<Vec<crate::protocol::UploadTargetViewDto>> {
    target::list()
}

/// 输出供 SSH 客户端读取的本机上传目标 JSON。
pub(crate) fn print_local_targets_json() -> anyhow::Result<()> {
    serde_json::to_writer(std::io::stdout(), &target::list()?)?;
    println!();
    Ok(())
}

/// 输出不会访问 Center 的 SSH 能力握手。
pub(crate) fn probe() {
    println!(
        "{}",
        serde_json::json!({
            "name": "procora-ssh",
            "platform": crate::config::DeployPlatform::current(),
            "transfer_protocol": {
                "min": protocol::TRANSFER_PROTOCOL_MIN_VERSION,
                "max": protocol::TRANSFER_PROTOCOL_VERSION,
            },
            "capabilities": [
                "configured_restart",
                "managed_deploy",
                "platform_binary_selection",
                "requested_restart",
                "target_metadata",
            ],
            "deploy_protocol": {
                "min": deploy_protocol::DEPLOY_PROTOCOL_VERSION,
                "max": deploy_protocol::DEPLOY_PROTOCOL_VERSION,
            },
        })
    );
}
