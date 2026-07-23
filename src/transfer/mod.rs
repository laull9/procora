//! 通过 OpenSSH 把本机文件安全提交到远端声明式上传目标。

mod archive;
mod protocol;
mod receive;
mod remote;
mod target;

/// 运行只供 SSH 子进程调用的远端接收器。
pub(crate) use receive::run as receive;
/// 从本机向远端声明式目标上传文件或目录。
pub(crate) use remote::push;

/// 输出不会访问 Center 的 SSH 能力握手。
pub(crate) fn probe() {
    println!("PROCORA_SSH_V1");
}
