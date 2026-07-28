use std::{
    env,
    io::{self, IsTerminal, Read, Write},
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, bail};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use zeroize::Zeroizing;

const ASKPASS_ENDPOINT: &str = "PROCORA_SSH_ASKPASS_ENDPOINT";
const ASKPASS_REQUEST: u8 = 1;
const ASKPASS_STOP: u8 = 0;
const MAX_PASSWORD_BYTES: usize = 64 * 1024;

/// 一次命令内复用的 SSH 认证方式。
pub(super) enum SshAuth {
    Automatic,
    Password(PasswordCache),
}

impl SshAuth {
    /// 创建不会询问任何凭据的自动认证。
    pub(super) const fn automatic() -> Self {
        Self::Automatic
    }

    /// 从终端隐藏读取一次密码并启动仅驻留内存的本地凭据通道。
    pub(super) fn prompt_password(ssh_target: &str) -> anyhow::Result<Self> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            bail!("SSH 密码登录需要交互终端；请为 `{ssh_target}` 配置 SSH 密钥，或在终端中重试");
        }
        eprintln!("SSH 地址：{ssh_target}");
        let password = rpassword::prompt_password("SSH 密码（仅本次命令，绝不落盘）：")
            .context("无法读取 SSH 密码")?;
        if password.is_empty() {
            bail!("SSH 密码不能为空");
        }
        Ok(Self::Password(PasswordCache::start(password)?))
    }

    /// 把当前认证方式应用到 OpenSSH 命令。
    fn configure(&self, command: &mut Command) -> anyhow::Result<()> {
        match self {
            Self::Automatic => {
                command.args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes"]);
                crate::process::configure_background_command(command);
            }
            Self::Password(cache) => {
                let executable =
                    crate::platform::current_exe().context("无法定位 SSH 密码应答程序")?;
                command.args([
                    "-o",
                    "BatchMode=no",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "NumberOfPasswordPrompts=1",
                    "-o",
                    "PubkeyAuthentication=no",
                    "-o",
                    "PreferredAuthentications=keyboard-interactive,password",
                ]);
                command
                    .env("SSH_ASKPASS", executable)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env("DISPLAY", "procora-ssh-askpass")
                    .env(ASKPASS_ENDPOINT, &cache.endpoint);
                crate::process::configure_background_command(command);
            }
        }
        Ok(())
    }
}

/// 构造共享安全参数并应用指定认证方式的 OpenSSH 命令。
pub(super) fn base_ssh(auth: &SshAuth) -> anyhow::Result<Command> {
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "LogLevel=ERROR",
    ]);
    auth.configure(&mut command)?;
    Ok(command)
}

/// 在未知主机时交给 OpenSSH 展示指纹并由用户显式确认。
pub(super) fn confirm_host_key(ssh_target: &str) -> anyhow::Result<()> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("SSH 主机身份尚未确认；请先在交互终端运行 `ssh {ssh_target}` 核对并接受主机指纹");
    }
    eprintln!("首次连接 `{ssh_target}`，请核对 OpenSSH 显示的主机指纹。");
    let mut command = Command::new("ssh");
    command
        .args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=ask",
            "-o",
            "PreferredAuthentications=none",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            "LogLevel=ERROR",
        ])
        .arg(ssh_target)
        .arg("exit")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let _ = command
        .status()
        .context("无法启动本机 ssh；请先安装 OpenSSH 客户端")?;
    Ok(())
}

/// 若当前进程由 OpenSSH 作为 askpass 助手启动，则输出内存中的密码。
pub(crate) fn answer_askpass_if_requested() -> anyhow::Result<bool> {
    let Some(endpoint) = env::var_os(ASKPASS_ENDPOINT) else {
        return Ok(false);
    };
    let endpoint = endpoint
        .into_string()
        .map_err(|_| anyhow::anyhow!("SSH 密码通道名称不是 UTF-8"))?;
    let password = request_password(&endpoint)?;
    io::stdout().write_all(&password)?;
    io::stdout().write_all(b"\n")?;
    io::stdout().flush()?;
    Ok(true)
}

/// 从仅当前用户可访问的本地端点读取一次密码副本。
fn request_password(endpoint: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let name = endpoint.to_owned().to_ns_name::<GenericNamespaced>()?;
    let mut connection = Stream::connect(name).context("无法连接 SSH 密码通道")?;
    connection.write_all(&[ASKPASS_REQUEST])?;
    connection.flush()?;
    let mut password = Zeroizing::new(Vec::new());
    connection
        .take(MAX_PASSWORD_BYTES as u64 + 1)
        .read_to_end(&mut password)?;
    if password.len() > MAX_PASSWORD_BYTES {
        bail!("SSH 密码响应超过安全上限");
    }
    Ok(password)
}

/// 为多个短生命周期 OpenSSH 进程提供同一份内存密码。
pub(super) struct PasswordCache {
    endpoint: String,
    worker: Option<thread::JoinHandle<()>>,
}

impl PasswordCache {
    /// 创建随机本地端点并把密码所有权交给清理线程。
    fn start(password: String) -> anyhow::Result<Self> {
        let endpoint = format!("procora-ssh-askpass-{}", uuid::Uuid::new_v4());
        let name = endpoint.clone().to_ns_name::<GenericNamespaced>()?;
        let options = ListenerOptions::new().name(name).try_overwrite(false);
        #[cfg(windows)]
        let options = restrict_windows_pipe(options)?;
        let listener = options.create_sync().context("无法创建 SSH 密码通道")?;
        let worker = thread::spawn(move || {
            let password = Zeroizing::new(password);
            while let Ok(mut connection) = listener.accept() {
                if !authorize_peer(&connection) {
                    continue;
                }
                let mut request = [0_u8; 1];
                if connection.read_exact(&mut request).is_err() {
                    continue;
                }
                match request[0] {
                    ASKPASS_REQUEST => {
                        let _ = connection.write_all(password.as_bytes());
                        let _ = connection.flush();
                    }
                    ASKPASS_STOP => break,
                    _ => {}
                }
            }
        });
        Ok(Self {
            endpoint,
            worker: Some(worker),
        })
    }
}

impl Drop for PasswordCache {
    /// 唤醒服务线程并等待密码在内存中清零。
    fn drop(&mut self) {
        if let Ok(name) = self.endpoint.clone().to_ns_name::<GenericNamespaced>()
            && let Ok(mut connection) = Stream::connect(name)
        {
            let _ = connection.write_all(&[ASKPASS_STOP]);
            let _ = connection.flush();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// 校验本地密码请求来自当前有效用户。
#[cfg(unix)]
fn authorize_peer(connection: &Stream) -> bool {
    connection
        .peer_creds()
        .ok()
        .and_then(|credentials| credentials.euid())
        .is_some_and(|uid| uid == rustix::process::geteuid().as_raw())
}

/// Windows 命名管道已通过当前用户 DACL 限制访问。
#[cfg(windows)]
const fn authorize_peer(_connection: &Stream) -> bool {
    true
}

/// 为 Windows 密码管道限制仅所有者、系统和管理员访问。
#[cfg(windows)]
fn restrict_windows_pipe(options: ListenerOptions<'_>) -> anyhow::Result<ListenerOptions<'_>> {
    use interprocess::os::windows::{
        local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
    };
    use widestring::U16CString;

    const CURRENT_USER_DACL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";
    let sddl = U16CString::from_str(CURRENT_USER_DACL)?;
    let descriptor = SecurityDescriptor::deserialize(&sddl)?;
    Ok(options.security_descriptor(descriptor))
}

#[cfg(test)]
mod tests {
    use super::{PasswordCache, request_password};

    // 同一命令内多个OpenSSH进程读取同一份内存密码且无需文件中转。
    #[test]
    fn password_cache_answers_repeated_requests() {
        let cache = PasswordCache::start("内存密码".to_owned()).unwrap();
        assert_eq!(
            &*request_password(&cache.endpoint).unwrap(),
            "内存密码".as_bytes()
        );
        assert_eq!(
            &*request_password(&cache.endpoint).unwrap(),
            "内存密码".as_bytes()
        );
    }
}
